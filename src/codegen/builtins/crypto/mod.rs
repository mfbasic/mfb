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
//! `CryptoResolver` idiom the clean-room `select()` subsumes). The unified
//! public-key members — `generate`/`sign`/`verify` over the `Certificate` enum —
//! are clean-room [`Body::abi_function`] lowerings (`func_generate`/`func_sign`/
//! `func_verify` + the shared `gen_cert` seam) that bind the platform key APIs
//! (macOS `SecKey`, Linux `EVP_PKEY`, Windows CNG) self-contained; the unified
//! `hash`/`hmac`/`hkdf`/`pbkdf2`, `seal`/`open`, `convert`, and `encrypt`/`decrypt`
//! members dispatch (via `Body::abi_function`/`Body::Rewrite`) to the MFB software
//! cores. The one remaining OS-seam helper is the CSPRNG `randomBytes`
//! (`getentropy` / `BCryptGenRandom`), itself a clean-room [`Body::abi_function`]
//! lowering (`func_random_bytes`) routed through the shared `RuntimeHelper::Abi`
//! family — so every `crypto` member is a `Body::abi_function`/`Body::Rewrite`.
//!
//! Source injection is the registry's ([`crate::codegen::registry::augment_project`]);
//! the `Sealed`/`KeyPair` record types are registered via `add_record`, carrying
//! their `DOC` descriptions on the `RegistryRecord`/`RecordProp` `description` fields.

use crate::codegen::registry::{
    Body, DefaultValue, EnumAdvisory, EnumVariant, Implementation, Parameter, RecordProp, Registry,
    RegistryEnum, RegistryFunction, RegistryPackage, RegistryRecord,
};
use crate::types::ParameterType;

const MODULE_INTRO: &str = r#"Cryptographic hashes, HMAC, KDFs, authenticated encryption, secure random and time-ordered identifiers, public-key signatures, and constant-time comparison"#;
const MODULE_DESC: &str = r#"The `crypto` package provides cryptographic hashes, HMAC, key-derivation
functions, authenticated encryption (AEAD), a cryptographically-secure random
generator, UUID and ULID identifiers, public-key signatures, and constant-time
comparison. It is a built-in package, so `IMPORT crypto` needs no manifest
dependency.

Inputs and outputs are `List OF Byte`; the hash/HMAC/PBKDF2 functions also accept
a `String` overload that UTF-8-encodes internally. A digest, ciphertext, or key
is raw binary — stringify it for display or storage with the `encoding` package
(`encoding::hexEncode`, `encoding::base64Encode`). The package defines two record
types, `crypto::Sealed` and `crypto::KeyPair`; see `mfb man crypto types`.

`crypto` is software-first: every hash (SHA-1, SHA-2, SHA-3, and the SHAKE256
XOF), HMAC, KDF, AEAD, Ed25519/Ed448 signature, and X25519/X448 key-agreement
primitive is
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

/// Register the `crypto` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("crypto", MODULE_INTRO, MODULE_DESC);

    // Injected `IMPORT`s, rendered by `get_mfb` before the records/helpers —
    // mirroring `package.mfb`'s original leading `IMPORT` lines.
    pkg.add_imports(vec![
        "crypto",
        "bits",
        "strings",
        "collections",
        "encoding",
        "datetime",
    ]);

    // The public value RECORDS `Sealed`/`KeyPair` (with their `DOC` blocks and
    // byte-exact fields), formerly authored in `package.mfb`. Emitted as real
    // `add_record` records so the generic `registry::is_builtin_type` /
    // `qualified_builtin_type` recognizes them.
    pkg.add_record(RegistryRecord {
        name: "Sealed",
        export: true,
        description: "The result of an AEAD seal operation: the encrypted `ciphertext` bytes paired with the 16-byte authentication `tag` that binds them. The 12-byte nonce is NOT part of this record — store or transmit it separately (it is not secret) and pass it back to `crypto::open`.",
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
    // declaration order (P256=0, P384=1, P521=2, Ed25519=3, X25519=4, X448=5,
    // Ed448=6);
    // `func_generate`'s `AbiFunction` body branches on that ordinal. `X25519` and
    // `X448` are key-agreement (ECDH) keys, not signing keys, so `sign`/`verify`
    // reject them with `ErrInvalidArgument` and `exchange` accepts only them.
    pkg.add_enum(RegistryEnum {
        name: "Certificate",
        export: true,
        variants: vec![
            EnumVariant {
                name: "P256",
                description: "NIST P-256 (secp256r1) ECDSA key pair.",
                advisory: None,
            },
            EnumVariant {
                name: "P384",
                description: "NIST P-384 (secp384r1) ECDSA key pair.",
                advisory: None,
            },
            EnumVariant {
                name: "P521",
                description: "NIST P-521 (secp521r1) ECDSA key pair.",
                advisory: None,
            },
            EnumVariant {
                name: "Ed25519",
                description: "Ed25519 EdDSA key pair.",
                advisory: None,
            },
            EnumVariant {
                name: "X25519",
                description: "X25519 (Curve25519 ECDH, RFC 7748) key-agreement key pair (32-byte keys) — not a signing key; use it with `crypto::exchange`. `crypto::encrypt`/`crypto::decrypt` take Ed25519 (or Ed448) keys, converted internally.",
                advisory: None,
            },
            // Curve448 (plan-109-C): appended after X25519 so the earlier ordinals stay
            // fixed (X448=5).
            EnumVariant {
                name: "X448",
                description: "X448 (Curve448 ECDH, RFC 7748) key-agreement key pair (56-byte keys) — not a signing key; use it with `crypto::exchange`. `crypto::KeyConvert.Ed448ToX448` derives one from an Ed448 pair; the `Ed448_*` suites of `crypto::encrypt`/`crypto::decrypt` do that conversion internally.",
                advisory: None,
            },
            // Ed448 (plan-109-D): appended after X448 (Ed448=6).
            EnumVariant {
                name: "Ed448",
                description: "Ed448 EdDSA key pair (RFC 8032 PureEd448 over edwards448 with SHAKE256): 57-byte seed and public key, 114-byte deterministic signatures.",
                advisory: None,
            },
        ],
    });
    // The key-conversion selector for `crypto::convert(conv, keys)`. Ordinals are
    // declaration order (Ed25519ToX25519=0, Ed448ToX448=1); the pure-MFB
    // `__crypto_convert` core branches on it.
    pkg.add_enum(RegistryEnum {
        name: "KeyConvert",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Ed25519ToX25519",
                description: "Convert an Ed25519 signing key pair to the matching X25519 (Curve25519 ECDH) key pair.",
                advisory: None,
            },
            EnumVariant {
                name: "Ed448ToX448",
                description: "Convert an Ed448 signing key pair (57-byte seed and public key) to the matching X448 (Curve448 ECDH) key pair (56-byte keys), by the RFC 7748 §4.2 edwards448→curve448 map for the public key and `SHAKE256(seed)[0..56]` for the private key.",
                advisory: None,
            },
        ],
    });
    // The hash-algorithm selector — every hash function `crypto` supports (SHA-1,
    // the SHA-2 family, the SHA-3 family). Ordinals are declaration order (SHA1=0,
    // SHA2_224=1, SHA2_256=2, SHA2_384=3, SHA2_512=4, SHA3_224=5, SHA3_256=6,
    // SHA3_384=7, SHA3_512=8); `gen_hash`'s `ORD_*` constants and the
    // `__crypto_sha{Digest,BlockSize,OutputLen}` dispatch helpers mirror this order.
    // The SHA-2 spellings carry the family prefix (`SHA2_256`, never `SHA256`) so
    // the SHA-3 widths added by plan-109-B read unambiguously beside them; the
    // old bare spellings were removed without aliases (plan-109-A).
    // `SHA1` carries the registry's enum-value advisory: every user-source use
    // reports `CRYPTO_SHA1_INSECURE` (warn, non-fatal) — the digest itself is the
    // standard FIPS 180-4 value and stays usable for legacy interoperability.
    pkg.add_enum(RegistryEnum {
        name: "Hash",
        export: true,
        variants: vec![
            EnumVariant {
                name: "SHA1",
                description: "SHA-1 (FIPS 180-4, 160-bit digest). Not collision-resistant: every source use reports the `CRYPTO_SHA1_INSECURE` warning; select it only for legacy interoperability, never for new designs.",
                advisory: Some(EnumAdvisory {
                    rule: "CRYPTO_SHA1_INSECURE",
                    detail: "`crypto::Hash.SHA1` selects SHA-1, which is not collision-resistant (practical collisions since 2017). Keep it only for legacy interoperability; use `crypto::Hash.SHA2_256` or stronger for new designs.",
                }),
            },
            EnumVariant {
                name: "SHA2_224",
                description: "SHA-224 (SHA-2 family, FIPS 180-4, 224-bit digest).",
                advisory: None,
            },
            EnumVariant {
                name: "SHA2_256",
                description: "SHA-256 (SHA-2 family, FIPS 180-4, 256-bit digest).",
                advisory: None,
            },
            EnumVariant {
                name: "SHA2_384",
                description: "SHA-384 (SHA-2 family, FIPS 180-4, 384-bit digest).",
                advisory: None,
            },
            EnumVariant {
                name: "SHA2_512",
                description: "SHA-512 (SHA-2 family, FIPS 180-4, 512-bit digest).",
                advisory: None,
            },
            // SHA-3 (plan-109-B): appended after the SHA-2 family so the earlier
            // ordinals stay fixed (SHA3_224=5, SHA3_256=6, SHA3_384=7, SHA3_512=8).
            EnumVariant {
                name: "SHA3_224",
                description: "SHA3-224 (SHA-3 family, FIPS 202, 224-bit digest; Keccak-f[1600] at rate 1152).",
                advisory: None,
            },
            EnumVariant {
                name: "SHA3_256",
                description: "SHA3-256 (SHA-3 family, FIPS 202, 256-bit digest; Keccak-f[1600] at rate 1088).",
                advisory: None,
            },
            EnumVariant {
                name: "SHA3_384",
                description: "SHA3-384 (SHA-3 family, FIPS 202, 384-bit digest; Keccak-f[1600] at rate 832).",
                advisory: None,
            },
            EnumVariant {
                name: "SHA3_512",
                description: "SHA3-512 (SHA-3 family, FIPS 202, 512-bit digest; Keccak-f[1600] at rate 576).",
                advisory: None,
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
                advisory: None,
            },
            EnumVariant {
                name: "CHACHA20POLY1305",
                description: "ChaCha20-Poly1305 AEAD (RFC 8439).",
                advisory: None,
            },
        ],
    });
    // The asymmetric-cipher suite selector for `crypto::encrypt`/`crypto::decrypt`:
    // an RFC 9180 HPKE ciphersuite (base mode). Ordinals are declaration order
    // (Ed25519_AES256GCM=0, Ed25519_CHACHA20POLY1305=1, Ed448_AES256GCM=2,
    // Ed448_CHACHA20POLY1305=3); the pure-MFB `__crypto_hpke*` profile helpers read
    // each value's KEM/KDF/AEAD ids by explicit property (never ordinal
    // arithmetic). The `Ed25519_*` suites take Ed25519 recipient keys (converted to
    // X25519), the `Ed448_*` suites Ed448 keys (converted to X448). No P* (NIST-EC
    // ECDH) variants — EC-ECDH isn't built.
    pkg.add_enum(RegistryEnum {
        name: "AsymmetricCipher",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Ed25519_AES256GCM",
                description: "RFC 9180 HPKE base mode: DHKEM(X25519, HKDF-SHA256) + HKDF-SHA256 + AES-256-GCM, over Ed25519 recipient keys (converted to X25519). Wire value `enc(32) ‖ ct`.",
                advisory: None,
            },
            EnumVariant {
                name: "Ed25519_CHACHA20POLY1305",
                description: "RFC 9180 HPKE base mode: DHKEM(X25519, HKDF-SHA256) + HKDF-SHA256 + ChaCha20Poly1305, over Ed25519 recipient keys (converted to X25519). Wire value `enc(32) ‖ ct`.",
                advisory: None,
            },
            // X448 suites (plan-109-F): appended so the X25519 ordinals stay fixed.
            EnumVariant {
                name: "Ed448_AES256GCM",
                description: "RFC 9180 HPKE base mode: DHKEM(X448, HKDF-SHA512) + HKDF-SHA512 + AES-256-GCM, over Ed448 recipient keys (converted to X448). Wire value `enc(56) ‖ ct`.",
                advisory: None,
            },
            EnumVariant {
                name: "Ed448_CHACHA20POLY1305",
                description: "RFC 9180 HPKE base mode: DHKEM(X448, HKDF-SHA512) + HKDF-SHA512 + ChaCha20Poly1305, over Ed448 recipient keys (converted to X448). Wire value `enc(56) ‖ ct`.",
                advisory: None,
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
    // SHA-1 (FIPS 180-4 §6.1) over the same padded 512-bit blocks and masked 32-bit
    // arithmetic as SHA-256; only the 80-word schedule, round function, and
    // constants differ.
    helper_rotl32::register(&mut pkg);
    helper_sha1_f::register(&mut pkg);
    helper_sha1_k::register(&mut pkg);
    helper_sha1_schedule::register(&mut pkg);
    helper_sha1_bytes::register(&mut pkg);
    helper_sha1_text::register(&mut pkg);
    helper_add64::register(&mut pkg);
    // Keccak-f[1600] / SHA-3 / SHAKE256 (FIPS 202, plan-109-B). One full 64-bit
    // lane per Integer (a raw `bits::` pattern, like SHA-512's words — Keccak is
    // XOR/AND/NOT/rotate only, so nothing enters trapping arithmetic); the
    // permutation and sponge are branch-free apart from the public lengths.
    helper_keccak_rc::register(&mut pkg);
    helper_keccak_rc_table::register(&mut pkg);
    helper_keccak_rho::register(&mut pkg);
    helper_keccak_rho_table::register(&mut pkg);
    helper_keccak_pi::register(&mut pkg);
    helper_keccak_pi_table::register(&mut pkg);
    helper_keccak_zero::register(&mut pkg);
    helper_keccak_round::register(&mut pkg);
    helper_keccak_f::register(&mut pkg);
    helper_le_lane::register(&mut pkg);
    helper_append_le_lane::register(&mut pkg);
    helper_keccak_sponge::register(&mut pkg);
    helper_sha3_224_bytes::register(&mut pkg);
    helper_sha3_224_text::register(&mut pkg);
    helper_sha3_256_bytes::register(&mut pkg);
    helper_sha3_256_text::register(&mut pkg);
    helper_sha3_384_bytes::register(&mut pkg);
    helper_sha3_384_text::register(&mut pkg);
    helper_sha3_512_bytes::register(&mut pkg);
    helper_sha3_512_text::register(&mut pkg);
    helper_shake256::register(&mut pkg);
    helper_shake256_text::register(&mut pkg);
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
    helper_hkdf_expand::register(&mut pkg);
    helper_be32::register(&mut pkg);
    helper_xor_bytes::register(&mut pkg);
    helper_pbkdf2_block::register(&mut pkg);
    // Hash-generic keyed-hash dispatch + constructions (work over ALL `Hash`
    // variants; a future variant is one new arm in each of the three `sha*`
    // dispatch helpers). See `func_hmac`/`func_hkdf`/`func_pbkdf2`.
    helper_sha_digest::register(&mut pkg);
    helper_sha_block_size::register(&mut pkg);
    helper_sha_output_len::register(&mut pkg);
    helper_hmac::register(&mut pkg);
    helper_hmac_text::register(&mut pkg);
    helper_hkdf::register(&mut pkg);
    helper_pbkdf2::register(&mut pkg);
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
    // X25519 (Curve25519 ECDH, RFC 7748) + Ed25519↔X25519 conversion, all pure
    // software over the shared GF(2^255-19) field ops (`__crypto_edA`/`edZ`/`edS`/
    // `edM`/`inv25519`/`unpack25519`/`pack25519`). `__crypto_generateX25519` backs the
    // `generate(Certificate.X25519)` ordinal branch; `__crypto_convert` backs the
    // `convert(KeyConvert, KeyPair)` member.
    helper_gf121665::register(&mut pkg);
    helper_x25519::register(&mut pkg);
    helper_generate_x25519::register(&mut pkg);
    helper_ed25519_pub_to_x25519::register(&mut pkg);
    helper_ed25519_priv_to_x25519::register(&mut pkg);
    // X448 (Curve448 ECDH, RFC 7748) + Ed448→X448 conversion (plan-109-C): a
    // 16 × 28-bit-limb GF(2^448-2^224-1) field (`__crypto_gf448*`), the 448-step
    // ladder with a branch-free select swap, generation, and the RFC 7748 §4.2
    // isogeny / SHAKE256 conversion. `__crypto_exchange` is the public X25519/X448
    // Diffie-Hellman core.
    helper_gf448_zero::register(&mut pkg);
    helper_gf448_one::register(&mut pkg);
    helper_gf448_carry::register(&mut pkg);
    helper_gf448_add::register(&mut pkg);
    helper_gf448_sub::register(&mut pkg);
    helper_gf448_mul::register(&mut pkg);
    helper_gf448_mul_small::register(&mut pkg);
    helper_gf448_inv::register(&mut pkg);
    helper_gf448_select::register(&mut pkg);
    helper_gf448_unpack::register(&mut pkg);
    helper_gf448_pack::register(&mut pkg);
    helper_clamp_scalar448::register(&mut pkg);
    helper_x448::register(&mut pkg);
    helper_x448_base::register(&mut pkg);
    helper_generate_x448::register(&mut pkg);
    helper_ed448_pub_to_x448::register(&mut pkg);
    helper_ed448_priv_to_x448::register(&mut pkg);
    helper_is_all_zero::register(&mut pkg);
    helper_exchange::register(&mut pkg);
    helper_convert::register(&mut pkg);
    // Ed448 (RFC 8032 §5.2 PureEd448, plan-109-D) over the same GF(2^448-2^224-1)
    // field: byte-limb scalar arithmetic with a fold-based `mod L`, projective
    // unified addition, a select-swap ladder, strict decoding, and the
    // SHAKE256/dom4 key/sign/verify flow.
    helper_zero_limbs::register(&mut pkg);
    helper_pad_limbs::register(&mut pkg);
    helper_bytes_to_limbs::register(&mut pkg);
    helper_bn_mul::register(&mut pkg);
    helper_bn_add::register(&mut pkg);
    helper_ed448_point3::register(&mut pkg);
    helper_ed448_tables::register(&mut pkg);
    helper_ed448_fold::register(&mut pkg);
    helper_ed448_mod_l::register(&mut pkg);
    helper_ed448_scalar_below_l::register(&mut pkg);
    helper_ed448_add::register(&mut pkg);
    helper_ed448_cswap::register(&mut pkg);
    helper_ed448_scalarmult::register(&mut pkg);
    helper_ed448_encode::register(&mut pkg);
    helper_gf448_pow_p34::register(&mut pkg);
    helper_ed448_decode::register(&mut pkg);
    helper_ed448_dom::register(&mut pkg);
    helper_ed448_public::register(&mut pkg);
    helper_generate_ed448::register(&mut pkg);
    helper_ed448_sign::register(&mut pkg);
    helper_ed448_verify::register(&mut pkg);
    // Asymmetric public-key encryption: RFC 9180 HPKE single-shot base mode
    // (plan-109-E, X448 suites plan-109-F). Pure software over the X25519/X448
    // ladders, the Ed25519→X25519 / Ed448→X448 conversion helpers, the hash-generic
    // HMAC/HKDF-expand cores, and the unified `seal`/`open`:
    // `__crypto_hpkeLabeledExtract`/`Expand` are the RFC labeled KDF,
    // `__crypto_hpke{IsX448,IsAesGcm,KemId,…,SuiteId}` the per-suite profile properties,
    // `__crypto_hpke{Dh,Base,RecipientPub,RecipientPriv,ExtractAndExpand}` the DHKEM
    // layer, `__crypto_hpkeKeySchedule` the base key schedule, and
    // `__crypto_hpkeSealWith` the deterministic seal seam `__crypto_encrypt` feeds a
    // fresh ephemeral key. `__crypto_encrypt`/`__crypto_decrypt` back the
    // `encrypt`/`decrypt` members; `__crypto_encryptText` is the String shim.
    helper_hpke_i2osp2::register(&mut pkg);
    helper_hpke_labeled_extract::register(&mut pkg);
    helper_hpke_labeled_expand::register(&mut pkg);
    helper_hpke_profile::register(&mut pkg);
    helper_hpke_kem::register(&mut pkg);
    helper_hpke_key_schedule::register(&mut pkg);
    helper_hpke_seal_with::register(&mut pkg);
    helper_encrypt::register(&mut pkg);
    helper_decrypt::register(&mut pkg);
    helper_encrypt_text::register(&mut pkg);

    // The unified clean-room `hash(Hash, data)` selects a SHA-2 digest by the `Hash`
    // ordinal and branch-links to the always-emitted MFB software SHA cores (the SHA
    // math stays in MFB), mirroring `generate`/`sign`/`verify` over `Certificate`. It
    // is the sole hashing surface — the per-digest `sha*` members were removed.
    func_hash::register(&mut pkg);
    // Unified hash-generic keyed-hash members: one `Hash`-selected surface each,
    // working over ALL `Hash` variants via the `__crypto_sha*` dispatch. They are the
    // sole HMAC/HKDF/PBKDF2 surface — the per-digest `hmacSha*`/`hkdfSha*`/`pbkdf2Sha*`
    // members were removed.
    func_hmac::register(&mut pkg);
    func_hkdf::register(&mut pkg);
    func_pbkdf2::register(&mut pkg);
    // The SHAKE256 extendable-output function (FIPS 202 §6.2): variable output
    // length, so it is its own member rather than a fixed-digest `Hash` selector.
    // Pure-MFB rewrite onto `__crypto_shake256` (also the Ed448 hash).
    func_shake256::register(&mut pkg);
    // The unified clean-room AEAD `seal`/`open` select a symmetric cipher by the
    // `SymmetricCipher` ordinal and branch-link to the always-emitted MFB software AEAD
    // cores (the AEAD math stays in MFB), mirroring `hash` over `Hash`. `seal` carries a
    // `List OF Byte` and a `String` `data` overload; `open` an explicit
    // `ciphertext`/`tag` and a `crypto::Sealed` overload. `aad` fills to the empty list.
    func_seal::register(&mut pkg);
    func_open::register(&mut pkg);
    // Secure random (`randomBytes` native; identifier generators are source glue).
    func_random_bytes::register(&mut pkg);
    func_random_int::register(&mut pkg);
    func_uuid4::register(&mut pkg);
    func_uuid7::register(&mut pkg);
    func_ulid::register(&mut pkg);
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
    // Key-pair conversion between curve encodings (`Ed25519ToX25519`,
    // `Ed448ToX448`). Pure-MFB rewrite onto `__crypto_convert` — no platform
    // library, so (like `hmac`/`hkdf`) it is NOT in any backend's `runtime_calls`.
    func_convert::register(&mut pkg);
    // X25519 / X448 Diffie-Hellman over the key-agreement `Certificate`s. Pure-MFB
    // rewrite onto `__crypto_exchange`.
    func_exchange::register(&mut pkg);
    // Asymmetric public-key encryption. `encrypt`/`decrypt` are RFC 9180 HPKE
    // base-mode `Seal`/`Open` over Ed25519 or Ed448 recipient keys, selected by
    // `AsymmetricCipher`. Pure-MFB rewrites onto `__crypto_encrypt`/`__crypto_decrypt`
    // — no platform library, so (like `hmac`/`hkdf`/`convert`) they are NOT in any
    // backend's `runtime_calls`.
    func_encrypt::register(&mut pkg);
    func_decrypt::register(&mut pkg);
    // Constant-time comparison (source).
    func_constant_time_equal::register(&mut pkg);

    r.add_package(pkg);
}

mod func_constant_time_equal;
mod func_convert;
mod func_decrypt;
mod func_encrypt;
mod func_exchange;
pub(crate) mod func_generate;
pub(crate) mod func_hash;
pub(crate) mod func_hkdf;
pub(crate) mod func_hmac;
pub(crate) mod func_open;
pub(crate) mod func_pbkdf2;
mod func_random_bytes;
mod func_random_int;
pub(crate) mod func_seal;
mod func_shake256;
pub(crate) mod func_sign;
mod func_ulid;
mod func_uuid4;
mod func_uuid7;
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
mod helper_append_le_lane;
mod helper_append_le_word;
mod helper_be32;
mod helper_be64;
mod helper_be_word;
mod helper_be_word64;
mod helper_be_words;
mod helper_be_words64;
mod helper_bn_add;
mod helper_bn_mul;
mod helper_bsig0;
mod helper_bsig0_64;
mod helper_bsig1;
mod helper_bsig1_64;
mod helper_bytes_to_limbs;
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
mod helper_clamp_scalar448;
mod helper_concat;
mod helper_concat_int;
mod helper_constant_time_equal;
mod helper_convert;
mod helper_copy_bytes;
mod helper_cswap128;
mod helper_decrypt;
mod helper_ed25519_priv_to_x25519;
mod helper_ed25519_pub_to_x25519;
mod helper_ed25519_public;
mod helper_ed25519_sign;
mod helper_ed25519_verify;
mod helper_ed448_add;
mod helper_ed448_cswap;
mod helper_ed448_decode;
mod helper_ed448_dom;
mod helper_ed448_encode;
mod helper_ed448_fold;
mod helper_ed448_mod_l;
mod helper_ed448_point3;
mod helper_ed448_priv_to_x448;
mod helper_ed448_pub_to_x448;
mod helper_ed448_public;
mod helper_ed448_scalar_below_l;
mod helper_ed448_scalarmult;
mod helper_ed448_sign;
mod helper_ed448_tables;
mod helper_ed448_verify;
mod helper_ed_a;
mod helper_ed_add;
mod helper_ed_l;
mod helper_ed_m;
mod helper_ed_s;
mod helper_ed_z;
mod helper_encrypt;
mod helper_encrypt_text;
mod helper_exchange;
mod helper_first64;
mod helper_gcm_gctr;
mod helper_gcm_ghash_data;
mod helper_gcm_inc32;
mod helper_gcm_j0;
mod helper_gcm_tag;
mod helper_generate_ed25519;
mod helper_generate_ed448;
mod helper_generate_x25519;
mod helper_generate_x448;
mod helper_gf0;
mod helper_gf1;
mod helper_gf121665;
mod helper_gf448_add;
mod helper_gf448_carry;
mod helper_gf448_inv;
mod helper_gf448_mul;
mod helper_gf448_mul_small;
mod helper_gf448_one;
mod helper_gf448_pack;
mod helper_gf448_pow_p34;
mod helper_gf448_select;
mod helper_gf448_sub;
mod helper_gf448_unpack;
mod helper_gf448_zero;
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
mod helper_hkdf;
mod helper_hkdf_expand;
mod helper_hmac;
mod helper_hmac_text;
mod helper_hpke_i2osp2;
mod helper_hpke_kem;
mod helper_hpke_key_schedule;
mod helper_hpke_labeled_expand;
mod helper_hpke_labeled_extract;
mod helper_hpke_profile;
mod helper_hpke_seal_with;
mod helper_inv25519;
mod helper_is_all_zero;
mod helper_iv224;
mod helper_iv256;
mod helper_iv384;
mod helper_iv512;
mod helper_k256;
mod helper_k512;
mod helper_keccak_f;
mod helper_keccak_pi;
mod helper_keccak_pi_table;
mod helper_keccak_rc;
mod helper_keccak_rc_table;
mod helper_keccak_rho;
mod helper_keccak_rho_table;
mod helper_keccak_round;
mod helper_keccak_sponge;
mod helper_keccak_zero;
mod helper_last64;
mod helper_le64;
mod helper_le_lane;
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
mod helper_pad_limbs;
mod helper_par25519;
mod helper_pbkdf2;
mod helper_pbkdf2_block;
mod helper_point4;
mod helper_poly1305;
mod helper_poly_finish;
mod helper_poly_r;
mod helper_pow2523;
mod helper_rand62;
mod helper_rand63;
mod helper_random_int;
mod helper_reduce;
mod helper_rotl32;
mod helper_rotr32;
mod helper_scalar_below_l;
mod helper_scalarbase;
mod helper_scalarmult;
mod helper_seal_text;
mod helper_sha1_bytes;
mod helper_sha1_f;
mod helper_sha1_k;
mod helper_sha1_schedule;
mod helper_sha1_text;
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
mod helper_sha3_224_bytes;
mod helper_sha3_224_text;
mod helper_sha3_256_bytes;
mod helper_sha3_256_text;
mod helper_sha3_384_bytes;
mod helper_sha3_384_text;
mod helper_sha3_512_bytes;
mod helper_sha3_512_text;
mod helper_sha512_bytes;
mod helper_sha512_iv;
mod helper_sha512_ktable;
mod helper_sha512_schedule;
mod helper_sha512_text;
mod helper_sha_block_size;
mod helper_sha_digest;
mod helper_sha_output_len;
mod helper_shake256;
mod helper_shake256_text;
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
mod helper_x25519;
mod helper_x448;
mod helper_x448_base;
mod helper_xor_bytes;
mod helper_xor_pad;
mod helper_xtime;
mod helper_zero_limbs;
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
        // 20 members: the unified `generate`/`sign`/`verify`/`hash`, the
        // `SymmetricCipher`-selected `seal`/`open`, the hash-generic
        // `hmac`/`hkdf`/`pbkdf2(Hash, …)`, the SHAKE256 XOF `shake256`, `convert`
        // (Ed25519→X25519 / Ed448→X448 key conversion), `exchange` (X25519/X448
        // Diffie-Hellman), the `AsymmetricCipher`-selected `encrypt`/`decrypt`
        // (RFC 9180 HPKE), plus
        // `randomBytes`/`randomInt`/`uuid4`/`uuid7`/`ulid`/`constantTimeEqual`. The
        // per-type generate/sign/verify/sha and per-digest `*Sha*`/per-cipher AEAD
        // members were all retired behind the unified surface.
        assert_eq!(pkg.functions().len(), 20);
    }

    #[test]
    fn membership_via_generic_registry() {
        for n in [
            "crypto.hash",
            "crypto.hmac",
            "crypto.hkdf",
            "crypto.pbkdf2",
            "crypto.shake256",
            "crypto.seal",
            "crypto.open",
            "crypto.randomBytes",
            "crypto.randomInt",
            "crypto.uuid4",
            "crypto.uuid7",
            "crypto.ulid",
            "crypto.generate",
            "crypto.sign",
            "crypto.verify",
            "crypto.convert",
            "crypto.exchange",
            "crypto.encrypt",
            "crypto.decrypt",
            "crypto.constantTimeEqual",
        ] {
            assert_eq!(registry().owning_package(n), Some("crypto"), "{n}");
        }
        // The per-digest/per-curve members were folded into `hash`/`sign`/`verify`;
        // the per-digest KDF/MAC and per-cipher AEAD members were retired behind the
        // `Hash`-generic `hmac`/`hkdf`/`pbkdf2` and `SymmetricCipher`-selected
        // `seal`/`open`.
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
        ] {
            assert!(registry().owning_package(gone).is_none(), "{gone}");
        }
        assert!(registry().owning_package("crypto.nope").is_none());
    }

    #[test]
    fn native_and_internal_flags() {
        use crate::codegen::registry::is_abi_function_call;
        // The OS-seam CSPRNG `randomBytes` is a clean-room `AbiFunction` (like
        // `generate`/`sign`/`verify`/`hash`/`seal`), so every `crypto` member is an
        // `AbiFunction`/`Rewrite` — it routes through the shared `RuntimeHelper::Abi` family.
        assert!(is_abi_function_call("crypto.randomBytes"));
        for f in [
            "crypto.generate",
            "crypto.sign",
            "crypto.verify",
            "crypto.hash",
        ] {
            assert!(is_abi_function_call(f), "{f}");
        }
        // The `Body::Rewrite` source members are NOT runtime/abi calls.
        for f in [
            "crypto.hmac",
            "crypto.convert",
            "crypto.exchange",
            "crypto.constantTimeEqual",
        ] {
            assert!(!is_abi_function_call(f), "{f}");
        }
    }

    #[test]
    fn builtin_types_recognized() {
        for t in ["crypto.Sealed", "crypto.KeyPair"] {
            assert!(registry().is_builtin_type(t), "{t}");
        }
        assert!(!registry().is_builtin_type("Nope"));
        assert_eq!(
            registry().qualified_builtin_type("crypto.Sealed"),
            Some("crypto.Sealed".to_string())
        );
    }

    #[test]
    fn bytes_text_overloads_select_distinct_rewrite_targets() {
        let sel = |call: &str, args: &[&str]| -> Option<&'static str> {
            let types: Vec<crate::types::ParameterType> = args
                .iter()
                .map(|s| crate::types::ParameterType::declared(s))
                .collect();
            registry::rewrite_target(call, &types)
        };
        // The unified `hash(Hash, data)`: the `List OF Byte` overload is a native
        // AbiFunction (no source rewrite), the `String` overload rewrites to the
        // `__crypto_hashText` UTF-8 shim.
        assert_eq!(sel("crypto.hash", &["crypto.Hash", "List OF Byte"]), None);
        assert_eq!(
            sel("crypto.hash", &["crypto.Hash", "String"]),
            Some("__crypto_hashText")
        );
        // The unified `hmac(Hash, key, data)` selects on `data` (arg index 2): the
        // `String` form rewrites to the `__crypto_hmacText` UTF-8 shim, the
        // `List OF Byte` form to the hash-generic `__crypto_hmac` core.
        assert_eq!(
            sel("crypto.hmac", &["crypto.Hash", "List OF Byte", "String"]),
            Some("__crypto_hmacText")
        );
        assert_eq!(
            sel(
                "crypto.hmac",
                &["crypto.Hash", "List OF Byte", "List OF Byte"]
            ),
            Some("__crypto_hmac")
        );
        // The unified `pbkdf2(Hash, password, …)` has a single `List OF Byte`
        // overload rewriting to the hash-generic `__crypto_pbkdf2` core.
        assert_eq!(
            sel(
                "crypto.pbkdf2",
                &[
                    "crypto.Hash",
                    "List OF Byte",
                    "List OF Byte",
                    "Integer",
                    "Integer"
                ]
            ),
            Some("__crypto_pbkdf2")
        );
        // `shake256(data, length)` selects on `data`: the `String` form rewrites to
        // the `__crypto_shake256Text` UTF-8 shim, the `List OF Byte` form to the
        // XOF core.
        assert_eq!(
            sel("crypto.shake256", &["List OF Byte", "Integer"]),
            Some("__crypto_shake256")
        );
        assert_eq!(
            sel("crypto.shake256", &["String", "Integer"]),
            Some("__crypto_shake256Text")
        );
        // Single-body source member.
        assert_eq!(sel("crypto.uuid4", &[]), Some("__crypto_uuid4"));
        assert_eq!(sel("crypto.uuid7", &[]), Some("__crypto_uuid7"));
        assert_eq!(sel("crypto.ulid", &[]), Some("__crypto_ulid"));
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
                &["crypto.Certificate", "List OF Byte", "List OF Byte"]
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
            r("crypto.hash", &["crypto.Hash", "List OF Byte"]),
            Some("List OF Byte".into())
        );
        assert_eq!(
            r("crypto.hash", &["crypto.Hash", "String"]),
            Some("List OF Byte".into())
        );
        assert_eq!(r("crypto.hash", &["crypto.Hash", "Integer"]), None);
        assert_eq!(
            r(
                "crypto.seal",
                &[
                    "crypto.SymmetricCipher",
                    "List OF Byte",
                    "List OF Byte",
                    "List OF Byte"
                ]
            ),
            Some("crypto.Sealed".into())
        );
        assert_eq!(
            r(
                "crypto.seal",
                &[
                    "crypto.SymmetricCipher",
                    "List OF Byte",
                    "List OF Byte",
                    "List OF Byte",
                    "List OF Byte"
                ]
            ),
            Some("crypto.Sealed".into())
        );
        assert_eq!(r("crypto.uuid4", &[]), Some("String".into()));
        assert_eq!(r("crypto.uuid7", &[]), Some("String".into()));
        assert_eq!(r("crypto.ulid", &[]), Some("String".into()));
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
                    "crypto.Certificate",
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
        // AEAD `seal(cipher, key, nonce, data, [aad])` pads one trailing `aad` when
        // omitted (4 provided -> 1), none at 5.
        assert_eq!(
            registry::default_argument_padding("crypto.seal", 4, None).len(),
            1
        );
        assert_eq!(
            registry::default_argument_padding("crypto.seal", 5, None).len(),
            0
        );
        // `open(cipher, key, nonce, ciphertext, tag, [aad])` pads one trailing `aad`
        // when omitted (5 provided -> 1), none at 6.
        assert_eq!(
            registry::default_argument_padding("crypto.open", 5, None).len(),
            1
        );
        assert_eq!(
            registry::default_argument_padding("crypto.open", 6, None).len(),
            0
        );
        assert_eq!(
            registry::default_argument_padding("crypto.hash", 2, None).len(),
            0
        );
    }

    /// Structural constant-time audit of the Keccak core (plan-109-B): the
    /// permutation and its round must contain no conditional or early exit at
    /// all, and the sponge's only conditional may test the PUBLIC output length.
    /// Every `collections::get` index in the round is a loop counter or a public
    /// table lookup — this pins that no arm branches on, or indexes by, state or
    /// message contents.
    #[test]
    fn keccak_core_is_branch_free() {
        let source = registry()
            .resolve_package("crypto")
            .expect("crypto")
            .get_mfb();
        let body_of = |name: &str| -> String {
            let start = source
                .find(&format!("FUNC {name}("))
                .unwrap_or_else(|| panic!("{name} not in assembled source"));
            let end = source[start..].find("END FUNC").expect("END FUNC") + start;
            source[start..end].to_string()
        };
        for name in [
            "__crypto_keccakRound",
            "__crypto_keccakF",
            "__crypto_leLane",
            "__crypto_appendLeLane",
        ] {
            let body = body_of(name);
            assert!(!body.contains("IF "), "{name} must be branch-free:\n{body}");
            assert!(
                !body.contains("EXIT ") && !body.contains("TRAP"),
                "{name} must have no early exit:\n{body}"
            );
        }
        let sponge = body_of("__crypto_keccakSponge");
        let conditionals: Vec<&str> = sponge.lines().filter(|l| l.contains("IF ")).collect();
        assert_eq!(conditionals.len(), 1, "{conditionals:?}");
        assert!(
            conditionals[0].contains("len(out) < outLen"),
            "the sponge's only branch is on the public output length: {conditionals:?}"
        );
        // Indices into the state within the round: a nested lookup whose inner
        // subject is the state (`get(a, get(a, …))`) would be a data-dependent
        // address; the only nested lookups are the public RHO/PI tables.
        let round = body_of("__crypto_keccakRound");
        for state in ["a", "b", "t", "c", "out"] {
            for inner in ["a", "b", "t", "c", "out"] {
                assert!(
                    !round.contains(&format!(
                        "collections::get({state}, collections::get({inner}"
                    )),
                    "{state} indexed by {inner} contents"
                );
            }
        }
        assert!(round.contains("collections::get(__CRYPTO_KECCAK_RHO, i)"));
        assert!(round.contains("collections::get(__CRYPTO_KECCAK_PI, i)"));
    }

    /// Structural constant-time audit of the Curve448 secret paths (plan-109-C/D):
    /// the X448 ladder, the Ed448 ladder/swap/addition, and the scalar reduction
    /// contain no conditional at all, and the field multiply/carry branch only on
    /// loop counters (`IF n …`, `IF j …`, `IF i …` — fold weights and bias limbs).
    #[test]
    fn curve448_secret_paths_are_branch_free() {
        let source = registry()
            .resolve_package("crypto")
            .expect("crypto")
            .get_mfb();
        let body_of = |name: &str| -> String {
            let start = source
                .find(&format!("FUNC {name}("))
                .unwrap_or_else(|| panic!("{name} not in assembled source"));
            let end = source[start..].find("END FUNC").expect("END FUNC") + start;
            source[start..end].to_string()
        };
        for name in [
            "__crypto_x448",
            "__crypto_gf448Select",
            "__crypto_gf448Add",
            "__crypto_gf448MulSmall",
            "__crypto_gf448Carry",
            "__crypto_ed448Scalarmult",
            "__crypto_ed448Cswap",
            "__crypto_ed448Add",
            "__crypto_ed448ModL",
            "__crypto_ed448Fold",
            "__crypto_bnMul",
            "__crypto_ed448Prune",
            "__crypto_clampScalar448",
        ] {
            let body = body_of(name);
            assert!(!body.contains("IF "), "{name} must be branch-free:\n{body}");
        }
        // The multiply, subtraction bias, and inverse/sqrt ladders branch only on
        // loop counters — every `IF` tests `n`, `j`, or `i`.
        for name in [
            "__crypto_gf448Mul",
            "__crypto_gf448Sub",
            "__crypto_gf448Inv",
            "__crypto_gf448PowP34",
            "__crypto_gf448Pack",
        ] {
            let body = body_of(name);
            for line in body.lines().filter(|l| l.trim_start().starts_with("IF ")) {
                let cond = line.trim_start().trim_start_matches("IF ");
                assert!(
                    cond.starts_with("n ") || cond.starts_with("j ") || cond.starts_with("i "),
                    "{name} branches on a non-counter: {line}"
                );
            }
        }
    }

    /// Executable bound proof for the GF(2^448−2^224−1) arithmetic (plan-109-C
    /// Phase 1): with every limb at its carried maximum 2^28 (`__crypto_gf448Carry`
    /// leaves limbs in `0..=2^28` after two passes), the schoolbook convolution
    /// columns and the folded output limbs of `__crypto_gf448Mul` — mirroring the
    /// helper's exact fold weights — and the biased sums of `Add`/`Sub`/`MulSmall`
    /// must all stay below the trapping `Integer` boundary 2^63.
    #[test]
    fn gf448_mul_accumulators_fit_i63() {
        let limb: u128 = 1 << 28;
        let product = limb * limb;
        // Convolution column k has min(k,15) - max(0,k-15) + 1 products.
        let column = |k: usize| -> u128 {
            let lo = k.saturating_sub(15);
            let hi = k.min(15);
            (hi - lo + 1) as u128 * product
        };
        let mut worst = 0u128;
        for n in 0..16 {
            let mut v = column(n);
            if n <= 14 {
                v += column(n + 16);
            }
            if n >= 8 {
                v += column(n + 8);
                if n <= 14 {
                    v += column(n + 16);
                }
            }
            if n <= 6 {
                v += column(n + 24);
            }
            worst = worst.max(v);
            assert!(v < 1u128 << 63, "output limb {n} accumulator {v} >= 2^63");
        }
        // The worst column (limb 8: 9 + 2·7 + 15 = 38 products) is ~2^61.25.
        assert_eq!(worst, 38 * product);
        assert!(worst < 1u128 << 62);
        // Carry-pass input: a limb plus the previous carry (< 2^35) stays < 2^63.
        assert!(worst + (1u128 << 35) < 1u128 << 63);
        // Add: two carried limbs; Sub: a − b + 2p_i with the biases the helper uses;
        // MulSmall: limb × 39081 (the largest constant, a24 / the Edwards d).
        assert!(2 * limb < 1u128 << 63);
        assert!(limb + 536_870_910 < 1u128 << 63);
        assert_eq!(536_870_910, 2 * ((1u128 << 28) - 1));
        assert_eq!(536_870_908, 2 * ((1u128 << 28) - 2));
        assert!(limb * 39081 < 1u128 << 63);
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
