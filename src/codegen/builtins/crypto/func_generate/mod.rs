//! `crypto::generate(type)` — a clean-room `AbiFunction` key-pair generator.
//!
//! Selected by a [`crypto::Certificate`] enum (`P256`/`P384`/`P521`/`Ed25519`/
//! `X25519`), this member's `Body::abi_function` body branches on the enum ordinal.
//! For the three NIST-EC curves it generates the key pair **from scratch** here — it
//! does not call into `crypto/native/*` — reproducing the platform key sequence
//! (macOS `SecKey`, Linux OpenSSL `EVP_PKEY`, Windows CNG) self-contained on each
//! target, driving the general marshallers for the `List OF Byte`/`KeyPair` output.
//! `Ed25519` dispatches to [`super::helper_generate_ed25519`] and `X25519` to
//! [`super::helper_generate_x25519`] — both pure MFBASIC software cores.
//!
//! All three platforms are implemented and per-platform verified (macOS host, Linux
//! aarch64, Windows x86-64).

use super::gen_cert::{self, CopyLen};
use super::{Body, Implementation, Parameter, ParameterType, RegistryFunction};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::emit_fail;
use crate::codegen::memory::arena::emit_data_address;
use crate::codegen::memory::marshal::{
    emit_build_byte_list, emit_build_inlined_record, emit_free_buffer_guarded,
    emit_free_byte_list_guarded, emit_zero_guarded, RecordBuildScratch, ScratchSize,
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

// The `Certificate` ordinals, framework/library paths, dlsym-name data objects, and
// the shared platform seam (dlopen/dlsym/CoreFoundation/BCrypt) live in
// [`super::gen_cert`]. This file keeps only the `crypto::generate`-specific per-curve
// geometry and the three platform key-generation sequences.

// ---------------------------------------------------------------------------
// Linux OpenSSL per-curve geometry (ordinal-indexed): (curve-name, nid, point_len,
// field_len, sec1_scalar_off, spki_prefix_len).
// ---------------------------------------------------------------------------
struct OsslCurve {
    name: &'static str,
    nid: &'static str,
    point_len: usize,
    field_len: usize,
    sec1_scalar_off: usize,
    spki_prefix_len: usize,
}
const OSSL_CURVES: &[OsslCurve] = &[
    OsslCurve {
        name: "P-256",
        nid: "415",
        point_len: 65,
        field_len: 32,
        sec1_scalar_off: 7,
        spki_prefix_len: 26,
    },
    OsslCurve {
        name: "P-384",
        nid: "715",
        point_len: 97,
        field_len: 48,
        sec1_scalar_off: 8,
        spki_prefix_len: 23,
    },
    OsslCurve {
        name: "P-521",
        nid: "716",
        point_len: 133,
        field_len: 66,
        sec1_scalar_off: 8,
        spki_prefix_len: 25,
    },
];

// ---------------------------------------------------------------------------
// Windows CNG per-curve parameters (ordinal-indexed): (algo-id, field_len). The raw
// (`1+3·field`) and public (`1+2·field`) lengths are derived at runtime.
// ---------------------------------------------------------------------------
struct WinCurve {
    algo: &'static str,
    field_len: usize,
    bits: &'static str,
}
const WIN_CURVES: &[WinCurve] = &[
    WinCurve {
        algo: "ECDSA_P256",
        field_len: 32,
        bits: "256",
    },
    WinCurve {
        algo: "ECDSA_P384",
        field_len: 48,
        bits: "384",
    },
    WinCurve {
        algo: "ECDSA_P521",
        field_len: 66,
        bits: "521",
    },
];

const LOCAL_SIZE: usize = 208;

/// The `AbiFunction` body for `crypto::generate`. `args[0]` is the `Certificate`
/// ordinal (in the first argument register). Self-managed fallible ABI, so it
/// returns the `void` sentinel — the wrapper adds no epilogue.
pub(crate) fn lower_generate(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let ord = args[0].location.clone();

    // Reserve the body's sp-relative scratch (base 0) so `finalize_vreg_body` sizes
    // it, and mint the body's scratch vregs. The marshal helpers mint theirs from the
    // same `vregs` (bug-564), so no helper can write one of these.
    builder.allocate_stack_object("crypto_generate_scratch", LOCAL_SIZE);
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();

    let done = format!("{symbol}_done");
    let ed25519 = format!("{symbol}_ed25519");
    let x25519 = format!("{symbol}_x25519");
    let x448 = format!("{symbol}_x448");
    let ed448 = format!("{symbol}_ed448");

    match ctx.platform.family() {
        PlatformFamily::MacOS => {
            // Select the runtime curve params (key size in bits -> NUMVAL, SEC1 point
            // length -> PUBLEN) from the ordinal, then run the one SecKey sequence.
            let p384 = format!("{symbol}_p384");
            let p521 = format!("{symbol}_p521");
            let have = format!("{symbol}_have_params");
            let set = |ins: &mut Vec<CodeInstruction>, v9: Reg, bits: &str, plen: &str| {
                ins.extend([
                    abi::move_immediate(v9, "Integer", bits),
                    abi::store_u64(v9, abi::stack_pointer(), NUMVAL),
                    abi::move_immediate(v9, "Integer", plen),
                    abi::store_u64(v9, abi::stack_pointer(), PUBLEN),
                    abi::branch(&have),
                ]);
            };
            builder.instructions.extend([
                abi::compare_immediate(&ord, gen_cert::ORD_P384),
                abi::branch_eq(&p384),
                abi::compare_immediate(&ord, gen_cert::ORD_P521),
                abi::branch_eq(&p521),
                abi::compare_immediate(&ord, gen_cert::ORD_ED25519),
                abi::branch_eq(&ed25519),
                abi::compare_immediate(&ord, gen_cert::ORD_X25519),
                abi::branch_eq(&x25519),
                abi::compare_immediate(&ord, gen_cert::ORD_X448),
                abi::branch_eq(&x448),
                abi::compare_immediate(&ord, gen_cert::ORD_ED448),
                abi::branch_eq(&ed448),
            ]);
            // P-256 (ordinal 0) falls through here.
            set(&mut builder.instructions, Reg::from(v9), "256", "65");
            builder.instructions.push(abi::label(&p384));
            set(&mut builder.instructions, Reg::from(v9), "384", "97");
            builder.instructions.push(abi::label(&p521));
            set(&mut builder.instructions, Reg::from(v9), "521", "133");
            builder.instructions.push(abi::label(&have));
            emit_macos_ec(builder, ctx, &symbol, &v9, &mut vregs, &done)?;
        }
        PlatformFamily::Linux => {
            // Select the runtime OpenSSL curve params (nid, point/field lengths, DER
            // offsets, curve-name pointer) from the ordinal, then run the sequence.
            let lp384 = format!("{symbol}_lp384");
            let lp521 = format!("{symbol}_lp521");
            let have = format!("{symbol}_lhave");
            let sym_owned = symbol.clone();
            let set = |ins: &mut Vec<CodeInstruction>,
                       rel: &mut Vec<CodeRelocation>,
                       v9: Reg,
                       c: &OsslCurve| {
                ins.extend([
                    abi::move_immediate(v9, "Integer", c.nid),
                    abi::store_u64(v9, abi::stack_pointer(), L_NID),
                    abi::move_immediate(v9, "Integer", &c.point_len.to_string()),
                    abi::store_u64(v9, abi::stack_pointer(), L_POINTLEN),
                    abi::move_immediate(v9, "Integer", &c.field_len.to_string()),
                    abi::store_u64(v9, abi::stack_pointer(), L_FIELDLEN),
                    abi::move_immediate(v9, "Integer", &c.sec1_scalar_off.to_string()),
                    abi::store_u64(v9, abi::stack_pointer(), L_SEC1OFF),
                    abi::move_immediate(v9, "Integer", &c.spki_prefix_len.to_string()),
                    abi::store_u64(v9, abi::stack_pointer(), L_SPKIPREFIX),
                ]);
                emit_data_address(&sym_owned, v9, &gen_cert::ossl_name_sym(c.name), ins, rel);
                ins.extend([
                    abi::store_u64(v9, abi::stack_pointer(), L_NAMEPTR),
                    abi::branch(&have),
                ]);
            };
            builder.instructions.extend([
                abi::compare_immediate(&ord, gen_cert::ORD_P384),
                abi::branch_eq(&lp384),
                abi::compare_immediate(&ord, gen_cert::ORD_P521),
                abi::branch_eq(&lp521),
                abi::compare_immediate(&ord, gen_cert::ORD_ED25519),
                abi::branch_eq(&ed25519),
                abi::compare_immediate(&ord, gen_cert::ORD_X25519),
                abi::branch_eq(&x25519),
                abi::compare_immediate(&ord, gen_cert::ORD_X448),
                abi::branch_eq(&x448),
                abi::compare_immediate(&ord, gen_cert::ORD_ED448),
                abi::branch_eq(&ed448),
            ]);
            // P-256 (ordinal 0) falls through here.
            set(
                &mut builder.instructions,
                &mut builder.relocations,
                Reg::from(v9),
                &OSSL_CURVES[0],
            );
            builder.instructions.push(abi::label(&lp384));
            set(
                &mut builder.instructions,
                &mut builder.relocations,
                Reg::from(v9),
                &OSSL_CURVES[1],
            );
            builder.instructions.push(abi::label(&lp521));
            set(
                &mut builder.instructions,
                &mut builder.relocations,
                Reg::from(v9),
                &OSSL_CURVES[2],
            );
            builder.instructions.push(abi::label(&have));
            emit_linux_ec(
                builder, ctx, &symbol, &v9, &v10, &v11, &v12, &v13, &mut vregs, &done,
            )?;
        }
        PlatformFamily::Windows => {
            // Select the runtime CNG curve params (field length, algorithm-id
            // pointer) from the ordinal, then run the BCrypt sequence.
            let wp384 = format!("{symbol}_wp384");
            let wp521 = format!("{symbol}_wp521");
            let have = format!("{symbol}_whave");
            let sym_owned = symbol.clone();
            let set = |ins: &mut Vec<CodeInstruction>,
                       rel: &mut Vec<CodeRelocation>,
                       v9: Reg,
                       c: &WinCurve| {
                ins.extend([
                    abi::move_immediate(v9, "Integer", &c.field_len.to_string()),
                    abi::store_u64(v9, abi::stack_pointer(), W_FIELD),
                    abi::move_immediate(v9, "Integer", c.bits),
                    abi::store_u64(v9, abi::stack_pointer(), W_BITS),
                ]);
                emit_data_address(&sym_owned, v9, &gen_cert::win_sym(c.algo), ins, rel);
                ins.extend([
                    abi::store_u64(v9, abi::stack_pointer(), W_ALGOPTR),
                    abi::branch(&have),
                ]);
            };
            builder.instructions.extend([
                abi::compare_immediate(&ord, gen_cert::ORD_P384),
                abi::branch_eq(&wp384),
                abi::compare_immediate(&ord, gen_cert::ORD_P521),
                abi::branch_eq(&wp521),
                abi::compare_immediate(&ord, gen_cert::ORD_ED25519),
                abi::branch_eq(&ed25519),
                abi::compare_immediate(&ord, gen_cert::ORD_X25519),
                abi::branch_eq(&x25519),
                abi::compare_immediate(&ord, gen_cert::ORD_X448),
                abi::branch_eq(&x448),
                abi::compare_immediate(&ord, gen_cert::ORD_ED448),
                abi::branch_eq(&ed448),
            ]);
            // P-256 (ordinal 0) falls through here.
            set(
                &mut builder.instructions,
                &mut builder.relocations,
                Reg::from(v9),
                &WIN_CURVES[0],
            );
            builder.instructions.push(abi::label(&wp384));
            set(
                &mut builder.instructions,
                &mut builder.relocations,
                Reg::from(v9),
                &WIN_CURVES[1],
            );
            builder.instructions.push(abi::label(&wp521));
            set(
                &mut builder.instructions,
                &mut builder.relocations,
                Reg::from(v9),
                &WIN_CURVES[2],
            );
            builder.instructions.push(abi::label(&have));
            emit_windows_ec(
                builder, ctx, &symbol, &v9, &v10, &v11, &v12, &v13, &mut vregs, &done,
            )?;
        }
    }

    // Ed25519 dispatch (all platforms): call the software
    // `__crypto_generateEd25519` MFB helper (always emitted with the crypto
    // package — seed = randomBytes(32); pub = ed25519Public(seed)). It leaves the
    // `KeyPair` in the result registers, so fall through to `done`.
    builder.instructions.push(abi::label(&ed25519));
    let ed_symbol = crate::target::shared::nir::function_symbol("#crypto_generateEd25519");
    // Win64 mandates 32 bytes of caller-reserved shadow space around every call.
    let win64 = matches!(ctx.platform.family(), PlatformFamily::Windows);
    if win64 {
        builder.instructions.push(abi::subtract_stack(0x20));
    }
    builder.emit_symbol_call(&ed_symbol);
    if win64 {
        builder.instructions.push(abi::add_stack(0x20));
    }
    builder.instructions.push(abi::branch(&done));

    // X25519 dispatch (all platforms): call the software `__crypto_generateX25519`
    // MFB helper (always emitted with the crypto package — scalar = clamp(randomBytes
    // (32)); pub = X25519(scalar, basepoint u=9)). It leaves the `KeyPair` in the
    // result registers, so fall through to `done`.
    builder.instructions.push(abi::label(&x25519));
    let x_symbol = crate::target::shared::nir::function_symbol("#crypto_generateX25519");
    if win64 {
        builder.instructions.push(abi::subtract_stack(0x20));
    }
    builder.emit_symbol_call(&x_symbol);
    if win64 {
        builder.instructions.push(abi::add_stack(0x20));
    }
    builder.instructions.push(abi::branch(&done));

    // X448 dispatch (all platforms): call the software `__crypto_generateX448` MFB
    // helper (scalar = clamp448(randomBytes(56)); pub = X448(scalar, basepoint u=5)).
    // It leaves the `KeyPair` in the result registers, so fall through to `done`.
    builder.instructions.push(abi::label(&x448));
    let x448_symbol = crate::target::shared::nir::function_symbol("#crypto_generateX448");
    if win64 {
        builder.instructions.push(abi::subtract_stack(0x20));
    }
    builder.emit_symbol_call(&x448_symbol);
    if win64 {
        builder.instructions.push(abi::add_stack(0x20));
    }
    builder.instructions.push(abi::branch(&done));

    // Ed448 dispatch (all platforms): call the software `__crypto_generateEd448`
    // MFB helper (seed = randomBytes(57); pub = ed448Public(seed)). It leaves the
    // `KeyPair` in the result registers, so fall through to `done`.
    builder.instructions.push(abi::label(&ed448));
    let ed448_symbol = crate::target::shared::nir::function_symbol("#crypto_generateEd448");
    if win64 {
        builder.instructions.push(abi::subtract_stack(0x20));
    }
    builder.emit_symbol_call(&ed448_symbol);
    if win64 {
        builder.instructions.push(abi::add_stack(0x20));
    }

    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::named("KeyPair"),
        location: Operand::from("void"),
        text: "crypto.generate".to_string(),
    })
}

const INTRO: &str = r#"Generate a fresh key pair of the requested certificate type."#;
const DESC: &str = r#"`crypto::generate(type)` draws a fresh random key pair for the curve selected by
`type` (a `crypto::Certificate`) and returns it as a `crypto::KeyPair`, whose
`privateKey` and `publicKey` fields are each a `List OF Byte`. The signing key
pairs — the three NIST prime curves (`P256`/`P384`/`P521`, FIPS 186-4 ECDSA over
SEC/NIST `secp256r1`/`secp384r1`/`secp521r1`), `Ed25519`, and `Ed448` (RFC 8032
EdDSA) — are usable with `crypto::sign(type, …)` and `crypto::verify(type, …)` for
that same `type`. `X25519` and `X448` produce Curve25519 / Curve448 ECDH key-agreement
pairs (RFC 7748) for `crypto::exchange`; they are **not** signing keys, so
`sign`/`verify` reject them with `ErrInvalidArgument`. Note that
`crypto::encrypt`/`crypto::decrypt` take **Ed25519** or **Ed448** keys (converting
them to X25519 / X448 internally, as `crypto::convert` does), so a directly
generated `X25519`/`X448` pair is the raw Diffie-Hellman building block for
`crypto::exchange` rather than a direct input to the asymmetric-encryption members.

**Encodings and sizes.** Every field is raw big-endian bytes — no PEM, no
base64, no DER wrapper on the key material itself. For the NIST curves the
`publicKey` is one **SEC1 / X9.62 uncompressed point** `0x04‖X‖Y` (each coordinate
`field` bytes), and the `privateKey` is that same uncompressed point immediately
followed by the big-endian secret scalar `d` (i.e. `0x04‖X‖Y‖d`). For `Ed25519`
the `privateKey` is the 32-byte seed and the `publicKey` is the 32-byte compressed
point; for `X25519` both are 32-byte Curve25519 values, for `X448` both are
56-byte Curve448 values (little-endian `u`-coordinate and clamped scalar), and for
`Ed448` the `privateKey` is the 57-byte seed and the `publicKey` the 57-byte
compressed edwards448 point (56-byte `y` plus the sign byte).

| `type` | Standard | Digest (sign/verify) | `publicKey` | `privateKey` |
| --- | --- | --- | --- | --- |
| `P256` | secp256r1, FIPS 186-4 | SHA-256 | 65 B (`0x04‖X‖Y`) | 97 B (`0x04‖X‖Y‖d`) |
| `P384` | secp384r1, FIPS 186-4 | SHA-384 | 97 B | 145 B |
| `P521` | secp521r1, FIPS 186-4 | SHA-512 | 133 B | 199 B |
| `Ed25519` | RFC 8032 | SHA-512 (internal) | 32 B | 32 B (seed) |
| `X25519` | RFC 7748 | — (not a signing key) | 32 B | 32 B |
| `X448` | RFC 7748 | — (not a signing key) | 56 B | 56 B |
| `Ed448` | RFC 8032 | SHAKE256 (internal) | 57 B | 57 B (seed) |

**Interoperating with other tools.** The `publicKey` needs no conversion: on the
NIST curves it is the standard SEC1 / X9.62 uncompressed point that OpenSSL, a
JWK and a PKCS#11 token all recognise. The `privateKey` does. `0x04‖X‖Y‖d` is
this package's own layout — carrying the public point is what lets `crypto::sign`
take the private key alone — and nothing outside MFBASIC reads it. Convert at the
boundary: the key material is the same either way, only the framing differs.

The two halves sit at fixed offsets. The leading `1 + 2*field` bytes are exactly
the `publicKey` (65 / 97 / 133 for `P256` / `P384` / `P521`); the trailing `field`
bytes are the SEC1 secret scalar `d` (32 / 48 / 66). In the other direction a
package `privateKey` is `publicKey ‖ d`, in that order. The example below cuts a
`P256` key into both.

`d` and the public point are everything a SEC1 `ECPrivateKey` (RFC 5915) holds,
so `openssl` does the DER and PEM framing on its side. Bringing a P-256 key in:

```
openssl ec -in key.pem -outform DER -out key.der
head -c 39 key.der | tail -c 32 > d.bin        # the scalar d
tail -c 65 key.der > pub.bin                   # 0x04||X||Y
cat pub.bin d.bin > privatekey.bin             # what sign() takes
```

and taking one out, from the 97 bytes `generate` returned:

```
head -c 65 privatekey.bin > pub.bin
tail -c 32 privatekey.bin > d.bin
{ printf '\x30\x77\x02\x01\x01\x04\x20'; cat d.bin
  printf '\xa0\x0a\x06\x08\x2a\x86\x48\xce\x3d\x03\x01\x07\xa1\x44\x03\x42\x00'
  cat pub.bin; } > key.der
openssl ec -inform DER -in key.der -out key.pem
```

Both were run on OpenSSL 3.6.2, and a signature made on each side was checked on
the other. The literal bytes in the second are the rest of the SEC1 structure —
the version, the `prime256v1` object identifier and the public-key tag — and they
are P-256 constants. `P384` and `P521` need their own, and their outer length
takes an extra byte, so on those the scalar starts one byte later: `head -c 56 |
tail -c 48` for P-384, `head -c 74 | tail -c 66` for P-521, with the public point
still the last 97 / 133 bytes. `openssl asn1parse -in key.pem` prints the offsets
if you would rather read them off than count them.

A different thing that looks similar: `tls::listen`'s `certPath` and `keyPath`
name PEM files the platform TLS stack reads for itself. They are not
`crypto::KeyPair` material and none of the above applies to them.

**Security.** The `privateKey` is secret key material — keep it confidential and
never transmit or log it; only the `publicKey` is safe to share. Randomness comes
from the platform CSPRNG (for the NIST curves) or `crypto::randomBytes` (for the
software curves).

**Implementation.** The three NIST curves are generated through the host
platform's key API, reproduced clean-room in this member (no third-party crypto is
bundled): on **macOS** via Security.framework `SecKey` (`SecKeyCreateRandomKey`,
EC key type, exported with `SecKeyCopyExternalRepresentation`); on **Linux** via
OpenSSL `libcrypto` (`EVP_EC_gen` on 3.x, else `EC_KEY_new_by_curve_name` +
`EC_KEY_generate_key`, serialized through `i2d_PrivateKey`/`i2d_PUBKEY`); on
**Windows** via CNG `bcrypt.dll` (`BCryptGenerateKeyPair` +
`BCryptFinalizeKeyPair` + `BCryptExportKey`, algorithms `ECDSA_P256/384/521`,
`ECCPRIVATEBLOB`). `Ed25519`, `X25519`, `X448`, and `Ed448` are a pure in-process
MFBASIC software core (over the `bits` package) with **no platform library**, so they are
byte-identical on every OS. Across platforms the encodings are wire-compatible: a
key made on one OS is accepted by `sign`/`verify` on the others."#;
const EX: &str = r#"Generate an ECDSA P-256 pair and use both halves:

```
IMPORT crypto
IMPORT strings

FUNC main() AS Nothing
  LET kp AS crypto::KeyPair = crypto::generate(crypto::Certificate.P256)
  LET msg AS List OF Byte = strings::toBytes("attack at dawn")
  LET sig AS List OF Byte = crypto::sign(crypto::Certificate.P256, kp.privateKey, msg)
  LET ok AS Boolean = crypto::verify(crypto::Certificate.P256, kp.publicKey, msg, sig)
END FUNC
```

Ed25519 keys are byte-identical on every platform:

```
IMPORT crypto

FUNC main() AS Nothing
  LET kp AS crypto::KeyPair = crypto::generate(crypto::Certificate.Ed25519)
END FUNC
```

Split a NIST `privateKey` into the two pieces other ecosystems ask for — the
uncompressed point, which is the `publicKey` again, and the SEC1 scalar `d`:

```
IMPORT crypto
IMPORT collections
IMPORT encoding
IMPORT io

FUNC main() AS Nothing
  LET kp AS crypto::KeyPair = crypto::generate(crypto::Certificate.P256)
  LET point AS List OF Byte = collections::mid(kp.privateKey, 0, 65)
  LET scalar AS List OF Byte = collections::mid(kp.privateKey, 65, 32)
  IF encoding::hexEncode(point) = encoding::hexEncode(kp.publicKey) THEN
    io::print("privateKey is publicKey (65 bytes) followed by the scalar d")
  END IF
  io::print("d is " & toString(len(scalar)) & " bytes, ready for a SEC1 encoder")
END FUNC
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "generate",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "type",
                desc: "The certificate/key type to generate.",
                aliases: &[],
                ty: ParameterType::named("Certificate"),
                default: crate::codegen::registry::DefaultValue::None,
            }],
            return_type: ParameterType::named("KeyPair"),
            errors: vec!["ErrOutOfMemory", "ErrUnknown"],
            body: Body::abi_function(lower_generate),
        }],
    });
}
