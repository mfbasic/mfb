//! `crypto::verify(type, publicKey, message, signature)` — a clean-room `AbiFunction`
//! signature verifier, the counterpart to [`super::func_sign`].
//!
//! Selected by a [`crypto::Certificate`] enum (`P256`/`P384`/`P521`/`Ed25519`), this
//! member's `Body::abi_function` body branches on the enum ordinal and, for the three
//! NIST-EC curves, verifies the signature **from scratch** here — it does not call into
//! `crypto/native/*` (the `p{256,384,521}Verify` helpers it replaces). It is *modeled
//! on* those (macOS `SecKey`, Linux `EVP_PKEY`, Windows CNG) but reproduces the platform
//! sequence in this file, self-contained, driving the general marshallers for the
//! `List OF Byte` inputs. `Ed25519` dispatches to the software `__crypto_ed25519Verify`
//! helper.
//!
//! Semantics: a valid signature returns Boolean TRUE, an invalid/mismatched one returns
//! Boolean FALSE (NOT a raised error). For the NIST curves a malformed public key
//! (wrong length, or a right-length off-curve key the platform import rejects) raises
//! `ErrInvalidArgument`, matching the per-curve `p*Verify`; for `Ed25519`/`Ed448` a
//! wrong-length key or signature is simply FALSE (the software helpers' strict
//! decode). The body self-manages its result (like [`super::func_sign::lower_sign`]).
//!
//! Structure mirrors [`super::func_sign`]: the `Certificate` ordinal consts and per-
//! platform seam live in [`super::gen_cert`]; this file keeps the `crypto::verify`-
//! specific per-curve selector and the three platform verify sequences.
//!
//! Windows note (bug-446): the old `p*Verify` returned FALSE for a valid signature
//! because the CNG verify read the `BCryptVerifySignature` status from
//! `return_register()`, which on Win64 is the *aligned MFB result bank* (`rcx`, not
//! `rax`) — so it saw the stale first argument (the key handle, non-zero) instead of the
//! NTSTATUS and never matched `== 0`. This clean-room verify reads the status from the
//! C-return register (`c_return(0)` = `rax`) where the ABI actually places it.

use super::gen_cert::{self, CopyLen, Sc};
use super::{bytes, Body, Implementation, Parameter, ParameterType, RegistryFunction};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::emit_fail;
use crate::codegen::memory::arena::emit_data_address;
use crate::codegen::memory::marshal::emit_read_byte_list;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
use std::collections::HashMap;

mod linux;
mod macos;
mod windows;
use linux::*;
use macos::*;
use windows::*;

/// The NIST curves this verifier covers. A compile-time selector: the ordinal branch
/// in `lower_verify` picks the curve, then emits its full sequence.
#[derive(Clone, Copy)]
enum VerifyCurve {
    P256,
    P384,
    P521,
}

impl VerifyCurve {
    fn field_len(self) -> usize {
        match self {
            VerifyCurve::P256 => 32,
            VerifyCurve::P384 => 48,
            VerifyCurve::P521 => 66,
        }
    }
    fn point_len(self) -> usize {
        match self {
            VerifyCurve::P256 => 65,
            VerifyCurve::P384 => 97,
            VerifyCurve::P521 => 133,
        }
    }
    /// The macOS `SecKeyAlgorithm` constant (a CFString) for ECDSA over a message.
    fn macos_algorithm(self) -> &'static str {
        match self {
            VerifyCurve::P256 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA256",
            VerifyCurve::P384 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA384",
            VerifyCurve::P521 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA512",
        }
    }
    fn name(self) -> &'static str {
        match self {
            VerifyCurve::P256 => "P-256",
            VerifyCurve::P384 => "P-384",
            VerifyCurve::P521 => "P-521",
        }
    }
    fn digest(self) -> &'static str {
        match self {
            VerifyCurve::P256 => "EVP_sha256",
            VerifyCurve::P384 => "EVP_sha384",
            VerifyCurve::P521 => "EVP_sha512",
        }
    }
    fn algo_id(self) -> &'static str {
        match self {
            VerifyCurve::P256 => "ECDSA_P256",
            VerifyCurve::P384 => "ECDSA_P384",
            VerifyCurve::P521 => "ECDSA_P521",
        }
    }
    fn hash_id(self) -> &'static str {
        match self {
            VerifyCurve::P256 => "SHA256",
            VerifyCurve::P384 => "SHA384",
            VerifyCurve::P521 => "SHA512",
        }
    }
    fn hash_len(self) -> usize {
        match self {
            VerifyCurve::P256 => 32,
            VerifyCurve::P384 => 48,
            VerifyCurve::P521 => 64,
        }
    }
    fn pub_magic(self) -> &'static str {
        match self {
            VerifyCurve::P256 => "827540293", // 0x31534345 'ECS1'
            VerifyCurve::P384 => "861094725", // 0x33534345 'ECS3'
            VerifyCurve::P521 => "894649157", // 0x35534345 'ECS5'
        }
    }
}

/// The body's sp-relative scratch size — the max over the three platform layouts
/// (macOS reaches CONST_SCRATCH=208 → 216; Linux 136 → 144; Windows 112 + 64 = 176).
const LOCAL_SIZE: usize = 216;

/// The `AbiFunction` body for `crypto::verify`. `args[0]` is the `Certificate` ordinal,
/// `args[1]` the public-key collection pointer, `args[2]` the message collection
/// pointer, `args[3]` the signature collection pointer — each in its argument register.
/// Self-managed fallible ABI, so it returns the `void` sentinel.
pub(crate) fn lower_verify(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let ord = args[0].location.clone();
    let pub_op = args[1].location.clone();
    let msg_op = args[2].location.clone();
    let sig_op = args[3].location.clone();

    builder.allocate_stack_object("crypto_verify_scratch", LOCAL_SIZE);

    let done = format!("{symbol}_done");
    let ed25519 = format!("{symbol}_ed25519");
    let ed448 = format!("{symbol}_ed448");
    let x25519_reject = format!("{symbol}_x25519_reject");
    let imports = ctx.platform_imports;
    let platform = ctx.platform;

    // X25519 and X448 are key-agreement (ECDH) keys, not signing keys: reject them
    // up front rather than fall through to the P-256 sequence.
    builder.instructions.extend([
        abi::compare_immediate(&ord, gen_cert::ORD_X25519),
        abi::branch_eq(&x25519_reject),
        abi::compare_immediate(&ord, gen_cert::ORD_X448),
        abi::branch_eq(&x25519_reject),
    ]);

    macro_rules! emit_curves {
        ($emit:ident, $p384:literal, $p521:literal, $t256:literal, $t384:literal, $t521:literal) => {{
            let p384 = format!("{symbol}_{}", $p384);
            let p521 = format!("{symbol}_{}", $p521);
            builder.instructions.extend([
                abi::compare_immediate(&ord, gen_cert::ORD_P384),
                abi::branch_eq(&p384),
                abi::compare_immediate(&ord, gen_cert::ORD_P521),
                abi::branch_eq(&p521),
                abi::compare_immediate(&ord, gen_cert::ORD_ED25519),
                abi::branch_eq(&ed25519),
                abi::compare_immediate(&ord, gen_cert::ORD_ED448),
                abi::branch_eq(&ed448),
            ]);
            // P-256 (ordinal 0) falls through here.
            $emit(
                VerifyCurve::P256,
                &symbol,
                $t256,
                pub_op.clone(),
                msg_op.clone(),
                sig_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p384));
            $emit(
                VerifyCurve::P384,
                &symbol,
                $t384,
                pub_op.clone(),
                msg_op.clone(),
                sig_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p521));
            $emit(
                VerifyCurve::P521,
                &symbol,
                $t521,
                pub_op.clone(),
                msg_op.clone(),
                sig_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
        }};
    }

    match platform.family() {
        PlatformFamily::MacOS => {
            emit_curves!(
                emit_macos_verify,
                "mp384",
                "mp521",
                "mp256",
                "mp384",
                "mp521"
            );
        }
        PlatformFamily::Linux => {
            emit_curves!(
                emit_linux_verify,
                "lp384",
                "lp521",
                "lp256",
                "lp384",
                "lp521"
            );
        }
        PlatformFamily::Windows => {
            emit_curves!(
                emit_windows_verify,
                "wp384",
                "wp521",
                "wp256",
                "wp384",
                "wp521"
            );
        }
    }

    // Ed25519 dispatch (all platforms): route the (publicKey, message, signature) triple
    // into the callee argument registers and call the software `__crypto_ed25519Verify`
    // MFB helper (always emitted with the crypto package). It leaves the Boolean verdict
    // in the result registers, so fall through to `done`.
    builder.instructions.push(abi::label(&ed25519));
    let pub_op_ed448 = pub_op.clone();
    let msg_op_ed448 = msg_op.clone();
    let sig_op_ed448 = sig_op.clone();
    builder.instructions.extend([
        abi::move_register(abi::argument_register(0)?, pub_op),
        abi::move_register(abi::argument_register(1)?, msg_op),
        abi::move_register(abi::argument_register(2)?, sig_op),
    ]);
    let ed_symbol = crate::target::shared::nir::function_symbol("#crypto_ed25519Verify");
    // Win64 mandates 32 bytes of caller-reserved shadow space around every call.
    let win64 = matches!(platform.family(), PlatformFamily::Windows);
    if win64 {
        builder.instructions.push(abi::subtract_stack(0x20));
    }
    builder.emit_symbol_call(&ed_symbol);
    if win64 {
        builder.instructions.push(abi::add_stack(0x20));
    }
    builder.instructions.push(abi::branch(&done));

    // Ed448 dispatch (all platforms): the same triple routing into the software
    // `__crypto_ed448Verify` MFB helper, which leaves the Boolean verdict in the
    // result registers.
    builder.instructions.push(abi::label(&ed448));
    builder.instructions.extend([
        abi::move_register(abi::argument_register(0)?, pub_op_ed448),
        abi::move_register(abi::argument_register(1)?, msg_op_ed448),
        abi::move_register(abi::argument_register(2)?, sig_op_ed448),
    ]);
    let ed448_symbol = crate::target::shared::nir::function_symbol("#crypto_ed448Verify");
    if win64 {
        builder.instructions.push(abi::subtract_stack(0x20));
    }
    builder.emit_symbol_call(&ed448_symbol);
    if win64 {
        builder.instructions.push(abi::add_stack(0x20));
    }
    builder.instructions.push(abi::branch(&done));

    // X25519 / X448 keys cannot verify signatures.
    builder.instructions.push(abi::label(&x25519_reject));
    emit_fail(
        &symbol,
        "ErrInvalidArgument",
        &mut builder.instructions,
        &mut builder.relocations,
        &done,
    );

    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Boolean,
        location: Operand::from("void"),
        text: "crypto.verify".to_string(),
    })
}

const INTRO: &str =
    r#"Verify a signature over a message with a public key of the given certificate type."#;
const DESC: &str = r#"`crypto::verify(type, publicKey, message, signature)` checks whether `signature`
is a valid signature over the raw bytes of `message` under `publicKey`, for the
NIST prime curve, `Ed25519`, or `Ed448` selected by `type` (a `crypto::Certificate`),
and returns a `Boolean`. `publicKey` is the `publicKey` field of the
`crypto::KeyPair` from `crypto::generate(type)` — for the NIST curves one SEC1
uncompressed point `0x04‖X‖Y` (65/97/133 bytes for `P256`/`P384`/`P521`), for
`Ed25519` the 32-byte compressed point, for `Ed448` the 57-byte compressed point —
and `signature` is the output of `crypto::sign(type, …)`. The NIST public form is
the standard SEC1 / X9.62 uncompressed point, so a key exported from OpenSSL or
another ecosystem can be passed straight in; only the *private* form is
package-local, and `mfb man crypto generate` covers converting that one.

It returns `TRUE` **if and only if** `signature` is a valid signature of that
exact `message` under that exact `publicKey`, and `FALSE` otherwise — a failed
verdict (wrong message, wrong key, or a corrupt/wrong-length `signature`) is a
normal outcome, not an error. For the NIST curves this is **FIPS 186-4 ECDSA**
with the curve's digest (SHA-256/384/512 for `P256`/`P384`/`P521`) and the
signature is an **ASN.1 DER** `Ecdsa-Sig-Value`. ECDSA signatures are **malleable**
and are not constrained here to a canonical low-S form, so a single message/key can
have more than one distinct `signature` that verifies `TRUE`; never use signature
bytes as a unique identifier (see `crypto::sign`). For `Ed25519` it is **RFC 8032
PureEdDSA** over the fixed 64-byte `R‖S`, which additionally rejects a
non-canonical scalar `S ≥ L` (returning `FALSE`) so a malleated `Ed25519` signature
cannot verify. For `Ed448` it is **RFC 8032 PureEd448** (empty context, SHAKE256
with `dom4`) over the fixed 114-byte `R‖S`, with strict decoding: a non-canonical
`S ≥ L`, a non-canonical (`y ≥ p`) or off-curve point, a sign byte with any of its
seven unused bits set, and a small-order (4-torsion) public key or `R` all return
`FALSE`.

**Boundary and errors.** For the NIST curves a malformed `publicKey` — wrong
length, or a right-length off-curve point the platform import rejects — raises
`ErrInvalidArgument` (it is a caller mistake, not a false verdict). For `Ed25519`
and `Ed448` a wrong-length key or signature is simply `FALSE`. `X25519` and `X448` cannot
verify and raise `ErrInvalidArgument`. A platform-library or system failure raises
`ErrUnknown`, and running out of memory raises `ErrOutOfMemory`. The untrusted
`signature` bytes are fully bounds-checked before use.

**Implementation.** ECDSA verification runs through the host platform key API,
reproduced clean-room in this member (no third-party crypto is bundled): on
**macOS** via Security.framework `SecKeyCreateWithData` + `SecKeyVerifySignature`
(`kSecKeyAlgorithmECDSASignatureMessageX962SHA256/384/512`); on **Linux** via
OpenSSL `libcrypto` `EVP_DigestVerifyInit` + one-shot `EVP_DigestVerify`
(`EVP_sha256/384/512`), the SEC1 point wrapped in a fixed SPKI DER prefix and
decoded with `d2i_PUBKEY`; on **Windows** via CNG `bcrypt.dll`
`BCryptImportKeyPair` (`ECCPUBLICBLOB`) + `BCryptHash` + `BCryptVerifySignature`,
the DER signature decoded here to the fixed `r‖s`. `Ed25519` and `Ed448` are pure
in-process MFBASIC software cores (RFC 8032) with **no platform library**,
byte-identical on every OS. A signature or key produced on one OS verifies on the
others."#;
const EX: &str = r#"```
IMPORT crypto
IMPORT strings
IMPORT io

FUNC main() AS Nothing
  LET kp AS crypto::KeyPair = crypto::generate(crypto::Certificate.P256)
  LET msg AS List OF Byte = strings::toBytes("attack at dawn")
  LET sig AS List OF Byte = crypto::sign(crypto::Certificate.P256, kp.privateKey, msg)
  LET ok AS Boolean = crypto::verify(crypto::Certificate.P256, kp.publicKey, msg, sig)
  io::print(toString(ok))
END FUNC
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "verify",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "type",
                    desc: "The certificate/key type of the public key.",
                    aliases: &[],
                    ty: ParameterType::named("Certificate"),
                    default: crate::codegen::registry::DefaultValue::None,
                },
                Parameter {
                    name: "publicKey",
                    desc: "The public key bytes (the `publicKey` field of a `crypto::KeyPair`).",
                    aliases: &[],
                    ty: bytes(),
                    default: crate::codegen::registry::DefaultValue::None,
                },
                Parameter {
                    name: "message",
                    desc: "The message bytes that were signed.",
                    aliases: &[],
                    ty: bytes(),
                    default: crate::codegen::registry::DefaultValue::None,
                },
                Parameter {
                    name: "signature",
                    desc: "The signature bytes to verify.",
                    aliases: &[],
                    ty: bytes(),
                    default: crate::codegen::registry::DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec!["ErrInvalidArgument", "ErrOutOfMemory", "ErrUnknown"],
            body: Body::abi_function(lower_verify),
        }],
    });
}
