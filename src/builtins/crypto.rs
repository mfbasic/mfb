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

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, BuiltinSource,
    BuiltinType, DefaultResolver, DefaultValue, Implementation, InjectionRule, Lowering, Parameter,
    ParameterType, ReturnType, TypeKind,
};

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

/// The raw NIST key generators are **not user-callable**: they exist for
/// `crypto_package.mfb`'s `__crypto_generateP*` glue, which slices the public
/// point out of the returned `0x04||X||Y||K` bytes and builds a `KeyPair`.
///
/// Unlike `audio`'s lowered-only internals (bug-213), these are reached from
/// injected MFBASIC source and so must stay resolvable there — the exclusion is
/// applied in the resolver, which knows whether the calling file is
/// toolchain-provided (`AstFile::internal`). `scripts/list_functions.py`'s
/// `INTERNAL_CALLS` has always agreed they are internal; the compiler did not
/// (bug-337-D9).
pub(crate) fn is_crypto_internal_call(name: &str) -> bool {
    matches!(
        name,
        GENERATE_P256_RAW | GENERATE_P384_RAW | GENERATE_P521_RAW
    )
}

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

// plan-72-F: `CRYPTO` is the descriptor authority. Every function's return type
// is fixed (the overloading is on ARGUMENT types, not the return), so
// `call_return_type_name` and `arity` derive from the descriptor. `resolve_call`
// (bytes/String overloads, variadic AEAD `aad`) and typed `implementation_name`
// (`_bytes`/`_text`) are argument-dependent and live on `CryptoResolver`.
// `default_argument_padding` (AEAD `aad`) derives from the optional `aad`
// parameter. `Sealed`/`KeyPair` are opaque builtin types; `WhenImported` source.
const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}

const fn cf(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        overloads,
        // Implementation is argument-dependent (native vs typed source body), so
        // the resolver owns it; `DefaultResolver::implementation_name` returns
        // None for `Custom` and the wrapper routes to `CryptoResolver`.
        implementation: Implementation::Custom,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

const fn req(name: &'static str, ty: &'static str) -> Parameter {
    Parameter::required(name, ty)
}

// The AEAD `aad` argument is optional and defaults to the empty byte list; this
// `Fill` is what `DefaultResolver::default_padding` reads to reproduce the legacy
// `default_argument_padding`, and what makes AEAD arity `(3,4)`/`(4,5)`.
const AAD: Parameter = Parameter {
    name: "aad",
    aliases: &[],
    ty: ParameterType::Named(BYTES),
    default: DefaultValue::Fill {
        type_name: BYTES,
        expr: "",
    },
};

const P_DATA: &[Parameter] = &[req("data", BYTES)];
const P_HMAC: &[Parameter] = &[req("key", BYTES), req("data", BYTES)];
const P_HKDF: &[Parameter] = &[
    req("ikm", BYTES),
    req("salt", BYTES),
    req("info", BYTES),
    req("length", "Integer"),
];
const P_PBKDF2: &[Parameter] = &[
    req("password", BYTES),
    req("salt", BYTES),
    req("iterations", "Integer"),
    req("length", "Integer"),
];
const P_SEAL: &[Parameter] = &[req("key", BYTES), req("nonce", BYTES), req("plaintext", BYTES), AAD];
const P_OPEN: &[Parameter] = &[
    req("key", BYTES),
    req("nonce", BYTES),
    req("ciphertext", BYTES),
    req("tag", BYTES),
    AAD,
];
const P_COUNT: &[Parameter] = &[req("count", "Integer")];
const P_MINMAX: &[Parameter] = &[req("min", "Integer"), req("max", "Integer")];
const P_SIGN: &[Parameter] = &[req("privateKey", BYTES), req("message", BYTES)];
const P_VERIFY: &[Parameter] = &[req("publicKey", BYTES), req("message", BYTES), req("signature", BYTES)];
const P_AB: &[Parameter] = &[req("a", BYTES), req("b", BYTES)];

const CRYPTO_FUNCTIONS: &[BuiltinFunction] = &[
    cf(SHA256, "sha256", &[ov(P_DATA, BYTES)]),
    cf(SHA224, "sha224", &[ov(P_DATA, BYTES)]),
    cf(SHA512, "sha512", &[ov(P_DATA, BYTES)]),
    cf(SHA384, "sha384", &[ov(P_DATA, BYTES)]),
    cf(HMAC_SHA256, "hmacSha256", &[ov(P_HMAC, BYTES)]),
    cf(HMAC_SHA512, "hmacSha512", &[ov(P_HMAC, BYTES)]),
    cf(HKDF_SHA256, "hkdfSha256", &[ov(P_HKDF, BYTES)]),
    cf(HKDF_SHA512, "hkdfSha512", &[ov(P_HKDF, BYTES)]),
    cf(PBKDF2_SHA256, "pbkdf2Sha256", &[ov(P_PBKDF2, BYTES)]),
    cf(PBKDF2_SHA512, "pbkdf2Sha512", &[ov(P_PBKDF2, BYTES)]),
    cf(AES256_GCM_SEAL, "aes256GcmSeal", &[ov(P_SEAL, SEALED_TYPE)]),
    cf(AES256_GCM_OPEN, "aes256GcmOpen", &[ov(P_OPEN, BYTES)]),
    cf(CHACHA20_POLY1305_SEAL, "chacha20Poly1305Seal", &[ov(P_SEAL, SEALED_TYPE)]),
    cf(CHACHA20_POLY1305_OPEN, "chacha20Poly1305Open", &[ov(P_OPEN, BYTES)]),
    cf(RANDOM_BYTES, "randomBytes", &[ov(P_COUNT, BYTES)]),
    cf(RANDOM_INT, "randomInt", &[ov(P_MINMAX, "Integer")]),
    cf(UUID4, "uuid4", &[ov(&[], "String")]),
    cf(GENERATE_ED25519, "generateEd25519", &[ov(&[], KEYPAIR_TYPE)]),
    cf(GENERATE_P256, "generateP256", &[ov(&[], KEYPAIR_TYPE)]),
    cf(GENERATE_P384, "generateP384", &[ov(&[], KEYPAIR_TYPE)]),
    cf(GENERATE_P521, "generateP521", &[ov(&[], KEYPAIR_TYPE)]),
    cf(GENERATE_P256_RAW, "generateP256Raw", &[ov(&[], BYTES)]),
    cf(GENERATE_P384_RAW, "generateP384Raw", &[ov(&[], BYTES)]),
    cf(GENERATE_P521_RAW, "generateP521Raw", &[ov(&[], BYTES)]),
    cf(ED25519_SIGN, "ed25519Sign", &[ov(P_SIGN, BYTES)]),
    cf(ED25519_VERIFY, "ed25519Verify", &[ov(P_VERIFY, "Boolean")]),
    cf(P256_SIGN, "p256Sign", &[ov(P_SIGN, BYTES)]),
    cf(P256_VERIFY, "p256Verify", &[ov(P_VERIFY, "Boolean")]),
    cf(P384_SIGN, "p384Sign", &[ov(P_SIGN, BYTES)]),
    cf(P384_VERIFY, "p384Verify", &[ov(P_VERIFY, "Boolean")]),
    cf(P521_SIGN, "p521Sign", &[ov(P_SIGN, BYTES)]),
    cf(P521_VERIFY, "p521Verify", &[ov(P_VERIFY, "Boolean")]),
    cf(CONSTANT_TIME_EQUAL, "constantTimeEqual", &[ov(P_AB, "Boolean")]),
];

const CRYPTO_TYPES: &[BuiltinType] = &[
    BuiltinType {
        name: SEALED_TYPE,
        kind: TypeKind::Opaque,
        fields: &[],
    },
    BuiltinType {
        name: KEYPAIR_TYPE,
        kind: TypeKind::Opaque,
        fields: &[],
    },
];

/// Argument-dependent resolution for crypto: bytes/String overload validation
/// (return type), and the `_bytes`/`_text` typed implementation body selection.
/// Both delegate to the retained `dispatch_*` helpers (the pre-migration logic).
struct CryptoResolver;
impl BuiltinResolver for CryptoResolver {
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        dispatch_resolve(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }

    fn implementation_name(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        dispatch_implementation_name(name, arg_types)
    }
}
static CRYPTO_RESOLVER: CryptoResolver = CryptoResolver;

pub(crate) static CRYPTO: BuiltinModule = BuiltinModule {
    name: "crypto",
    functions: CRYPTO_FUNCTIONS,
    types: CRYPTO_TYPES,
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: Some(&CRYPTO_RESOLVER),
};

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    CRYPTO.types.iter().any(|ty| ty.name == name)
}

pub(crate) fn is_crypto_call(name: &str) -> bool {
    DefaultResolver::contains(&CRYPTO, name)
}

// `call_param_names`, `expected_arguments`, and `argument_types` return `&'static`
// borrowed shapes the owned `DefaultResolver` cannot produce, and
// `expected_arguments`/`argument_types` use bespoke phrasing the descriptor's
// per-position types cannot render. They stay static: `call_param_names` PINNED
// equal to `CRYPTO` by the parity test; the other two not descriptor-derivable
// (documented). BB removes them.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    let params: &'static [&'static [&'static str]] = match name {
        SHA256 | SHA224 | SHA512 | SHA384 => &[&["data"]],
        HMAC_SHA256 | HMAC_SHA512 => &[&["key"], &["data"]],
        HKDF_SHA256 | HKDF_SHA512 => &[&["ikm"], &["salt"], &["info"], &["length"]],
        PBKDF2_SHA256 | PBKDF2_SHA512 => &[&["password"], &["salt"], &["iterations"], &["length"]],
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
    DefaultResolver::return_type_name(&CRYPTO, name)
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    DefaultResolver::arity(&CRYPTO, name)
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    let text = match name {
        SHA256 | SHA224 | SHA512 | SHA384 => "List OF Byte or String",
        HMAC_SHA256 | HMAC_SHA512 => "List OF Byte, (List OF Byte or String)",
        HKDF_SHA256 | HKDF_SHA512 => "List OF Byte, List OF Byte, List OF Byte, Integer",
        PBKDF2_SHA256 | PBKDF2_SHA512 => "(List OF Byte or String), List OF Byte, Integer, Integer",
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
    let return_type = CRYPTO
        .resolver?
        .resolve_return_type(&CRYPTO, name, arg_types)?;
    Some(ResolvedCall {
        return_type: Cow::Owned(return_type),
    })
}

/// The argument-validating return-type resolution, invoked through the descriptor
/// resolver by `resolve_call`. Overloaded hashes/HMAC/PBKDF2 accept `List OF Byte`
/// or `String`; AEAD accepts an optional trailing `aad`.
fn dispatch_resolve<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
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
        HKDF_SHA256 | HKDF_SHA512 if exact(arg_types, &[BYTES, BYTES, BYTES, "Integer"]) => BYTES,
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
        GENERATE_P256_RAW | GENERATE_P384_RAW | GENERATE_P521_RAW if arg_types.is_empty() => BYTES,
        ED25519_SIGN | P256_SIGN | P384_SIGN | P521_SIGN if exact(arg_types, &[BYTES, BYTES]) => {
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
/// literal (mirroring `http`'s empty-map default padding). Returns a `&'static`
/// borrowed slice the owned `DefaultResolver::default_padding` cannot produce, so
/// it stays static — PINNED equal to `CRYPTO`'s optional `aad` parameter by the
/// parity test (`DefaultResolver::default_padding` derives the same slots).
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
    CRYPTO
        .resolver?
        .implementation_name(&CRYPTO, name, arg_types)
}

/// The typed `__crypto_*` implementation selection, invoked through the descriptor
/// resolver by `implementation_name`.
fn dispatch_implementation_name(name: &str, arg_types: &[String]) -> Option<String> {
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

// The crypto companion is authored across five topic sources concatenated in
// declaration order (MFBASIC resolution is order-sensitive). The join is
// byte-exact — each file holds a contiguous line range of the former single
// `crypto_package.mfb`, so the parsed source is identical (bug-327 T1-4).
super::package_source_glue!(
    "crypto",
    "<builtin-crypto>",
    "builtins/crypto.mfb",
    concat!(
        include_str!("crypto_hash.mfb"),
        include_str!("crypto_aead.mfb"),
        include_str!("crypto_util.mfb"),
        include_str!("crypto_ed25519.mfb"),
        include_str!("crypto_ecdsa.mfb"),
    )
);

use super::exact;

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn ret(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &strings(args)).map(|r| r.return_type.into_owned())
    }

    fn project(src: &str) -> crate::ast::AstProject {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse source");
        crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    }

    #[test]
    fn is_native_crypto_call_flags() {
        for f in [
            RANDOM_BYTES,
            GENERATE_P256_RAW,
            GENERATE_P384_RAW,
            GENERATE_P521_RAW,
            P256_SIGN,
            P256_VERIFY,
            P384_SIGN,
            P384_VERIFY,
            P521_SIGN,
            P521_VERIFY,
        ] {
            assert!(is_native_crypto_call(f), "{f}");
        }
        assert!(!is_native_crypto_call(SHA256));
        assert!(!is_native_crypto_call(ED25519_SIGN));
        assert!(!is_native_crypto_call("crypto.bogus"));
    }

    #[test]
    fn builtin_types() {
        assert!(is_builtin_type(SEALED_TYPE));
        assert!(is_builtin_type(KEYPAIR_TYPE));
        assert!(!is_builtin_type("Nope"));
    }

    #[test]
    fn is_crypto_call_flags() {
        for f in [
            SHA256,
            SHA224,
            SHA512,
            SHA384,
            HMAC_SHA256,
            HMAC_SHA512,
            HKDF_SHA256,
            HKDF_SHA512,
            PBKDF2_SHA256,
            PBKDF2_SHA512,
            AES256_GCM_SEAL,
            AES256_GCM_OPEN,
            CHACHA20_POLY1305_SEAL,
            CHACHA20_POLY1305_OPEN,
            RANDOM_BYTES,
            RANDOM_INT,
            UUID4,
            GENERATE_ED25519,
            GENERATE_P256,
            GENERATE_P384,
            GENERATE_P521,
            GENERATE_P256_RAW,
            GENERATE_P384_RAW,
            GENERATE_P521_RAW,
            ED25519_SIGN,
            ED25519_VERIFY,
            P256_SIGN,
            P256_VERIFY,
            P384_SIGN,
            P384_VERIFY,
            P521_SIGN,
            P521_VERIFY,
            CONSTANT_TIME_EQUAL,
        ] {
            assert!(is_crypto_call(f), "{f}");
        }
        assert!(!is_crypto_call("crypto.bogus"));
    }

    #[test]
    fn call_param_names_shapes() {
        assert_eq!(call_param_names(SHA256), Some(&[&["data"][..]][..]));
        assert!(call_param_names(HMAC_SHA256).is_some());
        assert!(call_param_names(HKDF_SHA256).is_some());
        assert!(call_param_names(PBKDF2_SHA256).is_some());
        assert!(call_param_names(AES256_GCM_SEAL).is_some());
        assert!(call_param_names(AES256_GCM_OPEN).is_some());
        assert!(call_param_names(RANDOM_BYTES).is_some());
        assert!(call_param_names(RANDOM_INT).is_some());
        assert_eq!(call_param_names(UUID4), Some(&[][..]));
        assert_eq!(call_param_names(GENERATE_P256_RAW), Some(&[][..]));
        assert!(call_param_names(ED25519_SIGN).is_some());
        assert!(call_param_names(ED25519_VERIFY).is_some());
        assert!(call_param_names(CONSTANT_TIME_EQUAL).is_some());
        assert_eq!(call_param_names("crypto.bogus"), None);
    }

    #[test]
    fn call_return_type_names() {
        assert_eq!(call_return_type_name(SHA256), Some(BYTES));
        assert_eq!(call_return_type_name(HMAC_SHA512), Some(BYTES));
        assert_eq!(call_return_type_name(HKDF_SHA256), Some(BYTES));
        assert_eq!(call_return_type_name(PBKDF2_SHA256), Some(BYTES));
        assert_eq!(call_return_type_name(AES256_GCM_OPEN), Some(BYTES));
        assert_eq!(call_return_type_name(RANDOM_BYTES), Some(BYTES));
        assert_eq!(call_return_type_name(ED25519_SIGN), Some(BYTES));
        assert_eq!(call_return_type_name(GENERATE_P256_RAW), Some(BYTES));
        assert_eq!(call_return_type_name(AES256_GCM_SEAL), Some(SEALED_TYPE));
        assert_eq!(
            call_return_type_name(CHACHA20_POLY1305_SEAL),
            Some(SEALED_TYPE)
        );
        assert_eq!(call_return_type_name(GENERATE_ED25519), Some(KEYPAIR_TYPE));
        assert_eq!(call_return_type_name(GENERATE_P521), Some(KEYPAIR_TYPE));
        assert_eq!(call_return_type_name(RANDOM_INT), Some("Integer"));
        assert_eq!(call_return_type_name(UUID4), Some("String"));
        assert_eq!(call_return_type_name(ED25519_VERIFY), Some("Boolean"));
        assert_eq!(call_return_type_name(CONSTANT_TIME_EQUAL), Some("Boolean"));
        assert_eq!(call_return_type_name("crypto.bogus"), None);
    }

    #[test]
    fn arity_spans() {
        assert_eq!(arity(UUID4), Some((0, 0)));
        assert_eq!(arity(GENERATE_ED25519), Some((0, 0)));
        assert_eq!(arity(GENERATE_P256_RAW), Some((0, 0)));
        assert_eq!(arity(RANDOM_BYTES), Some((1, 1)));
        assert_eq!(arity(SHA256), Some((1, 1)));
        assert_eq!(arity(RANDOM_INT), Some((2, 2)));
        assert_eq!(arity(HMAC_SHA256), Some((2, 2)));
        assert_eq!(arity(ED25519_SIGN), Some((2, 2)));
        assert_eq!(arity(CONSTANT_TIME_EQUAL), Some((2, 2)));
        assert_eq!(arity(ED25519_VERIFY), Some((3, 3)));
        assert_eq!(arity(AES256_GCM_SEAL), Some((3, 4)));
        assert_eq!(arity(HKDF_SHA256), Some((4, 4)));
        assert_eq!(arity(PBKDF2_SHA256), Some((4, 4)));
        assert_eq!(arity(AES256_GCM_OPEN), Some((4, 5)));
        assert_eq!(arity("crypto.bogus"), None);
    }

    #[test]
    fn expected_arguments_present() {
        assert!(expected_arguments(SHA256).unwrap().contains("String"));
        assert!(expected_arguments(HMAC_SHA256).is_some());
        assert!(expected_arguments(HKDF_SHA256).is_some());
        assert!(expected_arguments(PBKDF2_SHA256).is_some());
        assert!(expected_arguments(AES256_GCM_SEAL).is_some());
        assert!(expected_arguments(AES256_GCM_OPEN).is_some());
        assert_eq!(expected_arguments(RANDOM_BYTES), Some("Integer"));
        assert_eq!(expected_arguments(RANDOM_INT), Some("Integer, Integer"));
        assert_eq!(expected_arguments(UUID4), Some("()"));
        assert!(expected_arguments(ED25519_SIGN).is_some());
        assert!(expected_arguments(ED25519_VERIFY).is_some());
        assert!(expected_arguments(CONSTANT_TIME_EQUAL).is_some());
        assert_eq!(expected_arguments("crypto.bogus"), None);
    }

    #[test]
    fn resolve_hashes_overloaded() {
        assert_eq!(ret(SHA256, &[BYTES]), Some(BYTES.to_string()));
        assert_eq!(ret(SHA256, &["String"]), Some(BYTES.to_string()));
        assert_eq!(ret(SHA224, &[BYTES]), Some(BYTES.to_string()));
        assert_eq!(ret(SHA512, &["String"]), Some(BYTES.to_string()));
        assert_eq!(ret(SHA384, &[BYTES]), Some(BYTES.to_string()));
        assert_eq!(ret(SHA256, &["Integer"]), None);
        assert_eq!(ret(SHA256, &[]), None);
        assert_eq!(ret(SHA256, &[BYTES, BYTES]), None);
    }

    #[test]
    fn resolve_hmac() {
        assert_eq!(ret(HMAC_SHA256, &[BYTES, BYTES]), Some(BYTES.to_string()));
        assert_eq!(
            ret(HMAC_SHA512, &[BYTES, "String"]),
            Some(BYTES.to_string())
        );
        // key must be bytes
        assert_eq!(ret(HMAC_SHA256, &["String", BYTES]), None);
    }

    #[test]
    fn resolve_hkdf_and_pbkdf2() {
        assert_eq!(
            ret(HKDF_SHA256, &[BYTES, BYTES, BYTES, "Integer"]),
            Some(BYTES.to_string())
        );
        assert_eq!(ret(HKDF_SHA512, &[BYTES, BYTES, BYTES]), None);
        assert_eq!(
            ret(PBKDF2_SHA256, &[BYTES, BYTES, "Integer", "Integer"]),
            Some(BYTES.to_string())
        );
        assert_eq!(
            ret(PBKDF2_SHA512, &["String", BYTES, "Integer", "Integer"]),
            Some(BYTES.to_string())
        );
        // salt must be bytes
        assert_eq!(
            ret(PBKDF2_SHA256, &[BYTES, "String", "Integer", "Integer"]),
            None
        );
    }

    #[test]
    fn resolve_aead() {
        assert_eq!(
            ret(AES256_GCM_SEAL, &[BYTES, BYTES, BYTES]),
            Some(SEALED_TYPE.to_string())
        );
        assert_eq!(
            ret(AES256_GCM_SEAL, &[BYTES, BYTES, BYTES, BYTES]),
            Some(SEALED_TYPE.to_string())
        );
        assert_eq!(
            ret(CHACHA20_POLY1305_SEAL, &[BYTES, BYTES, BYTES]),
            Some(SEALED_TYPE.to_string())
        );
        assert_eq!(
            ret(AES256_GCM_OPEN, &[BYTES, BYTES, BYTES, BYTES]),
            Some(BYTES.to_string())
        );
        assert_eq!(
            ret(AES256_GCM_OPEN, &[BYTES, BYTES, BYTES, BYTES, BYTES]),
            Some(BYTES.to_string())
        );
        assert_eq!(
            ret(CHACHA20_POLY1305_OPEN, &[BYTES, BYTES, BYTES, BYTES]),
            Some(BYTES.to_string())
        );
        assert_eq!(ret(AES256_GCM_SEAL, &[BYTES, BYTES]), None);
        assert_eq!(ret(AES256_GCM_OPEN, &[BYTES, BYTES, BYTES]), None);
    }

    #[test]
    fn resolve_random_and_uuid() {
        assert_eq!(ret(RANDOM_BYTES, &["Integer"]), Some(BYTES.to_string()));
        assert_eq!(ret(RANDOM_BYTES, &[BYTES]), None);
        assert_eq!(
            ret(RANDOM_INT, &["Integer", "Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(ret(UUID4, &[]), Some("String".to_string()));
        assert_eq!(ret(UUID4, &["Integer"]), None);
    }

    #[test]
    fn resolve_keygen() {
        assert_eq!(ret(GENERATE_ED25519, &[]), Some(KEYPAIR_TYPE.to_string()));
        assert_eq!(ret(GENERATE_P256, &[]), Some(KEYPAIR_TYPE.to_string()));
        assert_eq!(ret(GENERATE_P384, &[]), Some(KEYPAIR_TYPE.to_string()));
        assert_eq!(ret(GENERATE_P521, &[]), Some(KEYPAIR_TYPE.to_string()));
        assert_eq!(ret(GENERATE_P256_RAW, &[]), Some(BYTES.to_string()));
        assert_eq!(ret(GENERATE_P384_RAW, &[]), Some(BYTES.to_string()));
        assert_eq!(ret(GENERATE_P521_RAW, &[]), Some(BYTES.to_string()));
        assert_eq!(ret(GENERATE_ED25519, &["Integer"]), None);
        assert_eq!(ret(GENERATE_P256_RAW, &["Integer"]), None);
    }

    #[test]
    fn resolve_sign_verify_and_ct_equal() {
        assert_eq!(ret(ED25519_SIGN, &[BYTES, BYTES]), Some(BYTES.to_string()));
        assert_eq!(ret(P256_SIGN, &[BYTES, BYTES]), Some(BYTES.to_string()));
        assert_eq!(ret(P521_SIGN, &[BYTES, BYTES]), Some(BYTES.to_string()));
        assert_eq!(
            ret(ED25519_VERIFY, &[BYTES, BYTES, BYTES]),
            Some("Boolean".to_string())
        );
        assert_eq!(
            ret(P384_VERIFY, &[BYTES, BYTES, BYTES]),
            Some("Boolean".to_string())
        );
        assert_eq!(
            ret(CONSTANT_TIME_EQUAL, &[BYTES, BYTES]),
            Some("Boolean".to_string())
        );
        assert_eq!(ret(ED25519_SIGN, &[BYTES]), None);
        assert_eq!(ret(CONSTANT_TIME_EQUAL, &[BYTES, "String"]), None);
        assert_eq!(ret("crypto.bogus", &[BYTES]), None);
    }

    #[test]
    fn argument_types_present_and_none() {
        assert_eq!(
            argument_types(HKDF_SHA256),
            Some("List OF Byte, List OF Byte, List OF Byte, Integer")
        );
        assert_eq!(argument_types(RANDOM_BYTES), Some("Integer"));
        assert_eq!(argument_types(RANDOM_INT), Some("Integer, Integer"));
        assert_eq!(
            argument_types(CONSTANT_TIME_EQUAL),
            Some("List OF Byte, List OF Byte")
        );
        assert_eq!(
            argument_types(ED25519_SIGN),
            Some("List OF Byte, List OF Byte")
        );
        assert_eq!(
            argument_types(ED25519_VERIFY),
            Some("List OF Byte, List OF Byte, List OF Byte")
        );
        // overloaded / source calls -> None
        assert_eq!(argument_types(SHA256), None);
        assert_eq!(argument_types(HMAC_SHA256), None);
        assert_eq!(argument_types("crypto.bogus"), None);
    }

    #[test]
    fn default_argument_padding_variants() {
        // seal: with 3 provided, one default; with 4 provided, none
        assert_eq!(default_argument_padding(AES256_GCM_SEAL, 3).len(), 1);
        assert_eq!(default_argument_padding(AES256_GCM_SEAL, 4).len(), 0);
        assert_eq!(default_argument_padding(CHACHA20_POLY1305_SEAL, 3).len(), 1);
        // open: with 4 provided, one default; with 5 provided, none
        assert_eq!(default_argument_padding(AES256_GCM_OPEN, 4).len(), 1);
        assert_eq!(default_argument_padding(AES256_GCM_OPEN, 5).len(), 0);
        assert_eq!(default_argument_padding(CHACHA20_POLY1305_OPEN, 4).len(), 1);
        assert_eq!(default_argument_padding(SHA256, 1).len(), 0);
    }

    #[test]
    fn implementation_name_native_and_source() {
        // native -> None
        assert_eq!(implementation_name(RANDOM_BYTES, &[]), None);
        assert_eq!(
            implementation_name(P256_SIGN, &strings(&[BYTES, BYTES])),
            None
        );
        // hash bytes vs text
        assert_eq!(
            implementation_name(SHA256, &strings(&[BYTES])),
            Some("__crypto_sha256_bytes".to_string())
        );
        assert_eq!(
            implementation_name(SHA256, &strings(&["String"])),
            Some("__crypto_sha256_text".to_string())
        );
        assert_eq!(
            implementation_name(SHA224, &strings(&["String"])),
            Some("__crypto_sha224_text".to_string())
        );
        assert_eq!(
            implementation_name(SHA512, &strings(&[BYTES])),
            Some("__crypto_sha512_bytes".to_string())
        );
        assert_eq!(
            implementation_name(SHA384, &strings(&["String"])),
            Some("__crypto_sha384_text".to_string())
        );
        // hmac selects on arg index 1
        assert_eq!(
            implementation_name(HMAC_SHA256, &strings(&[BYTES, "String"])),
            Some("__crypto_hmacSha256_text".to_string())
        );
        assert_eq!(
            implementation_name(HMAC_SHA512, &strings(&[BYTES, BYTES])),
            Some("__crypto_hmacSha512_bytes".to_string())
        );
        // pbkdf2 selects on arg index 0
        assert_eq!(
            implementation_name(PBKDF2_SHA256, &strings(&["String", BYTES])),
            Some("__crypto_pbkdf2Sha256_text".to_string())
        );
        assert_eq!(
            implementation_name(PBKDF2_SHA512, &strings(&[BYTES, BYTES])),
            Some("__crypto_pbkdf2Sha512_bytes".to_string())
        );
        // default arm: strips crypto. prefix
        assert_eq!(
            implementation_name(UUID4, &[]),
            Some("__crypto_uuid4".to_string())
        );
        assert_eq!(
            implementation_name(GENERATE_ED25519, &[]),
            Some("__crypto_generateEd25519".to_string())
        );
    }

    #[test]
    fn exact_helper() {
        assert!(exact(&strings(&[BYTES, BYTES]), &[BYTES, BYTES]));
        assert!(!exact(&strings(&[BYTES]), &[BYTES, BYTES]));
        assert!(!exact(&strings(&["String"]), &[BYTES]));
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT crypto\nSUB main\nEND SUB\n");
        assert!(uses_package(&ast));
        let augmented = augmented_project(&ast).expect("augment");
        assert_eq!(augmented.files.len(), ast.files.len() + 1);
    }

    #[test]
    fn augmented_project_noop_without_import() {
        let ast = project("SUB main\nEND SUB\n");
        assert!(!uses_package(&ast));
        assert_eq!(
            augmented_project(&ast).expect("a").files.len(),
            ast.files.len()
        );
    }

    // plan-72-F migration gate: prove `CRYPTO` + `CryptoResolver` reproduce the
    // legacy answers — membership/arity/param-names via the descriptor, typed
    // return + implementation via resolver samples, AEAD default padding derived
    // from the optional `aad` parameter, and the two opaque builtin types. Keep
    // until plan-72-BB.
    #[test]
    fn parity_matches_descriptor() {
        use crate::builtins::descriptor::{parity, DefaultResolver};

        let calls: Vec<&str> = CRYPTO_FUNCTIONS.iter().map(|f| f.name).collect();
        let legacy = parity::LegacySet {
            is_call: &is_crypto_call,
            arity: &arity,
            param_names: &|name| {
                call_param_names(name).map(|rows| rows.iter().map(|row| row.to_vec()).collect())
            },
            // Return/impl/padding are resolver- or descriptor-owned and checked
            // below; the harness skips them for a resolver-backed module.
            return_type_name: &|_| None,
            expected_arguments: None, // custom "or"-phrased strings
            param_name_overloads: None,
            argument_types: None, // custom joined strings
            implementation_name: None,
            default_padding: None,
            // Sealed/KeyPair are opaque (no record fields) and crypto has no
            // builtin_type_fields helper; membership is asserted below.
            builtin_type_fields: None,
        };
        let mut probe = calls.clone();
        probe.push("crypto.bogus");
        assert!(is_builtin_type(SEALED_TYPE));
        assert!(is_builtin_type(KEYPAIR_TYPE));
        assert!(!is_builtin_type("Nope"));

        let samples = [
            // hash: bytes vs text implementation selection.
            parity::ResolverSample {
                call: SHA256, arg_types: &[BYTES], expected_return: Some(BYTES),
                expected_impl: Some("__crypto_sha256_bytes"), expected_padding: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: SHA256, arg_types: &["String"], expected_return: Some(BYTES),
                expected_impl: Some("__crypto_sha256_text"), expected_padding: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: HMAC_SHA256, arg_types: &[BYTES, "String"], expected_return: Some(BYTES),
                expected_impl: Some("__crypto_hmacSha256_text"), expected_padding: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: AES256_GCM_SEAL, arg_types: &[BYTES, BYTES, BYTES], expected_return: Some(SEALED_TYPE),
                expected_impl: Some("__crypto_aes256GcmSeal"), expected_padding: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: GENERATE_P256, arg_types: &[], expected_return: Some(KEYPAIR_TYPE),
                expected_impl: Some("__crypto_generateP256"), expected_padding: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: UUID4, arg_types: &[], expected_return: Some("String"),
                expected_impl: Some("__crypto_uuid4"), expected_padding: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: CONSTANT_TIME_EQUAL, arg_types: &[BYTES, BYTES], expected_return: Some("Boolean"),
                expected_impl: Some("__crypto_constantTimeEqual"), expected_padding: None,
                expected_overload_target: None,
            },
        ];
        parity::assert_parity(&CRYPTO, &probe, &legacy, &samples);

        // call_return_type_name (descriptor-derived Fixed) matches legacy values,
        // and AEAD default padding derives from the optional `aad` parameter.
        assert_eq!(call_return_type_name(SHA256), Some(BYTES));
        assert_eq!(call_return_type_name(AES256_GCM_SEAL), Some(SEALED_TYPE));
        assert_eq!(call_return_type_name(GENERATE_ED25519), Some(KEYPAIR_TYPE));
        for name in [AES256_GCM_SEAL, CHACHA20_POLY1305_SEAL, AES256_GCM_OPEN, CHACHA20_POLY1305_OPEN] {
            let (_, max) = arity(name).unwrap();
            for provided in 0..=max {
                assert_eq!(
                    DefaultResolver::default_padding(&CRYPTO, name, provided),
                    default_argument_padding(name, provided).to_vec(),
                    "padding parity {name} @ {provided}"
                );
            }
        }

        // native entry points resolve to no source implementation.
        assert_eq!(implementation_name(RANDOM_BYTES, &strings(&["Integer"])), None);
        assert_eq!(implementation_name(P256_SIGN, &strings(&[BYTES, BYTES])), None);
    }
}
