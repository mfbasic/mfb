//! Native code generation for the `crypto::` NIST-EC public-key helpers
//! (plan-04-crypto.md Part C). The elliptic-curve operations are bound to the
//! platform's modern key API rather than implemented as software cores: generic
//! NIST bignum arithmetic is ~100x costlier than Ed25519's special-prime field
//! and is impractical over the package's `bits::` layer.
//!
//!   * macOS  — `SecKey` (Security.framework) + CoreFoundation, dlopen/dlsym.
//!   * Linux  — `EVP_PKEY` (libcrypto) via dlopen/dlsym (see `crypto_ec_openssl`).
//!
//! The two backends are wire-compatible (the user's hard requirement): a key or
//! signature produced on one platform is accepted by the other. The agreed
//! encodings are
//!
//!   * private key = `0x04 ‖ X ‖ Y ‖ K`  (SEC1 uncompressed point followed by the
//!     big-endian scalar) — self-contained so every backend can reconstruct the
//!     key without deriving the public point;
//!   * public key  = `0x04 ‖ X ‖ Y`      (SEC1 uncompressed point);
//!   * signature   = ASN.1 DER `Ecdsa-Sig-Value` (X9.62).
//!
//! Field width per curve: P-256 → 32, P-384 → 48, P-521 → 66 bytes.
//!
//! Native helpers only ever return a `List OF Byte` (raw key bytes / DER
//! signature) or a `Boolean` (verify) — never a record. `crypto::generateP*`
//! is source glue that calls the raw-keygen helper and slices the public point
//! out of the private bytes to build the `KeyPair` (see `crypto_package.mfb`).

use std::collections::HashMap;

use super::*;
use crate::target::shared::abi;

/// The elliptic curve a helper targets. Only the input key size (keygen) and the
/// ECDSA message-digest algorithm (sign/verify) vary between curves.
#[derive(Clone, Copy)]
pub(super) enum Curve {
    P256,
    P384,
    P521,
}

impl Curve {
    fn bits(self) -> &'static str {
        match self {
            Curve::P256 => "256",
            Curve::P384 => "384",
            Curve::P521 => "521",
        }
    }
    /// Length in bytes of an uncompressed SEC1 public point (`04 || X || Y`).
    ///
    /// Both backends splice fixed-length keys, so both must reject a public key
    /// of any other length up front. The OpenSSL backend carries this in its
    /// `CurveParams`; it lives here so the macOS backend's parity check (bug-317
    /// T4) cannot drift from it.
    fn point_len(self) -> usize {
        match self {
            Curve::P256 => 65,
            Curve::P384 => 97,
            Curve::P521 => 133,
        }
    }
    /// The macOS `SecKeyAlgorithm` constant (a CFString) for ECDSA over a message.
    fn macos_algorithm(self) -> &'static str {
        match self {
            Curve::P256 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA256",
            Curve::P384 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA384",
            Curve::P521 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA512",
        }
    }
}

/// Map a runtime-helper call name onto (operation, curve).
pub(super) fn ec_call(call: &str) -> Option<(EcOp, Curve)> {
    let (op, curve) = match call {
        "crypto.generateP256Raw" => (EcOp::Generate, Curve::P256),
        "crypto.generateP384Raw" => (EcOp::Generate, Curve::P384),
        "crypto.generateP521Raw" => (EcOp::Generate, Curve::P521),
        "crypto.p256Sign" => (EcOp::Sign, Curve::P256),
        "crypto.p384Sign" => (EcOp::Sign, Curve::P384),
        "crypto.p521Sign" => (EcOp::Sign, Curve::P521),
        "crypto.p256Verify" => (EcOp::Verify, Curve::P256),
        "crypto.p384Verify" => (EcOp::Verify, Curve::P384),
        "crypto.p521Verify" => (EcOp::Verify, Curve::P521),
        _ => return None,
    };
    Some((op, curve))
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum EcOp {
    Generate,
    Sign,
    Verify,
}

/// True for the runtime-helper symbols emitted by this module (used to gate the
/// per-platform read-only data objects).
pub(super) fn is_ec_symbol(symbol: &str) -> bool {
    symbol.starts_with("_mfb_rt_crypto_crypto_generateP")
        || symbol.starts_with("_mfb_rt_crypto_crypto_p256")
        || symbol.starts_with("_mfb_rt_crypto_crypto_p384")
        || symbol.starts_with("_mfb_rt_crypto_crypto_p521")
}

pub(super) fn lower_crypto_ec_helper(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let (op, curve) =
        ec_call(call).ok_or_else(|| format!("crypto EC helper: unknown call {call}"))?;
    if platform.target().contains("macos") {
        macos::lower(op, curve, symbol, platform_imports, platform)
    } else {
        openssl::lower(op, curve, symbol, platform_imports, platform)
    }
}

// ---------------------------------------------------------------------------
// Shared List OF Byte marshalling
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
/// Read a `List OF Byte` (collection pointer already stored at `coll_off`) into a
/// freshly arena-allocated contiguous buffer. Stores the buffer pointer at
/// `buf_off` and the byte count at `len_off`. Uses only vreg scratch (no calls).
/// Branches to `alloc_fail` on allocation failure.
pub(super) fn emit_read_byte_list(
    symbol: &str,
    tag: &str,
    coll_off: usize,
    buf_off: usize,
    len_off: usize,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let copy_loop = format!("{symbol}_{tag}_read_loop");
    let copy_done = format!("{symbol}_{tag}_read_done");
    // count = coll->count; allocate max(count,1) bytes.
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), coll_off),
        abi::load_u64("%v10", "%v9", COLLECTION_OFFSET_COUNT),
        abi::store_u64("%v10", abi::stack_pointer(), len_off),
        abi::add_immediate(abi::return_register(), "%v10", 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), buf_off),
        // dataBase = coll + HEADER + capacity*ENTRY_SIZE
        abi::load_u64("%v9", abi::stack_pointer(), coll_off),
        abi::load_u64("%v11", "%v9", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("%v12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v13", "%v11", "%v12"),
        abi::add_immediate("%v13", "%v13", COLLECTION_HEADER_SIZE),
        abi::add_registers("%v13", "%v9", "%v13"), // %v13 = dataBase
        abi::add_immediate("%v14", "%v9", COLLECTION_HEADER_SIZE), // %v14 = entry cursor
        abi::load_u64("%v10", abi::stack_pointer(), len_off),
        abi::load_u64("%v15", abi::stack_pointer(), buf_off), // out cursor
        abi::move_immediate("%v9", "Integer", "0"),           // i
        abi::label(&copy_loop),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_eq(&copy_done),
        // byte = dataBase[entry->value_offset]
        abi::load_u64("%v16", "%v14", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers("%v16", "%v13", "%v16"),
        abi::load_u8("%v17", "%v16", 0),
        abi::store_u8("%v17", "%v15", 0),
        abi::add_immediate("%v15", "%v15", 1),
        abi::add_immediate("%v14", "%v14", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("%v9", "%v9", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
    ]);
}

#[allow(clippy::too_many_arguments)]
/// Build a `List OF Byte` of `len_off` bytes copied from the contiguous buffer at
/// `src_off`, storing the collection pointer at `coll_off`. Uses only vreg
/// scratch. Branches to `alloc_fail` on allocation failure.
/// Allocate a `List OF Byte` of `len_off` elements, write its header, and fill
/// the lookup table with the identity mapping while copying the payload bytes
/// from `src_off` — entry write and byte copy fused in one pass.
///
/// The single `List OF Byte` constructor for the socket/TLS/EC/entropy
/// backends. It is one of the places that must stop writing a lookup table when
/// plan-57-D gives a fixed-width list no entry array; consolidating the copies
/// here is what keeps that a single edit.
///
/// Two knobs, because that is the whole of the variation between the sites:
///
/// - `block` is the register holding the freshly allocated block. `net/io`
///   moves the allocator result into a vreg first (plan-34-B Phase 3); the TLS
///   and EC paths address `abi::RET[1]` directly. When `block` is not
///   `abi::RET[1]` the move is emitted here.
/// - `coll_off` is `Some` only where the caller wants the block pointer spilled
///   to a frame slot. The TLS paths keep it in the register and never spill.
///
/// `entry_loop`/`entry_done` are the caller's own label names rather than being
/// derived here, for the same reason: the sites had different naming schemes and
/// a rename would show up as a real diff in the generated dump.
///
/// All three knobs exist so every caller reproduces its previous instruction
/// stream exactly. `scripts/artifact-gate.sh` covers all of them across
/// macos-aarch64 / linux-aarch64 / linux-x86_64.
pub(super) fn emit_build_byte_list(
    symbol: &str,
    entry_loop: &str,
    entry_done: &str,
    src_off: usize,
    len_off: usize,
    coll_off: Option<usize>,
    block: &str,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // size = HEADER + count*ENTRY_SIZE + count(data)
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), len_off),
        abi::move_immediate("%v11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v12", "%v10", "%v11"),
        abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), "%v12", "%v10"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    if block != block {
        instructions.push(abi::move_register(block, block));
    }
    if let Some(slot) = coll_off {
        instructions.push(abi::store_u64(block, abi::stack_pointer(), slot));
    }
    instructions.extend([
        abi::move_immediate("%v9", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("%v9", block, COLLECTION_OFFSET_KIND),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("%v9", block, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8("%v9", block, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("%v9", "Byte", "1"),
        abi::store_u8("%v9", block, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("%v10", abi::stack_pointer(), len_off),
        abi::store_u64("%v10", block, COLLECTION_OFFSET_COUNT),
        abi::store_u64("%v10", block, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64("%v10", block, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("%v10", block, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate("%v11", block, COLLECTION_HEADER_SIZE),
        abi::move_immediate("%v12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v13", "%v10", "%v12"),
        abi::add_registers("%v14", "%v11", "%v13"), // data base
        abi::load_u64("%v15", abi::stack_pointer(), src_off),
        abi::move_immediate("%v9", "Integer", "0"),
        abi::label(entry_loop),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_eq(entry_done),
        abi::move_immediate("%v12", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("%v12", "%v11", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64("%v9", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate("%v12", "Integer", "1"),
        abi::store_u64("%v12", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_registers("%v12", "%v14", "%v9"),
        abi::load_u8("%v13", "%v15", 0),
        abi::store_u8("%v13", "%v12", 0),
        abi::add_immediate("%v15", "%v15", 1),
        abi::add_immediate("%v11", "%v11", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("%v9", "%v9", 1),
        abi::branch(entry_loop),
        abi::label(entry_done),
    ]);
}

/// `bl _mfb_arena_alloc` (size in x0, align in x1); block pointer left in x1.

pub(super) fn emit_fail(
    symbol: &str,
    code: &str,
    message_symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    done: &str,
) {
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", code),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, message_symbol, instructions, relocations);
    instructions.push(abi::branch(done));
}

pub(super) mod macos;
pub(super) mod openssl;
