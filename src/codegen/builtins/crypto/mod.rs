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
    Body, DefaultValue, EnumVariant, Implementation, Parameter, RecordProp, Registry, RegistryEnum,
    RegistryFunction, RegistryPackage, RegistryRecord,
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

/// The native (runtime-helper) `crypto::` entry point: it lowers to
/// `_mfb_rt_crypto_*` rather than to an injected `__crypto_*` source body, so
/// [`Body::rewrite_target`] returns `None` for it and `runtime::helper_for_call`
/// claims it as [`crate::target::shared::runtime::RuntimeHelper::Crypto`]. Since the
/// per-curve `p*Sign`/`p*Verify` members were folded into the clean-room
/// `crypto::sign`/`verify` AbiFunctions, `crypto.randomBytes` (the OS CSPRNG) is the
/// only native crypto call left.
pub(crate) fn is_native_crypto_call(name: &str) -> bool {
    name == "crypto.randomBytes"
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
    // The certificate/key type selector for `crypto::generate(type)`. Ordinals are
    // declaration order (P256=0, P384=1, P521=2, Ed25519=3); `func_generate`'s
    // `AbiFunction` body branches on that ordinal.
    pkg.add_enum(RegistryEnum {
        name: "Certificate",
        export: true,
        variants: vec![
            EnumVariant {
                name: "P256",
                description: "NIST P-256 (secp256r1) ECDSA key pair.",
            },
            EnumVariant {
                name: "P384",
                description: "NIST P-384 (secp384r1) ECDSA key pair.",
            },
            EnumVariant {
                name: "P521",
                description: "NIST P-521 (secp521r1) ECDSA key pair.",
            },
            EnumVariant {
                name: "Ed25519",
                description: "Ed25519 EdDSA key pair.",
            },
        ],
    });
    // The hash-algorithm selector — every hash function `crypto` supports (the
    // SHA-2 family). Ordinals are declaration order (SHA224=0, SHA256=1, SHA384=2,
    // SHA512=3).
    pkg.add_enum(RegistryEnum {
        name: "Hash",
        export: true,
        variants: vec![
            EnumVariant {
                name: "SHA224",
                description: "SHA-224 (SHA-2 family, 224-bit digest).",
            },
            EnumVariant {
                name: "SHA256",
                description: "SHA-256 (SHA-2 family, 256-bit digest).",
            },
            EnumVariant {
                name: "SHA384",
                description: "SHA-384 (SHA-2 family, 384-bit digest).",
            },
            EnumVariant {
                name: "SHA512",
                description: "SHA-512 (SHA-2 family, 512-bit digest).",
            },
        ],
    });
    // The symmetric-AEAD cipher selector for `crypto::seal`/`crypto::open`. Ordinals are
    // declaration order (AES256GCM=0, CHACHA20POLY1305=1); `func_seal`/`func_open`'s
    // `AbiFunction` bodies branch on that ordinal.
    pkg.add_enum(RegistryEnum {
        name: "SymmetricCipher",
        export: true,
        variants: vec![
            EnumVariant {
                name: "AES256GCM",
                description: "AES-256 in Galois/Counter Mode (NIST SP 800-38D).",
            },
            EnumVariant {
                name: "CHACHA20POLY1305",
                description: "ChaCha20-Poly1305 AEAD (RFC 8439).",
            },
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
    // The `String`-overload shim for the unified `hash(Hash, data)` member: UTF-8-encodes
    // then re-enters the `List OF Byte` `hash` AbiFunction (see `func_hash`).
    helper_hash_text::register(&mut pkg);
    // The overload shims for the unified AEAD `seal`/`open` members: `__crypto_sealText`
    // UTF-8-encodes a `String` `data` and re-enters the `List OF Byte` `seal`; and
    // `__crypto_openSealed` unpacks a `crypto::Sealed` and re-enters the five-argument
    // `open` (see `func_seal`/`func_open`).
    helper_seal_text::register(&mut pkg);
    helper_open_sealed::register(&mut pkg);

    // The unified clean-room `hash(Hash, data)` selects a SHA-2 digest by the `Hash`
    // ordinal and branch-links to the always-emitted MFB software SHA cores (the SHA
    // math stays in MFB), mirroring `generate`/`sign`/`verify` over `Certificate`. It
    // is the sole hashing surface — the per-digest `sha*` members were removed.
    func_hash::register(&mut pkg);
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
    // The unified clean-room AEAD `seal`/`open` select a symmetric cipher by the
    // `SymmetricCipher` ordinal and branch-link to the always-emitted MFB software AEAD
    // cores (the AEAD math stays in MFB), mirroring `hash` over `Hash`. `seal` carries a
    // `List OF Byte` and a `String` `data` overload; `open` an explicit
    // `ciphertext`/`tag` and a `crypto::Sealed` overload. `aad` fills to the empty list.
    func_seal::register(&mut pkg);
    func_open::register(&mut pkg);
    // Secure random (`randomBytes` native; `randomInt`/`uuid4` source glue).
    func_random_bytes::register(&mut pkg);
    func_random_int::register(&mut pkg);
    func_uuid4::register(&mut pkg);
    // Public-key key generation. The unified clean-room `generate(Certificate)`
    // covers every curve (NIST-EC via CNG/SecKey/OpenSSL, Ed25519 via the software
    // helper); the per-type `generateP*`/`generateEd25519` members it replaced were
    // removed.
    func_generate::register(&mut pkg);
    // The unified clean-room `sign(Certificate, privateKey, message)` covers every
    // curve (NIST-EC via CNG/SecKey/OpenSSL, Ed25519 via the software helper),
    // mirroring `generate`. It is the sole signing surface — the per-curve
    // `p*Sign`/`ed25519Sign` members were removed.
    func_sign::register(&mut pkg);
    // The unified clean-room `verify(Certificate, publicKey, message, signature)` covers
    // every curve (NIST-EC via CNG/SecKey/OpenSSL, Ed25519 via the software helper),
    // mirroring `sign`. It is the sole verification surface — the per-curve
    // `p*Verify`/`ed25519Verify` members were removed.
    func_verify::register(&mut pkg);
    // Constant-time comparison (source).
    func_constant_time_equal::register(&mut pkg);

    r.add_package(pkg);
}

mod func_aes256_gcm_open;
mod func_aes256_gcm_seal;
mod func_chacha20_poly1305_open;
mod func_chacha20_poly1305_seal;
mod func_constant_time_equal;
pub(crate) mod func_generate;
pub(crate) mod func_hash;
mod func_hkdf_sha256;
mod func_hkdf_sha512;
mod func_hmac_sha256;
mod func_hmac_sha512;
pub(crate) mod func_open;
mod func_pbkdf2_sha256;
mod func_pbkdf2_sha512;
mod func_random_bytes;
mod func_random_int;
pub(crate) mod func_seal;
pub(crate) mod func_sign;
mod func_uuid4;
pub(crate) mod func_verify;
pub(crate) mod gen_cert;
pub(crate) mod gen_cipher;
pub(crate) mod gen_hash;

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
mod helper_hash_text;
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
mod helper_open_sealed;
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
mod helper_seal_text;
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
        // 20 members: the unified clean-room `generate(Certificate)` replaced the
        // four per-type `generateP*`/`generateEd25519` members, the unified
        // `sign(Certificate, …)` / `verify(Certificate, …)` replaced the eight per-curve
        // signers/verifiers, the unified `hash(Hash, …)` replaced the four per-digest
        // `sha*` members, and the unified `seal`/`open` over `SymmetricCipher` were added
        // (each a single member with two overloads).
        assert_eq!(pkg.functions().len(), 20);
    }

    #[test]
    fn membership_via_generic_registry() {
        for n in [
            "crypto.hash",
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
            "crypto.seal",
            "crypto.open",
            "crypto.randomBytes",
            "crypto.randomInt",
            "crypto.uuid4",
            "crypto.generate",
            "crypto.sign",
            "crypto.verify",
            "crypto.constantTimeEqual",
        ] {
            assert_eq!(registry().owning_package(n), Some("crypto"), "{n}");
        }
        // The per-digest/per-curve members were folded into `hash`/`sign`/`verify`.
        for gone in [
            "crypto.sha224",
            "crypto.sha256",
            "crypto.sha384",
            "crypto.sha512",
            "crypto.p256Sign",
            "crypto.p256Verify",
            "crypto.p384Sign",
            "crypto.p384Verify",
            "crypto.p521Sign",
            "crypto.p521Verify",
            "crypto.ed25519Sign",
            "crypto.ed25519Verify",
        ] {
            assert!(registry().owning_package(gone).is_none(), "{gone}");
        }
        assert!(registry().owning_package("crypto.nope").is_none());
    }

    #[test]
    fn native_and_internal_flags() {
        // `randomBytes` (the OS CSPRNG) is the only native crypto call left; the
        // per-curve signers/verifiers were folded into the `sign`/`verify` AbiFunctions.
        assert!(super::is_native_crypto_call("crypto.randomBytes"));
        for f in [
            "crypto.sha256",
            "crypto.hash",
            "crypto.sign",
            "crypto.verify",
            "crypto.p256Sign",
            "crypto.p256Verify",
            "crypto.ed25519Sign",
        ] {
            assert!(!super::is_native_crypto_call(f), "{f}");
        }
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
        // The unified `hash(Hash, data)`: the `List OF Byte` overload is a native
        // AbiFunction (no source rewrite), the `String` overload rewrites to the
        // `__crypto_hashText` UTF-8 shim.
        assert_eq!(sel("crypto.hash", &["Hash", "List OF Byte"]), None);
        assert_eq!(
            sel("crypto.hash", &["Hash", "String"]),
            Some("__crypto_hashText")
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
        // Native / AbiFunction member -> no rewrite target.
        assert_eq!(sel("crypto.randomBytes", &["Integer"]), None);
        assert_eq!(
            sel(
                "crypto.sign",
                &["Certificate", "List OF Byte", "List OF Byte"]
            ),
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
            r("crypto.hash", &["Hash", "List OF Byte"]),
            Some("List OF Byte".into())
        );
        assert_eq!(
            r("crypto.hash", &["Hash", "String"]),
            Some("List OF Byte".into())
        );
        assert_eq!(r("crypto.hash", &["Hash", "Integer"]), None);
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
                "crypto.verify",
                &[
                    "Certificate",
                    "List OF Byte",
                    "List OF Byte",
                    "List OF Byte"
                ]
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
            registry::default_argument_padding("crypto.hash", 2).len(),
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
