//! Built-in `crypto::` package — migrated onto the clean-room registry
//! (`crate::codegen::registry`), mirroring `datetime`/`csv`/`json`.
//!
//! `crypto` is HETEROGENEOUS. Its symmetric, hashing, AEAD, KDF, Ed25519, and
//! secure-random glue are portable **software cores** written in MFBASIC over the
//! `bits` package; those `__crypto_*` bodies live in per-helper `helper_*.rs` files
//! (each a `RegistryHelper` registered via `add_helper`, split from the former
//! `package.mfb` blob) and every public source member rewrites onto its
//! `__crypto_*` body via [`Body::Rewrite`]. The hash/HMAC/PBKDF2 members
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
//! the `Sealed`/`KeyPair` record types are registered via `add_record`, carrying
//! their `DOC` descriptions on the `RegistryRecord`/`RecordProp` `description` fields.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RecordProp, Registry, RegistryFunction,
    RegistryPackage, RegistryRecord,
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
            | "crypto.generateP256"
            | "crypto.generateP384"
            | "crypto.generateP521"
            | "crypto.p256Sign"
            | "crypto.p256Verify"
            | "crypto.p384Sign"
            | "crypto.p384Verify"
            | "crypto.p521Sign"
            | "crypto.p521Verify"
    )
}

/// Register the `crypto` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("crypto", MODULE_INTRO, MODULE_DESC);

    // Injected `IMPORT`s, rendered by `get_mfb` before the records/helpers —
    // mirroring `package.mfb`'s original leading `IMPORT` lines.
    pkg.add_imports(vec!["crypto", "bits", "strings", "collections", "encoding"]);

    // The public value RECORDS `Sealed`/`KeyPair` (with their `DOC` blocks and
    // byte-exact fields), formerly authored in `package.mfb`. Emitted as real
    // `add_record` records so the generic `registry::is_builtin_type` /
    // `qualified_builtin_type` recognizes them.
    pkg.add_record(RegistryRecord {
        name: "Sealed",
        export: true,
        description: "The result of an AEAD seal operation: the encrypted `ciphertext` bytes paired with the authentication `tag` that binds them.",
        props: vec![
            RecordProp { name: "ciphertext", ty: ParameterType::list_of(ParameterType::Byte), description: "The encrypted message bytes." },
            RecordProp { name: "tag", ty: ParameterType::list_of(ParameterType::Byte), description: "The authentication tag produced by the AEAD cipher, verified on open." },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: "KeyPair",
        export: true,
        description: "A public-key key pair, holding the secret `privateKey` used to sign or derive and the matching `publicKey` shared with counterparties.",
        props: vec![
            RecordProp { name: "privateKey", ty: ParameterType::list_of(ParameterType::Byte), description: "The secret key bytes; keep confidential." },
            RecordProp { name: "publicKey", ty: ParameterType::list_of(ParameterType::Byte), description: "The public key bytes; safe to share." },
        ],
    });

    // The shared private `__crypto_*` helpers (module globals + every `__crypto_*`
    // body). Each lives in its own `helper_*.rs` and registers via `add_helper`;
    // order preserved from the old `package.mfb` blob so the compiled `.ncode`
    // stays byte-identical.
    helper_k256::register(&mut pkg);
    helper_iv256::register(&mut pkg);
    helper_iv224::register(&mut pkg);
    helper_k512::register(&mut pkg);
    helper_iv512::register(&mut pkg);
    helper_iv384::register(&mut pkg);
    helper_aes_sbox::register(&mut pkg);
    helper_m32::register(&mut pkg);
    helper_add32::register(&mut pkg);
    helper_rotr32::register(&mut pkg);
    helper_shr32::register(&mut pkg);
    helper_not32::register(&mut pkg);
    helper_be_word::register(&mut pkg);
    helper_append_be_word::register(&mut pkg);
    helper_copy_bytes::register(&mut pkg);
    helper_sha256_ktable::register(&mut pkg);
    helper_be_words::register(&mut pkg);
    helper_sha256_iv::register(&mut pkg);
    helper_sha224_iv::register(&mut pkg);
    helper_ch32::register(&mut pkg);
    helper_maj32::register(&mut pkg);
    helper_bsig0::register(&mut pkg);
    helper_bsig1::register(&mut pkg);
    helper_ssig0::register(&mut pkg);
    helper_ssig1::register(&mut pkg);
    helper_pad512::register(&mut pkg);
    helper_sha256_schedule::register(&mut pkg);
    helper_sha2_32::register(&mut pkg);
    helper_truncate::register(&mut pkg);
    helper_sha256_bytes::register(&mut pkg);
    helper_sha256_text::register(&mut pkg);
    helper_sha224_bytes::register(&mut pkg);
    helper_sha224_text::register(&mut pkg);
    helper_add64::register(&mut pkg);
    helper_be_word64::register(&mut pkg);
    helper_be_words64::register(&mut pkg);
    helper_append_be_word64::register(&mut pkg);
    helper_ch64::register(&mut pkg);
    helper_maj64::register(&mut pkg);
    helper_bsig0_64::register(&mut pkg);
    helper_bsig1_64::register(&mut pkg);
    helper_ssig0_64::register(&mut pkg);
    helper_ssig1_64::register(&mut pkg);
    helper_sha512_ktable::register(&mut pkg);
    helper_sha512_iv::register(&mut pkg);
    helper_sha384_iv::register(&mut pkg);
    helper_pad1024::register(&mut pkg);
    helper_sha512_schedule::register(&mut pkg);
    helper_sha2_64::register(&mut pkg);
    helper_sha512_bytes::register(&mut pkg);
    helper_sha512_text::register(&mut pkg);
    helper_sha384_bytes::register(&mut pkg);
    helper_sha384_text::register(&mut pkg);
    helper_xor_pad::register(&mut pkg);
    helper_zero_pad::register(&mut pkg);
    helper_concat::register(&mut pkg);
    helper_hmac_sha256_bytes::register(&mut pkg);
    helper_hmac_sha256_text::register(&mut pkg);
    helper_hmac_sha512_bytes::register(&mut pkg);
    helper_hmac_sha512_text::register(&mut pkg);
    helper_hkdf_sha256::register(&mut pkg);
    helper_hkdf_expand::register(&mut pkg);
    helper_hkdf_sha512::register(&mut pkg);
    helper_be32::register(&mut pkg);
    helper_xor_bytes::register(&mut pkg);
    helper_pbkdf2_block::register(&mut pkg);
    helper_pbkdf2_sha256_bytes::register(&mut pkg);
    helper_pbkdf2_sha256_text::register(&mut pkg);
    helper_pbkdf2_sha512_bytes::register(&mut pkg);
    helper_pbkdf2_sha512_text::register(&mut pkg);
    helper_le_word::register(&mut pkg);
    helper_append_le_word::register(&mut pkg);
    helper_chacha_qr::register(&mut pkg);
    helper_chacha_state::register(&mut pkg);
    helper_chacha_block::register(&mut pkg);
    helper_chacha20::register(&mut pkg);
    helper_poly_r::register(&mut pkg);
    helper_poly1305::register(&mut pkg);
    helper_poly_finish::register(&mut pkg);
    helper_pad16::register(&mut pkg);
    helper_le64::register(&mut pkg);
    helper_aead_mac_data::register(&mut pkg);
    helper_chacha20_poly1305_seal::register(&mut pkg);
    helper_chacha20_poly1305_open::register(&mut pkg);
    helper_aes_sbox_table::register(&mut pkg);
    helper_aes_sub::register(&mut pkg);
    helper_xtime::register(&mut pkg);
    helper_gmul8::register(&mut pkg);
    helper_aes_expand_key::register(&mut pkg);
    helper_aes_add_round_key::register(&mut pkg);
    helper_aes_sub_bytes::register(&mut pkg);
    helper_aes_shift_rows::register(&mut pkg);
    helper_aes_mix_columns::register(&mut pkg);
    helper_aes_encrypt_block::register(&mut pkg);
    helper_ghash_mul::register(&mut pkg);
    helper_ghash::register(&mut pkg);
    helper_gcm_j0::register(&mut pkg);
    helper_gcm_inc32::register(&mut pkg);
    helper_gcm_gctr::register(&mut pkg);
    helper_gcm_ghash_data::register(&mut pkg);
    helper_be64::register(&mut pkg);
    helper_gcm_tag::register(&mut pkg);
    helper_aes256_gcm_seal::register(&mut pkg);
    helper_aes256_gcm_open::register(&mut pkg);
    helper_constant_time_equal::register(&mut pkg);
    helper_rand62::register(&mut pkg);
    helper_rand63::register(&mut pkg);
    helper_random_int::register(&mut pkg);
    helper_uuid4::register(&mut pkg);
    helper_gf0::register(&mut pkg);
    helper_gf1::register(&mut pkg);
    helper_gf_d::register(&mut pkg);
    helper_gf_d2::register(&mut pkg);
    helper_gf_x::register(&mut pkg);
    helper_gf_y::register(&mut pkg);
    helper_gf_i::register(&mut pkg);
    helper_ed_l::register(&mut pkg);
    helper_ed_a::register(&mut pkg);
    helper_ed_z::register(&mut pkg);
    helper_car25519::register(&mut pkg);
    helper_ed_m::register(&mut pkg);
    helper_ed_s::register(&mut pkg);
    helper_inv25519::register(&mut pkg);
    helper_pow2523::register(&mut pkg);
    helper_pack25519::register(&mut pkg);
    helper_unpack25519::register(&mut pkg);
    helper_par25519::register(&mut pkg);
    helper_neq25519::register(&mut pkg);
    helper_concat_int::register(&mut pkg);
    helper_gf_at::register(&mut pkg);
    helper_point4::register(&mut pkg);
    helper_first64::register(&mut pkg);
    helper_last64::register(&mut pkg);
    helper_ed_add::register(&mut pkg);
    helper_cswap128::register(&mut pkg);
    helper_scalarmult::register(&mut pkg);
    helper_scalarbase::register(&mut pkg);
    helper_pack_point::register(&mut pkg);
    helper_unpackneg::register(&mut pkg);
    helper_mod_l::register(&mut pkg);
    helper_reduce::register(&mut pkg);
    helper_slice::register(&mut pkg);
    helper_clamp_scalar::register(&mut pkg);
    helper_ed25519_public::register(&mut pkg);
    helper_generate_ed25519::register(&mut pkg);
    helper_ed25519_sign::register(&mut pkg);
    helper_scalar_below_l::register(&mut pkg);
    helper_ed25519_verify::register(&mut pkg);

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
    // Public-key key generation. Ed25519 source; each NIST-EC `generateP*` is a
    // single NATIVE member that builds the `KeyPair` record itself (its raw twin +
    // source glue were collapsed in).
    func_generate_ed25519::register(&mut pkg);
    func_generate_p256::register(&mut pkg);
    func_generate_p384::register(&mut pkg);
    func_generate_p521::register(&mut pkg);
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
mod func_generate_p384;
mod func_generate_p521;
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

mod helper_add32;
mod helper_add64;
mod helper_aead_mac_data;
mod helper_aes256_gcm_open;
mod helper_aes256_gcm_seal;
mod helper_aes_add_round_key;
mod helper_aes_encrypt_block;
mod helper_aes_expand_key;
mod helper_aes_mix_columns;
mod helper_aes_sbox;
mod helper_aes_sbox_table;
mod helper_aes_shift_rows;
mod helper_aes_sub;
mod helper_aes_sub_bytes;
mod helper_append_be_word;
mod helper_append_be_word64;
mod helper_append_le_word;
mod helper_be32;
mod helper_be64;
mod helper_be_word;
mod helper_be_word64;
mod helper_be_words;
mod helper_be_words64;
mod helper_bsig0;
mod helper_bsig0_64;
mod helper_bsig1;
mod helper_bsig1_64;
mod helper_car25519;
mod helper_ch32;
mod helper_ch64;
mod helper_chacha20;
mod helper_chacha20_poly1305_open;
mod helper_chacha20_poly1305_seal;
mod helper_chacha_block;
mod helper_chacha_qr;
mod helper_chacha_state;
mod helper_clamp_scalar;
mod helper_concat;
mod helper_concat_int;
mod helper_constant_time_equal;
mod helper_copy_bytes;
mod helper_cswap128;
mod helper_ed25519_public;
mod helper_ed25519_sign;
mod helper_ed25519_verify;
mod helper_ed_a;
mod helper_ed_add;
mod helper_ed_l;
mod helper_ed_m;
mod helper_ed_s;
mod helper_ed_z;
mod helper_first64;
mod helper_gcm_gctr;
mod helper_gcm_ghash_data;
mod helper_gcm_inc32;
mod helper_gcm_j0;
mod helper_gcm_tag;
mod helper_generate_ed25519;
mod helper_gf0;
mod helper_gf1;
mod helper_gf_at;
mod helper_gf_d;
mod helper_gf_d2;
mod helper_gf_i;
mod helper_gf_x;
mod helper_gf_y;
mod helper_ghash;
mod helper_ghash_mul;
mod helper_gmul8;
mod helper_hkdf_expand;
mod helper_hkdf_sha256;
mod helper_hkdf_sha512;
mod helper_hmac_sha256_bytes;
mod helper_hmac_sha256_text;
mod helper_hmac_sha512_bytes;
mod helper_hmac_sha512_text;
mod helper_inv25519;
mod helper_iv224;
mod helper_iv256;
mod helper_iv384;
mod helper_iv512;
mod helper_k256;
mod helper_k512;
mod helper_last64;
mod helper_le64;
mod helper_le_word;
mod helper_m32;
mod helper_maj32;
mod helper_maj64;
mod helper_mod_l;
mod helper_neq25519;
mod helper_not32;
mod helper_pack25519;
mod helper_pack_point;
mod helper_pad1024;
mod helper_pad16;
mod helper_pad512;
mod helper_par25519;
mod helper_pbkdf2_block;
mod helper_pbkdf2_sha256_bytes;
mod helper_pbkdf2_sha256_text;
mod helper_pbkdf2_sha512_bytes;
mod helper_pbkdf2_sha512_text;
mod helper_point4;
mod helper_poly1305;
mod helper_poly_finish;
mod helper_poly_r;
mod helper_pow2523;
mod helper_rand62;
mod helper_rand63;
mod helper_random_int;
mod helper_reduce;
mod helper_rotr32;
mod helper_scalar_below_l;
mod helper_scalarbase;
mod helper_scalarmult;
mod helper_sha224_bytes;
mod helper_sha224_iv;
mod helper_sha224_text;
mod helper_sha256_bytes;
mod helper_sha256_iv;
mod helper_sha256_ktable;
mod helper_sha256_schedule;
mod helper_sha256_text;
mod helper_sha2_32;
mod helper_sha2_64;
mod helper_sha384_bytes;
mod helper_sha384_iv;
mod helper_sha384_text;
mod helper_sha512_bytes;
mod helper_sha512_iv;
mod helper_sha512_ktable;
mod helper_sha512_schedule;
mod helper_sha512_text;
mod helper_shr32;
mod helper_slice;
mod helper_ssig0;
mod helper_ssig0_64;
mod helper_ssig1;
mod helper_ssig1_64;
mod helper_truncate;
mod helper_unpack25519;
mod helper_unpackneg;
mod helper_uuid4;
mod helper_xor_bytes;
mod helper_xor_pad;
mod helper_xtime;
mod helper_zero_pad;

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
        // 30 members: each generateP*Raw was collapsed into its native generateP*.
        assert_eq!(pkg.functions().len(), 30);
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
            "crypto.generateP256",
            "crypto.generateP384",
            "crypto.generateP521",
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
