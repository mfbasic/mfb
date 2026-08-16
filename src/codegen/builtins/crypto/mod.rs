//! Built-in `crypto::` package — migrated onto the clean-room registry
//! (`crate::codegen::registry`), mirroring `datetime`/`csv`/`json`.
//!
//! `crypto` is HETEROGENEOUS. Its symmetric, hashing, AEAD, KDF, Ed25519, and
//! secure-random glue are portable **software cores** written in MFBASIC over the
//! `bits` package; those live in `package.mfb` (the byte-exact concatenation of the
//! former five `crypto_*.mfb` topic files) and every public source member rewrites
//! onto its `__crypto_*` body via [`Body::Rewrite`]. The hash/HMAC/PBKDF2 members
//! carry a `List OF Byte` **and** a `String` overload, modeled as two
//! `Implementation`s whose distinct parameter types make `select` pick the
//! `_bytes`/`_text` body — no resolver, no `implementation_name` hook (the legacy
//! `CryptoResolver` idiom the clean-room `select()` subsumes). The only native
//! members are the OS-entropy CSPRNG (`randomBytes`) and the NIST-EC public-key
//! operations (`generateP*Raw`, `p{256,384,521}{Sign,Verify}`); they are
//! [`Body::native`] OS-seam helpers whose per-backend emission (macOS `SecKey`,
//! Linux `EVP_PKEY`, Windows CNG, `getentropy`/`BCryptGenRandom`) lives in
//! `native/`, dispatched generically through `registry::os_helper` and their runtime
//! specs DERIVED by `registry::runtime_specs`.
//!
//! Source injection is the registry's ([`crate::codegen::registry::augment_project`]);
//! the `Sealed`/`KeyPair` record types are authored (with their `DOC` blocks) in
//! `package.mfb` and recognized generically via `add_source_types`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, Registry, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(crate) mod native;

const MODULE_INTRO: &str = r#"Cryptographic hashes, HMAC, KDFs, authenticated encryption, a secure RNG, public-key signatures, and constant-time comparison"#;
const MODULE_DESC: &str = r#"The `crypto` package provides cryptographic hashes, HMAC, key-derivation
functions, authenticated encryption (AEAD), a cryptographically-secure random
generator, public-key signatures, and constant-time comparison. It is a built-in
package, so `IMPORT crypto` needs no manifest dependency.

Inputs and outputs are `List OF Byte`; the hash/HMAC/PBKDF2 functions also accept
a `String` overload that UTF-8-encodes internally. A digest, ciphertext, or key
is raw binary — stringify it for display or storage with the `encoding` package
(`encoding::hexEncode`, `encoding::base64Encode`). The package defines two record
types, `crypto::Sealed` and `crypto::KeyPair`; see `mfb man crypto types`.

`crypto` is software-first: every hash, HMAC, KDF, AEAD, and Ed25519 primitive is
a portable core written in MFBASIC source over the `bits` package, so its output
is byte-identical on every target and uses no deprecated platform functions. Two
categories bind the platform instead: `randomBytes` draws from the OS CSPRNG
(`getentropy` / `BCryptGenRandom`), and the NIST-EC public-key operations bind the
platform's modern key API (`SecKey` on macOS, `EVP_PKEY` on Linux, CNG on
Windows), whose backends are wire-compatible across platforms.

AEAD `seal` returns a `crypto::Sealed` (ciphertext plus a 16-byte tag); `open`
verifies the tag in constant time and fails closed with `ErrAuthenticationFailed`,
returning plaintext only on success. `aad` defaults to empty. Nonces must be
unique per key — never reuse a `(key, nonce)` pair."#;

// Man-page + spec citation anchor: `CRYPTO`. The `crypto/*` man pages ground their
// fixed-return / argument-typing facts in this descriptor with
// `[[src/codegen/builtins/crypto/mod.rs:CRYPTO]]`.
//
// The former per-package `CryptoResolver` (`resolve_return_type` /
// `implementation_name`), `default_argument_padding`, `call_param_names`, and
// `expected_arguments` are all DERIVED generically by the registry now: return
// typing + overload selection are `RegistryFunction::select`, the AEAD `aad` default
// is the trailing `DefaultValue::Fill` parameter, and the `_bytes`/`_text` body
// choice is a distinct `Implementation` per argument type.

/// The native (runtime-helper) `crypto::` entry points: they lower to
/// `_mfb_rt_crypto_*` rather than to an injected `__crypto_*` source body, so
/// [`Body::rewrite_target`] returns `None` for them and `runtime::helper_for_call`
/// claims them as [`crate::target::shared::runtime::RuntimeHelper::Crypto`].
pub(crate) fn is_native_crypto_call(name: &str) -> bool {
    matches!(
        name,
        "crypto.randomBytes"
            | "crypto.generateP256Raw"
            | "crypto.generateP384Raw"
            | "crypto.generateP521Raw"
            | "crypto.p256Sign"
            | "crypto.p256Verify"
            | "crypto.p384Sign"
            | "crypto.p384Verify"
            | "crypto.p521Sign"
            | "crypto.p521Verify"
    )
}

/// The raw NIST key generators are **not user-callable**: they exist only for
/// `package.mfb`'s `__crypto_generateP*` glue, which slices the public point out of
/// the returned `0x04||X||Y||K` bytes and builds a `KeyPair`. Reached from injected
/// toolchain source, so they must stay resolvable there — the exclusion is applied in
/// the resolver, which knows whether the calling file is toolchain-provided
/// (`AstFile::internal`); `scripts/list_functions.py`'s `INTERNAL_CALLS` agrees
/// (bug-337-D9).
pub(crate) fn is_crypto_internal_call(name: &str) -> bool {
    matches!(
        name,
        "crypto.generateP256Raw" | "crypto.generateP384Raw" | "crypto.generateP521Raw"
    )
}

/// Register the `crypto` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("crypto", MODULE_INTRO, MODULE_DESC);

    // `package.mfb` carries its own `IMPORT crypto/bits/strings/collections/encoding`,
    // the `Sealed`/`KeyPair` `EXPORT TYPE` declarations (with `DOC` blocks), the module
    // round-constant/IV globals, the private `__crypto_*` helpers, and every public
    // member's `__crypto_*` body. Injected verbatim (byte-exact concatenation of the
    // former five `crypto_*.mfb` files), so the reassembled source is identical to the
    // legacy `package_source_glue!` `concat!` and public members rewrite onto it via
    // `Body::Rewrite`.
    pkg.add_helper_functions(vec![include_str!("package.mfb")]);

    // The public value RECORDS `Sealed`/`KeyPair` are authored (with their `DOC` blocks
    // and byte-exact fields) in `package.mfb`; recording their names as source-declared
    // types lets the generic `registry::is_builtin_type` / `qualified_builtin_type`
    // recognize them without double-declaring.
    pkg.add_source_types(&["Sealed", "KeyPair"]);

    // Hashes (source, `_bytes`/`_text` overloads).
    func_sha256::register(&mut pkg);
    func_sha224::register(&mut pkg);
    func_sha512::register(&mut pkg);
    func_sha384::register(&mut pkg);
    // HMAC (source, `_bytes`/`_text` overloads on `data`).
    func_hmac_sha256::register(&mut pkg);
    func_hmac_sha512::register(&mut pkg);
    // KDF (source).
    func_hkdf_sha256::register(&mut pkg);
    func_hkdf_sha512::register(&mut pkg);
    func_pbkdf2_sha256::register(&mut pkg);
    func_pbkdf2_sha512::register(&mut pkg);
    // AEAD (source; `aad` defaults to the empty byte list).
    func_aes256_gcm_seal::register(&mut pkg);
    func_aes256_gcm_open::register(&mut pkg);
    func_chacha20_poly1305_seal::register(&mut pkg);
    func_chacha20_poly1305_open::register(&mut pkg);
    // Secure random (`randomBytes` native; `randomInt`/`uuid4` source glue).
    func_random_bytes::register(&mut pkg);
    func_random_int::register(&mut pkg);
    func_uuid4::register(&mut pkg);
    // Public-key key generation (Ed25519 source; NIST-EC source glue over native raw).
    func_generate_ed25519::register(&mut pkg);
    func_generate_p256::register(&mut pkg);
    func_generate_p384::register(&mut pkg);
    func_generate_p521::register(&mut pkg);
    // Raw NIST keygen (native, internal-only).
    func_generate_p256_raw::register(&mut pkg);
    func_generate_p384_raw::register(&mut pkg);
    func_generate_p521_raw::register(&mut pkg);
    // Signatures (Ed25519 source; NIST-EC native).
    func_ed25519_sign::register(&mut pkg);
    func_ed25519_verify::register(&mut pkg);
    func_p256_sign::register(&mut pkg);
    func_p256_verify::register(&mut pkg);
    func_p384_sign::register(&mut pkg);
    func_p384_verify::register(&mut pkg);
    func_p521_sign::register(&mut pkg);
    func_p521_verify::register(&mut pkg);
    // Constant-time comparison (source).
    func_constant_time_equal::register(&mut pkg);

    r.add_package(pkg);
}

mod func_aes256_gcm_open;
mod func_aes256_gcm_seal;
mod func_chacha20_poly1305_open;
mod func_chacha20_poly1305_seal;
mod func_constant_time_equal;
mod func_ed25519_sign;
mod func_ed25519_verify;
mod func_generate_ed25519;
mod func_generate_p256;
mod func_generate_p256_raw;
mod func_generate_p384;
mod func_generate_p384_raw;
mod func_generate_p521;
mod func_generate_p521_raw;
mod func_hkdf_sha256;
mod func_hkdf_sha512;
mod func_hmac_sha256;
mod func_hmac_sha512;
mod func_p256_sign;
mod func_p256_verify;
mod func_p384_sign;
mod func_p384_verify;
mod func_p521_sign;
mod func_p521_verify;
mod func_pbkdf2_sha256;
mod func_pbkdf2_sha512;
mod func_random_bytes;
mod func_random_int;
mod func_sha224;
mod func_sha256;
mod func_sha384;
mod func_sha512;
mod func_uuid4;

/// `List OF Byte` — the pervasive `crypto` argument/return type.
fn bytes() -> ParameterType {
    ParameterType::list_of(ParameterType::Byte)
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::{self, registry};

    #[test]
    fn crypto_registered_on_the_clean_room_registry() {
        let pkg = registry()
            .resolve_package("crypto")
            .expect("crypto package");
        // 33 documented members (23 source + 10 native).
        assert_eq!(pkg.functions().len(), 33);
    }

    #[test]
    fn membership_via_generic_registry() {
        for n in [
            "crypto.sha256",
            "crypto.sha224",
            "crypto.sha512",
            "crypto.sha384",
            "crypto.hmacSha256",
            "crypto.hmacSha512",
            "crypto.hkdfSha256",
            "crypto.hkdfSha512",
            "crypto.pbkdf2Sha256",
            "crypto.pbkdf2Sha512",
            "crypto.aes256GcmSeal",
            "crypto.aes256GcmOpen",
            "crypto.chacha20Poly1305Seal",
            "crypto.chacha20Poly1305Open",
            "crypto.randomBytes",
            "crypto.randomInt",
            "crypto.uuid4",
            "crypto.generateEd25519",
            "crypto.generateP256",
            "crypto.generateP384",
            "crypto.generateP521",
            "crypto.generateP256Raw",
            "crypto.generateP384Raw",
            "crypto.generateP521Raw",
            "crypto.ed25519Sign",
            "crypto.ed25519Verify",
            "crypto.p256Sign",
            "crypto.p256Verify",
            "crypto.p384Sign",
            "crypto.p384Verify",
            "crypto.p521Sign",
            "crypto.p521Verify",
            "crypto.constantTimeEqual",
        ] {
            assert_eq!(registry().owning_package(n), Some("crypto"), "{n}");
        }
        assert!(registry().owning_package("crypto.nope").is_none());
    }

    #[test]
    fn native_and_internal_flags() {
        for f in [
            "crypto.randomBytes",
            "crypto.generateP256Raw",
            "crypto.generateP384Raw",
            "crypto.generateP521Raw",
            "crypto.p256Sign",
            "crypto.p256Verify",
            "crypto.p384Sign",
            "crypto.p384Verify",
            "crypto.p521Sign",
            "crypto.p521Verify",
        ] {
            assert!(super::is_native_crypto_call(f), "{f}");
        }
        assert!(!super::is_native_crypto_call("crypto.sha256"));
        assert!(!super::is_native_crypto_call("crypto.ed25519Sign"));
        for f in [
            "crypto.generateP256Raw",
            "crypto.generateP384Raw",
            "crypto.generateP521Raw",
        ] {
            assert!(super::is_crypto_internal_call(f), "{f}");
        }
        assert!(!super::is_crypto_internal_call("crypto.generateP256"));
    }

    #[test]
    fn builtin_types_recognized() {
        for t in ["Sealed", "KeyPair"] {
            assert!(registry().is_builtin_type(t), "{t}");
        }
        assert!(!registry().is_builtin_type("Nope"));
        assert_eq!(
            registry().qualified_builtin_type("crypto.Sealed"),
            Some("Sealed".to_string())
        );
    }

    #[test]
    fn bytes_text_overloads_select_distinct_rewrite_targets() {
        let sel = |call: &str, args: &[&str]| -> Option<&'static str> {
            let types: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            registry::rewrite_target(call, &types)
        };
        // Hash: bytes arg -> `_bytes`, String arg -> `_text`.
        assert_eq!(
            sel("crypto.sha256", &["List OF Byte"]),
            Some("__crypto_sha256_bytes")
        );
        assert_eq!(
            sel("crypto.sha256", &["String"]),
            Some("__crypto_sha256_text")
        );
        assert_eq!(
            sel("crypto.sha512", &["String"]),
            Some("__crypto_sha512_text")
        );
        // HMAC selects on `data` (arg index 1).
        assert_eq!(
            sel("crypto.hmacSha256", &["List OF Byte", "String"]),
            Some("__crypto_hmacSha256_text")
        );
        assert_eq!(
            sel("crypto.hmacSha512", &["List OF Byte", "List OF Byte"]),
            Some("__crypto_hmacSha512_bytes")
        );
        // PBKDF2 selects on `password` (arg index 0).
        assert_eq!(
            sel(
                "crypto.pbkdf2Sha256",
                &["String", "List OF Byte", "Integer", "Integer"]
            ),
            Some("__crypto_pbkdf2Sha256_text")
        );
        // Single-body source member.
        assert_eq!(sel("crypto.uuid4", &[]), Some("__crypto_uuid4"));
        assert_eq!(
            sel(
                "crypto.constantTimeEqual",
                &["List OF Byte", "List OF Byte"]
            ),
            Some("__crypto_constantTimeEqual")
        );
        // Native member -> no rewrite target.
        assert_eq!(sel("crypto.randomBytes", &["Integer"]), None);
        assert_eq!(
            sel("crypto.p256Sign", &["List OF Byte", "List OF Byte"]),
            None
        );
    }

    #[test]
    fn argument_typed_return_resolution() {
        let r = |call: &str, args: &[&str]| {
            let types: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            registry::resolve_call(call, &types, false)
        };
        assert_eq!(
            r("crypto.sha256", &["List OF Byte"]),
            Some("List OF Byte".into())
        );
        assert_eq!(r("crypto.sha256", &["String"]), Some("List OF Byte".into()));
        assert_eq!(r("crypto.sha256", &["Integer"]), None);
        assert_eq!(
            r(
                "crypto.aes256GcmSeal",
                &["List OF Byte", "List OF Byte", "List OF Byte"]
            ),
            Some("Sealed".into())
        );
        assert_eq!(
            r(
                "crypto.aes256GcmSeal",
                &[
                    "List OF Byte",
                    "List OF Byte",
                    "List OF Byte",
                    "List OF Byte"
                ]
            ),
            Some("Sealed".into())
        );
        assert_eq!(r("crypto.uuid4", &[]), Some("String".into()));
        assert_eq!(r("crypto.generateEd25519", &[]), Some("KeyPair".into()));
        assert_eq!(
            r("crypto.randomBytes", &["Integer"]),
            Some("List OF Byte".into())
        );
        assert_eq!(
            r("crypto.randomInt", &["Integer", "Integer"]),
            Some("Integer".into())
        );
        assert_eq!(
            r(
                "crypto.ed25519Verify",
                &["List OF Byte", "List OF Byte", "List OF Byte"]
            ),
            Some("Boolean".into())
        );
    }

    #[test]
    fn aead_aad_default_padding() {
        // AEAD `seal` pads one trailing `aad` when omitted (3 provided -> 1), none at 4.
        assert_eq!(
            registry::default_argument_padding("crypto.aes256GcmSeal", 3).len(),
            1
        );
        assert_eq!(
            registry::default_argument_padding("crypto.aes256GcmSeal", 4).len(),
            0
        );
        // `open` pads one trailing `aad` when omitted (4 provided -> 1), none at 5.
        assert_eq!(
            registry::default_argument_padding("crypto.chacha20Poly1305Open", 4).len(),
            1
        );
        assert_eq!(
            registry::default_argument_padding("crypto.chacha20Poly1305Open", 5).len(),
            0
        );
        assert_eq!(
            registry::default_argument_padding("crypto.sha256", 1).len(),
            0
        );
    }

    #[test]
    fn reassembled_source_parses() {
        let source = registry()
            .resolve_package("crypto")
            .expect("crypto")
            .get_mfb();
        crate::ast::parse_source_internal(
            std::path::Path::new("<builtin-crypto>"),
            "builtins/crypto.mfb",
            &source,
        )
        .expect("reassembled crypto source parses");
    }
}
