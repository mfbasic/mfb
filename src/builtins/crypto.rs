//! Built-in `crypto::` package seam (plan-04-crypto.md).
//!
//! Mirrors `encoding`/`datetime`: the cryptographic algorithms live in
//! `crypto_package.mfb` as internal `__crypto_*` functions (portable software
//! cores over the `bits` package — see the plan's hybrid backend note), and this
//! module owns registration, syntaxcheck metadata, and the mapping from a public
//! `crypto::` call onto its internal implementation. A small number of entry
//! points are **native runtime helpers** instead of source (they return `None`
//! from `implementation_name` and route through `runtime::helper_for_call`):
//! `randomBytes` (OS entropy) and the NIST-EC public-key operations (bound to the
//! platform's modern key API — `SecKey` on macOS, `EVP_PKEY` on Linux).
//!
//! The String-argument overloads of the hashes/HMAC/PBKDF2 are resolved by a
//! type-aware `implementation_name` (like `vector::`): the concrete argument
//! types select a `_bytes`/`_text` internal body, so no monomorph-level overload
//! plumbing is required.

use std::borrow::Cow;
use std::path::Path;

pub(crate) const SEALED_TYPE: &str = "Sealed";
pub(crate) const KEYPAIR_TYPE: &str = "KeyPair";

const BYTES: &str = "List OF Byte";

// Hashes (source). Each has a `List OF Byte` and a `String` overload.
const SHA256: &str = "crypto.sha256";
const SHA224: &str = "crypto.sha224";
const SHA512: &str = "crypto.sha512";
const SHA384: &str = "crypto.sha384";
// HMAC (source). Overloaded on the `data` argument.
const HMAC_SHA256: &str = "crypto.hmacSha256";
const HMAC_SHA512: &str = "crypto.hmacSha512";
// Key derivation (source). PBKDF2 is overloaded on the `password` argument.
const HKDF_SHA256: &str = "crypto.hkdfSha256";
const HKDF_SHA512: &str = "crypto.hkdfSha512";
const PBKDF2_SHA256: &str = "crypto.pbkdf2Sha256";
const PBKDF2_SHA512: &str = "crypto.pbkdf2Sha512";
// AEAD (source). `aad` defaults to the empty list.
const AES256_GCM_SEAL: &str = "crypto.aes256GcmSeal";
const AES256_GCM_OPEN: &str = "crypto.aes256GcmOpen";
const CHACHA20_POLY1305_SEAL: &str = "crypto.chacha20Poly1305Seal";
const CHACHA20_POLY1305_OPEN: &str = "crypto.chacha20Poly1305Open";
// Secure random. `randomBytes` is a native runtime helper (OS entropy);
// `randomInt`/`uuid4` are source glue over it.
const RANDOM_BYTES: &str = "crypto.randomBytes";
const RANDOM_INT: &str = "crypto.randomInt";
const UUID4: &str = "crypto.uuid4";
// Public-key. Ed25519 is a source software core; the NIST curves are native.
const GENERATE_ED25519: &str = "crypto.generateEd25519";
const GENERATE_P256: &str = "crypto.generateP256";
const GENERATE_P384: &str = "crypto.generateP384";
const GENERATE_P521: &str = "crypto.generateP521";
// Raw NIST keygen (native): returns the private bytes `0x04||X||Y||K`. The
// public `generateP*` above is source glue that slices out the public point and
// builds a `KeyPair`, so it stays a source call while these route to the helper.
const GENERATE_P256_RAW: &str = "crypto.generateP256Raw";
const GENERATE_P384_RAW: &str = "crypto.generateP384Raw";
const GENERATE_P521_RAW: &str = "crypto.generateP521Raw";
const ED25519_SIGN: &str = "crypto.ed25519Sign";
const ED25519_VERIFY: &str = "crypto.ed25519Verify";
const P256_SIGN: &str = "crypto.p256Sign";
const P256_VERIFY: &str = "crypto.p256Verify";
const P384_SIGN: &str = "crypto.p384Sign";
const P384_VERIFY: &str = "crypto.p384Verify";
const P521_SIGN: &str = "crypto.p521Sign";
const P521_VERIFY: &str = "crypto.p521Verify";
// Verification.
const CONSTANT_TIME_EQUAL: &str = "crypto.constantTimeEqual";

/// The native (runtime-helper) entry points: they lower to `_mfb_rt_crypto_*`
/// rather than to an injected `__crypto_*` source body, so `implementation_name`
/// returns `None` and `runtime::helper_for_call` claims them.
pub(crate) fn is_native_crypto_call(name: &str) -> bool {
    matches!(
        name,
        RANDOM_BYTES
            | GENERATE_P256_RAW
            | GENERATE_P384_RAW
            | GENERATE_P521_RAW
            | P256_SIGN
            | P256_VERIFY
            | P384_SIGN
            | P384_VERIFY
            | P521_SIGN
            | P521_VERIFY
    )
}

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    matches!(name, SEALED_TYPE | KEYPAIR_TYPE)
}

pub(crate) fn is_crypto_call(name: &str) -> bool {
    matches!(
        name,
        SHA256
            | SHA224
            | SHA512
            | SHA384
            | HMAC_SHA256
            | HMAC_SHA512
            | HKDF_SHA256
            | HKDF_SHA512
            | PBKDF2_SHA256
            | PBKDF2_SHA512
            | AES256_GCM_SEAL
            | AES256_GCM_OPEN
            | CHACHA20_POLY1305_SEAL
            | CHACHA20_POLY1305_OPEN
            | RANDOM_BYTES
            | RANDOM_INT
            | UUID4
            | GENERATE_ED25519
            | GENERATE_P256
            | GENERATE_P384
            | GENERATE_P521
            | GENERATE_P256_RAW
            | GENERATE_P384_RAW
            | GENERATE_P521_RAW
            | ED25519_SIGN
            | ED25519_VERIFY
            | P256_SIGN
            | P256_VERIFY
            | P384_SIGN
            | P384_VERIFY
            | P521_SIGN
            | P521_VERIFY
            | CONSTANT_TIME_EQUAL
    )
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    let params: &'static [&'static [&'static str]] = match name {
        SHA256 | SHA224 | SHA512 | SHA384 => &[&["data"]],
        HMAC_SHA256 | HMAC_SHA512 => &[&["key"], &["data"]],
        HKDF_SHA256 | HKDF_SHA512 => &[&["ikm"], &["salt"], &["info"], &["length"]],
        PBKDF2_SHA256 | PBKDF2_SHA512 => {
            &[&["password"], &["salt"], &["iterations"], &["length"]]
        }
        AES256_GCM_SEAL | CHACHA20_POLY1305_SEAL => {
            &[&["key"], &["nonce"], &["plaintext"], &["aad"]]
        }
        AES256_GCM_OPEN | CHACHA20_POLY1305_OPEN => {
            &[&["key"], &["nonce"], &["ciphertext"], &["tag"], &["aad"]]
        }
        RANDOM_BYTES => &[&["count"]],
        RANDOM_INT => &[&["min"], &["max"]],
        UUID4 | GENERATE_ED25519 | GENERATE_P256 | GENERATE_P384 | GENERATE_P521
        | GENERATE_P256_RAW | GENERATE_P384_RAW | GENERATE_P521_RAW => &[],
        ED25519_SIGN | P256_SIGN | P384_SIGN | P521_SIGN => &[&["privateKey"], &["message"]],
        ED25519_VERIFY | P256_VERIFY | P384_VERIFY | P521_VERIFY => {
            &[&["publicKey"], &["message"], &["signature"]]
        }
        CONSTANT_TIME_EQUAL => &[&["a"], &["b"]],
        _ => return None,
    };
    Some(params)
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    let type_ = match name {
        SHA256 | SHA224 | SHA512 | SHA384 | HMAC_SHA256 | HMAC_SHA512 | HKDF_SHA256
        | HKDF_SHA512 | PBKDF2_SHA256 | PBKDF2_SHA512 | AES256_GCM_OPEN
        | CHACHA20_POLY1305_OPEN | RANDOM_BYTES | ED25519_SIGN | P256_SIGN | P384_SIGN
        | P521_SIGN | GENERATE_P256_RAW | GENERATE_P384_RAW | GENERATE_P521_RAW => BYTES,
        AES256_GCM_SEAL | CHACHA20_POLY1305_SEAL => SEALED_TYPE,
        GENERATE_ED25519 | GENERATE_P256 | GENERATE_P384 | GENERATE_P521 => KEYPAIR_TYPE,
        RANDOM_INT => "Integer",
        UUID4 => "String",
        ED25519_VERIFY | P256_VERIFY | P384_VERIFY | P521_VERIFY | CONSTANT_TIME_EQUAL => {
            "Boolean"
        }
        _ => return None,
    };
    Some(type_)
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    let span = match name {
        UUID4 | GENERATE_ED25519 | GENERATE_P256 | GENERATE_P384 | GENERATE_P521
        | GENERATE_P256_RAW | GENERATE_P384_RAW | GENERATE_P521_RAW => (0, 0),
        RANDOM_BYTES => (1, 1),
        SHA256 | SHA224 | SHA512 | SHA384 => (1, 1),
        RANDOM_INT | HMAC_SHA256 | HMAC_SHA512 | CONSTANT_TIME_EQUAL | ED25519_SIGN
        | P256_SIGN | P384_SIGN | P521_SIGN => (2, 2),
        ED25519_VERIFY | P256_VERIFY | P384_VERIFY | P521_VERIFY => (3, 3),
        AES256_GCM_SEAL | CHACHA20_POLY1305_SEAL => (3, 4),
        HKDF_SHA256 | HKDF_SHA512 | PBKDF2_SHA256 | PBKDF2_SHA512 => (4, 4),
        AES256_GCM_OPEN | CHACHA20_POLY1305_OPEN => (4, 5),
        _ => return None,
    };
    Some(span)
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    let text = match name {
        SHA256 | SHA224 | SHA512 | SHA384 => "List OF Byte or String",
        HMAC_SHA256 | HMAC_SHA512 => "List OF Byte, (List OF Byte or String)",
        HKDF_SHA256 | HKDF_SHA512 => {
            "List OF Byte, List OF Byte, List OF Byte, Integer"
        }
        PBKDF2_SHA256 | PBKDF2_SHA512 => {
            "(List OF Byte or String), List OF Byte, Integer, Integer"
        }
        AES256_GCM_SEAL | CHACHA20_POLY1305_SEAL => {
            "List OF Byte, List OF Byte, List OF Byte[, List OF Byte]"
        }
        AES256_GCM_OPEN | CHACHA20_POLY1305_OPEN => {
            "List OF Byte, List OF Byte, List OF Byte, List OF Byte[, List OF Byte]"
        }
        RANDOM_BYTES => "Integer",
        RANDOM_INT => "Integer, Integer",
        UUID4 | GENERATE_ED25519 | GENERATE_P256 | GENERATE_P384 | GENERATE_P521
        | GENERATE_P256_RAW | GENERATE_P384_RAW | GENERATE_P521_RAW => "()",
        ED25519_SIGN | P256_SIGN | P384_SIGN | P521_SIGN => "List OF Byte, List OF Byte",
        ED25519_VERIFY | P256_VERIFY | P384_VERIFY | P521_VERIFY => {
            "List OF Byte, List OF Byte, List OF Byte"
        }
        CONSTANT_TIME_EQUAL => "List OF Byte, List OF Byte",
        _ => return None,
    };
    Some(text)
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let bytes_or_text = |t: &str| t == BYTES || t == "String";
    let return_type: &str = match name {
        SHA256 | SHA224 | SHA512 | SHA384
            if arg_types.len() == 1 && bytes_or_text(&arg_types[0]) =>
        {
            BYTES
        }
        HMAC_SHA256 | HMAC_SHA512
            if arg_types.len() == 2 && arg_types[0] == BYTES && bytes_or_text(&arg_types[1]) =>
        {
            BYTES
        }
        HKDF_SHA256 | HKDF_SHA512
            if exact(arg_types, &[BYTES, BYTES, BYTES, "Integer"]) =>
        {
            BYTES
        }
        PBKDF2_SHA256 | PBKDF2_SHA512
            if arg_types.len() == 4
                && bytes_or_text(&arg_types[0])
                && arg_types[1] == BYTES
                && arg_types[2] == "Integer"
                && arg_types[3] == "Integer" =>
        {
            BYTES
        }
        AES256_GCM_SEAL | CHACHA20_POLY1305_SEAL
            if exact(arg_types, &[BYTES, BYTES, BYTES])
                || exact(arg_types, &[BYTES, BYTES, BYTES, BYTES]) =>
        {
            SEALED_TYPE
        }
        AES256_GCM_OPEN | CHACHA20_POLY1305_OPEN
            if exact(arg_types, &[BYTES, BYTES, BYTES, BYTES])
                || exact(arg_types, &[BYTES, BYTES, BYTES, BYTES, BYTES]) =>
        {
            BYTES
        }
        RANDOM_BYTES if exact(arg_types, &["Integer"]) => BYTES,
        RANDOM_INT if exact(arg_types, &["Integer", "Integer"]) => "Integer",
        UUID4 if arg_types.is_empty() => "String",
        GENERATE_ED25519 | GENERATE_P256 | GENERATE_P384 | GENERATE_P521
            if arg_types.is_empty() =>
        {
            KEYPAIR_TYPE
        }
        GENERATE_P256_RAW | GENERATE_P384_RAW | GENERATE_P521_RAW if arg_types.is_empty() => {
            BYTES
        }
        ED25519_SIGN | P256_SIGN | P384_SIGN | P521_SIGN
            if exact(arg_types, &[BYTES, BYTES]) =>
        {
            BYTES
        }
        ED25519_VERIFY | P256_VERIFY | P384_VERIFY | P521_VERIFY
            if exact(arg_types, &[BYTES, BYTES, BYTES]) =>
        {
            "Boolean"
        }
        CONSTANT_TIME_EQUAL if exact(arg_types, &[BYTES, BYTES]) => "Boolean",
        _ => return None,
    };
    Some(ResolvedCall {
        return_type: Cow::Borrowed(return_type),
    })
}

/// Concrete per-argument types for literal coercion. Only calls whose positional
/// types are fixed (not overloaded) return a value; the overloaded hash/HMAC/
/// PBKDF2 forms rely on the argument's own inferred type.
pub(crate) fn argument_types(name: &str) -> Option<&'static str> {
    match name {
        HKDF_SHA256 | HKDF_SHA512 => Some("List OF Byte, List OF Byte, List OF Byte, Integer"),
        RANDOM_BYTES => Some("Integer"),
        RANDOM_INT => Some("Integer, Integer"),
        CONSTANT_TIME_EQUAL | ED25519_SIGN | P256_SIGN | P384_SIGN | P521_SIGN => {
            Some("List OF Byte, List OF Byte")
        }
        ED25519_VERIFY | P256_VERIFY | P384_VERIFY | P521_VERIFY => {
            Some("List OF Byte, List OF Byte, List OF Byte")
        }
        _ => None,
    }
}

/// The `aad` argument of the AEAD operations defaults to the empty byte list.
/// Returned as a `List OF Byte` default so IR lowering injects an empty list
/// literal (mirroring `http`'s empty-map default padding).
pub(crate) fn default_argument_padding(
    name: &str,
    provided: usize,
) -> &'static [(&'static str, &'static str)] {
    const SEAL_DEFAULTS: &[(&str, &str)] = &[("List OF Byte", "")];
    const OPEN_DEFAULTS: &[(&str, &str)] = &[("List OF Byte", "")];
    match name {
        AES256_GCM_SEAL | CHACHA20_POLY1305_SEAL => {
            &SEAL_DEFAULTS[provided.saturating_sub(3).min(SEAL_DEFAULTS.len())..]
        }
        AES256_GCM_OPEN | CHACHA20_POLY1305_OPEN => {
            &OPEN_DEFAULTS[provided.saturating_sub(4).min(OPEN_DEFAULTS.len())..]
        }
        _ => &[],
    }
}

/// The internal `__crypto_*` implementation for a public source call, given the
/// supplied argument types. Native entry points (`randomBytes`, the NIST-EC
/// operations) return `None` — they stay `crypto.*` runtime-helper calls. The
/// hash/HMAC/PBKDF2 functions carry a `String` overload, so the relevant
/// argument's type selects a `_bytes`/`_text` body (type-aware, like `vector::`).
pub(crate) fn implementation_name(name: &str, arg_types: &[String]) -> Option<String> {
    if is_native_crypto_call(name) {
        return None;
    }
    let is_text = |index: usize| arg_types.get(index).map(String::as_str) == Some("String");
    let suffix = |text: bool| if text { "_text" } else { "_bytes" };
    let internal = match name {
        SHA256 => format!("__crypto_sha256{}", suffix(is_text(0))),
        SHA224 => format!("__crypto_sha224{}", suffix(is_text(0))),
        SHA512 => format!("__crypto_sha512{}", suffix(is_text(0))),
        SHA384 => format!("__crypto_sha384{}", suffix(is_text(0))),
        HMAC_SHA256 => format!("__crypto_hmacSha256{}", suffix(is_text(1))),
        HMAC_SHA512 => format!("__crypto_hmacSha512{}", suffix(is_text(1))),
        PBKDF2_SHA256 => format!("__crypto_pbkdf2Sha256{}", suffix(is_text(0))),
        PBKDF2_SHA512 => format!("__crypto_pbkdf2Sha512{}", suffix(is_text(0))),
        _ => format!("__crypto_{}", name.strip_prefix("crypto.")?),
    };
    Some(internal)
}

pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        Path::new("<builtin-crypto>"),
        "builtins/crypto.mfb",
        include_str!("crypto_package.mfb"),
    )
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "crypto")
    })
}

pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    if !uses_package(ast) {
        return Ok(ast.clone());
    }
    let mut augmented = ast.clone();
    augmented.files.push(source_file()?);
    Ok(augmented)
}

fn exact(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == expected.len()
        && arg_types
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| actual == expected)
}
