//! `crypto::sign(type, privateKey, message)` — a clean-room `AbiFunction` signer.
//!
//! Selected by a [`crypto::Certificate`] enum (`P256`/`P384`/`P521`/`Ed25519`),
//! this member's `Body::abi_function` body branches on the enum ordinal and, for
//! the three NIST-EC curves, signs the message **from scratch** here — it does not
//! call into `crypto/native/*` (the migrated per-curve `p{256,384,521}Sign`). It is
//! *modeled on* those (macOS `SecKey`, Linux `EVP_PKEY`, Windows CNG) but reproduces
//! the platform sequence in this file, self-contained, driving the general
//! marshallers for the `List OF Byte` inputs/output. `Ed25519` dispatches to the
//! software [`super::helper_ed25519_sign`] helper (`__crypto_ed25519Sign`).
//!
//! Structure mirrors [`super::func_generate`]: the `Certificate` ordinal consts, a
//! per-platform seam with self-contained clean-room helpers, a `data_objects(family)`
//! driver-gate, and the `Ed25519` branch that calls the always-emitted MFB helper.
//! The one deviation: because each NIST curve differs in its algorithm/DER details,
//! the ordinal branch dispatches to a full per-curve *compile-time* sequence (each a
//! faithful reproduction of the matching `p*Sign` reference), rather than staging
//! per-curve params into one runtime-parametric sequence.
//!
//! Output encoding is identical to the per-curve signers: ECDSA is ASN.1 DER
//! `Ecdsa-Sig-Value`; Ed25519 is the 64-byte raw `R‖S`.

use super::gen_cert::{self, CopyLen, Sc};
use super::{bytes, Body, Implementation, Parameter, ParameterType, RegistryFunction};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::emit_fail;
use crate::codegen::memory::arena::emit_data_address;
use crate::codegen::memory::marshal::{
    emit_build_byte_list, emit_read_byte_list, emit_zero_guarded,
};
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

// The `Certificate` ordinals, framework/library paths, dlsym-name and PKCS#8-template
// data objects, and the shared platform seam (dlopen/dlsym/CoreFoundation/BCrypt) live
// in [`super::gen_cert`]. This file keeps only the `crypto::sign`-specific per-curve
// selector and the three platform signing sequences.

/// The NIST curves this signer covers. A compile-time selector: the ordinal branch
/// in `lower_sign` picks the curve, then emits its full sequence.
#[derive(Clone, Copy)]
enum SignCurve {
    P256,
    P384,
    P521,
}

impl SignCurve {
    fn field_len(self) -> usize {
        match self {
            SignCurve::P256 => 32,
            SignCurve::P384 => 48,
            SignCurve::P521 => 66,
        }
    }
    fn point_len(self) -> usize {
        match self {
            SignCurve::P256 => 65,
            SignCurve::P384 => 97,
            SignCurve::P521 => 133,
        }
    }
    /// The macOS `SecKeyAlgorithm` constant (a CFString) for ECDSA over a message.
    fn macos_algorithm(self) -> &'static str {
        match self {
            SignCurve::P256 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA256",
            SignCurve::P384 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA384",
            SignCurve::P521 => "kSecKeyAlgorithmECDSASignatureMessageX962SHA512",
        }
    }
    /// OpenSSL PKCS#8 template + splice geometry.
    fn pkcs8_len(self) -> usize {
        match self {
            SignCurve::P256 => 138,
            SignCurve::P384 => 185,
            SignCurve::P521 => 241,
        }
    }
    fn p8_scalar_off(self) -> usize {
        match self {
            SignCurve::P256 => 36,
            SignCurve::P384 => 35,
            SignCurve::P521 => 35,
        }
    }
    fn p8_point_off(self) -> usize {
        match self {
            SignCurve::P256 => 73,
            SignCurve::P384 => 88,
            SignCurve::P521 => 108,
        }
    }
    fn name(self) -> &'static str {
        match self {
            SignCurve::P256 => "P-256",
            SignCurve::P384 => "P-384",
            SignCurve::P521 => "P-521",
        }
    }
    fn digest(self) -> &'static str {
        match self {
            SignCurve::P256 => "EVP_sha256",
            SignCurve::P384 => "EVP_sha384",
            SignCurve::P521 => "EVP_sha512",
        }
    }
    /// Windows CNG identifiers + magic.
    fn algo_id(self) -> &'static str {
        match self {
            SignCurve::P256 => "ECDSA_P256",
            SignCurve::P384 => "ECDSA_P384",
            SignCurve::P521 => "ECDSA_P521",
        }
    }
    fn hash_id(self) -> &'static str {
        match self {
            SignCurve::P256 => "SHA256",
            SignCurve::P384 => "SHA384",
            SignCurve::P521 => "SHA512",
        }
    }
    fn hash_len(self) -> usize {
        match self {
            SignCurve::P256 => 32,
            SignCurve::P384 => 48,
            SignCurve::P521 => 64,
        }
    }
    fn priv_magic(self) -> &'static str {
        match self {
            SignCurve::P256 => "844317509", // 0x32534345 'ECS2'
            SignCurve::P384 => "877871941", // 0x34534345 'ECS4'
            SignCurve::P521 => "911426373", // 0x36534345 'ECS6'
        }
    }
}

/// The body's sp-relative scratch size — the max over the three platform layouts
/// (macOS reaches CONST_SCRATCH=200 → 208; Linux 128 → 144; Windows 136 + 64 = 200).
const LOCAL_SIZE: usize = 208;

/// The `AbiFunction` body for `crypto::sign`. `args[0]` is the `Certificate` ordinal,
/// `args[1]` the private-key collection pointer, `args[2]` the message collection
/// pointer — each in its argument register. Self-managed fallible ABI, so it returns
/// the `void` sentinel (the wrapper adds no epilogue).
pub(crate) fn lower_sign(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let ord = args[0].location.clone();
    let priv_op = args[1].location.clone();
    let msg_op = args[2].location.clone();

    builder.allocate_stack_object("crypto_sign_scratch", LOCAL_SIZE);

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

    match platform.family() {
        PlatformFamily::MacOS => {
            let p384 = format!("{symbol}_mp384");
            let p521 = format!("{symbol}_mp521");
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
            emit_macos_sign(
                SignCurve::P256,
                &symbol,
                "mp256",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p384));
            emit_macos_sign(
                SignCurve::P384,
                &symbol,
                "mp384",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p521));
            emit_macos_sign(
                SignCurve::P521,
                &symbol,
                "mp521",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
        }
        PlatformFamily::Linux => {
            let p384 = format!("{symbol}_lp384");
            let p521 = format!("{symbol}_lp521");
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
            emit_linux_sign(
                SignCurve::P256,
                &symbol,
                "lp256",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p384));
            emit_linux_sign(
                SignCurve::P384,
                &symbol,
                "lp384",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p521));
            emit_linux_sign(
                SignCurve::P521,
                &symbol,
                "lp521",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
        }
        PlatformFamily::Windows => {
            let p384 = format!("{symbol}_wp384");
            let p521 = format!("{symbol}_wp521");
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
            emit_windows_sign(
                SignCurve::P256,
                &symbol,
                "wp256",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p384));
            emit_windows_sign(
                SignCurve::P384,
                &symbol,
                "wp384",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.push(abi::label(&p521));
            emit_windows_sign(
                SignCurve::P521,
                &symbol,
                "wp521",
                priv_op.clone(),
                msg_op.clone(),
                &done,
                imports,
                platform,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
        }
    }

    // Ed25519 dispatch (all platforms): route the (privateKey, message) pair into the
    // callee argument registers and call the software `__crypto_ed25519Sign` MFB
    // helper (always emitted with the crypto package). It leaves the 64-byte raw
    // signature `List OF Byte` in the result registers, so fall through to `done`.
    builder.instructions.push(abi::label(&ed25519));
    let priv_op_ed448 = priv_op.clone();
    let msg_op_ed448 = msg_op.clone();
    builder.instructions.extend([
        abi::move_register(abi::argument_register(0)?, priv_op),
        abi::move_register(abi::argument_register(1)?, msg_op),
    ]);
    let ed_symbol = crate::target::shared::nir::function_symbol("#crypto_ed25519Sign");
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

    // Ed448 dispatch (all platforms): the same (privateKey, message) routing into
    // the software `__crypto_ed448Sign` MFB helper, which leaves the 114-byte raw
    // `R‖S` signature in the result registers.
    builder.instructions.push(abi::label(&ed448));
    builder.instructions.extend([
        abi::move_register(abi::argument_register(0)?, priv_op_ed448),
        abi::move_register(abi::argument_register(1)?, msg_op_ed448),
    ]);
    let ed448_symbol = crate::target::shared::nir::function_symbol("#crypto_ed448Sign");
    if win64 {
        builder.instructions.push(abi::subtract_stack(0x20));
    }
    builder.emit_symbol_call(&ed448_symbol);
    if win64 {
        builder.instructions.push(abi::add_stack(0x20));
    }
    builder.instructions.push(abi::branch(&done));

    // X25519 / X448 keys cannot sign.
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
        type_: ParameterType::list_of(ParameterType::Byte),
        location: Operand::from("void"),
        text: "crypto.sign".to_string(),
    })
}

const INTRO: &str = r#"Sign a message with a private key of the given certificate type."#;
const DESC: &str = r#"`crypto::sign(type, privateKey, message)` produces a digital signature over the
raw bytes of `message` using `privateKey`, for the NIST prime curve, `Ed25519`, or
`Ed448` selected by `type` (a `crypto::Certificate`), and returns it as a
`List OF Byte`. `privateKey` is the exact `privateKey` field of the `crypto::KeyPair`
that `crypto::generate(type)` returned for the same `type` — for the NIST curves
the SEC1 uncompressed point followed by the secret scalar (`0x04‖X‖Y‖d`,
97/145/199 bytes for `P256`/`P384`/`P521`), for `Ed25519` the 32-byte seed, and
for `Ed448` the 57-byte seed. That NIST form is this package's own, not one
OpenSSL or a JWK reads — see `mfb man crypto generate` for how to convert a key
in either direction.

For the NIST curves this is **FIPS 186-4 ECDSA** with the curve's mandated digest
(**SHA-256** for `P256`, **SHA-384** for `P384`, **SHA-512** for `P521`) over the
whole message, and the result is an **ASN.1 DER** `Ecdsa-Sig-Value` (a `SEQUENCE {
INTEGER r, INTEGER s }`, X9.62); its length is variable (roughly 70–72 bytes for
`P256`) because `r`/`s` are minimally encoded. ECDSA is randomized, so signing the
same message twice yields different (both valid) signatures. ECDSA signatures are
also **malleable**: from one valid `(r, s)` a third party can derive another
signature that verifies for the same message and key (e.g. `(r, n − s)`), and the
DER can be re-serialized — this implementation does not enforce a canonical low-S
form. Never treat a signature's bytes (or a hash of them) as a unique identifier,
and never use them for replay protection or deduplication. For `Ed25519` this is
**RFC 8032 PureEdDSA** (deterministic; the message is hashed with SHA-512
internally) and the result is the fixed **64-byte** raw `R‖S`. For `Ed448` it is
**RFC 8032 PureEd448** with the empty context (deterministic; SHAKE256 with the
`dom4` prefix internally) and the result is the fixed **114-byte** raw `R‖S`
(57 + 57). Any of them is verifiable with `crypto::verify(type, …)` for the same
`type`.

**Boundary and errors.** A `privateKey` whose length is not the exact SEC1 size
for the chosen curve, or an `Ed25519` key that is not 32 bytes, or an `Ed448` key
that is not 57 bytes, or an otherwise
malformed key the platform import rejects, raises `ErrInvalidArgument`; `message`
may be any length, including empty. `X25519` and `X448` are key-agreement keys and
cannot sign — they raise `ErrInvalidArgument`. Within a size, though, the curve of
`privateKey` is not checked and cannot be: every distinct signing curve here has a
distinct key size, so a cross-curve mix-up is caught, but any 32 bytes are a valid
`Ed25519` seed, so an X25519 private key handed to `sign(crypto::Certificate.Ed25519, …)`
produces a perfectly valid signature — under a public key that is not the one you
published. Take the `crypto::Certificate` from wherever the key was generated.
A platform-library or system failure raises
`ErrUnknown`, and running out of memory raises `ErrOutOfMemory`. The private key
material is zeroed from the working buffers before return.

**Implementation.** ECDSA signing runs through the host platform key API,
reproduced clean-room in this member (no third-party crypto is bundled): on
**macOS** via Security.framework `SecKeyCreateWithData` + `SecKeyCreateSignature`
with `kSecKeyAlgorithmECDSASignatureMessageX962SHA256/384/512`; on **Linux** via
OpenSSL `libcrypto` `EVP_DigestSignInit` + one-shot `EVP_DigestSign`
(`EVP_sha256/384/512`), the SEC1 key spliced into a PKCS#8 template and decoded
with `d2i_AutoPrivateKey`; on **Windows** via CNG `bcrypt.dll`
`BCryptImportKeyPair` (`ECCPRIVATEBLOB`) + `BCryptHash` + `BCryptSignHash`, whose
fixed `r‖s` output is re-encoded to ASN.1 DER here. `Ed25519` and `Ed448` are pure
in-process MFBASIC software cores (RFC 8032) with **no platform library** — the
Ed448 scalar multiplication is a fixed 448-step ladder with a branch-free swap and
its scalar reduction is fixed-length limb arithmetic, so no control flow depends
on the key — and are byte-identical on every OS. Signatures are wire-compatible
across platforms."#;
const EX: &str = r#"```
IMPORT crypto
IMPORT strings

FUNC main() AS Nothing
  LET kp AS crypto::KeyPair = crypto::generate(crypto::Certificate.P256)
  LET msg AS List OF Byte = strings::toBytes("attack at dawn")
  LET sig AS List OF Byte = crypto::sign(crypto::Certificate.P256, kp.privateKey, msg)
  LET ok AS Boolean = crypto::verify(crypto::Certificate.P256, kp.publicKey, msg, sig)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sign",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "type",
                    desc: "The certificate/key type of the private key.",
                    aliases: &[],
                    ty: ParameterType::named("Certificate"),
                    default: crate::codegen::registry::DefaultValue::None,
                },
                Parameter {
                    name: "privateKey",
                    desc: "The private key bytes (the `privateKey` field of a `crypto::KeyPair`).",
                    aliases: &[],
                    ty: bytes(),
                    default: crate::codegen::registry::DefaultValue::None,
                },
                Parameter {
                    name: "message",
                    desc: "The message bytes to sign.",
                    aliases: &[],
                    ty: bytes(),
                    default: crate::codegen::registry::DefaultValue::None,
                },
            ],
            return_type: bytes(),
            errors: vec!["ErrInvalidArgument", "ErrOutOfMemory", "ErrUnknown"],
            body: Body::abi_function(lower_sign),
        }],
    });
}
