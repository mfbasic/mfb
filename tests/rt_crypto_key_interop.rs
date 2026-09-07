//! Interop proof for `crypto::generate`, `crypto::sign`, `crypto::verify`,
//! `crypto::exchange` and `crypto::convert`: keys and signatures cross the
//! boundary in BOTH directions between MFBASIC and implementations that share no
//! code with it (`ed25519-dalek`, `curve25519-dalek`, `ring`).
//!
//! Key generation is random, so unlike the AEAD and MAC/KDF checks this cannot
//! be a byte comparison of the whole operation. What it can do — and what a
//! self-contained round trip inside one implementation cannot — is establish
//! that the two sides agree about the *encodings*:
//!
//!   * a key pair MFB generated is usable by the foreign implementation, and
//!     its public half really is the one derived from its private half;
//!   * a signature MFB produced verifies over there;
//!   * a key pair and signature produced over there verify HERE;
//!   * an ECDH secret computed from each side's private key and the other's
//!     public key is the same secret.
//!
//! Ed25519 additionally *is* deterministic (RFC 8032), so its signature is
//! compared byte for byte as well — a round trip would accept a signer that
//! chose a nonce differently and still verified.
//!
//! Every crate used here is already in the lockfile, so this costs no new
//! compiled code and runs on every `cargo test` — the best of the three oracle
//! homes in `.ai/testing-gates.md`.
//!
//! NOT covered here, and why: Ed448, X448 and P-521 have no implementation in
//! the lockfile to check them against, so they live in
//! `tools/oracles/crypto/keys/` where a pinned third-party crate is allowed.
//! `crypto::encrypt`/`decrypt` are covered by `rt_crypto_hpke_interop`.

mod common;
use common::{build_project, run_capture_with_env, temp_project};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use ring::rand::SystemRandom;
use ring::signature::{
    EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_ASN1,
    ECDSA_P256_SHA256_ASN1_SIGNING, ECDSA_P384_SHA384_ASN1, ECDSA_P384_SHA384_ASN1_SIGNING,
    ED25519,
};
use sha2::{Digest, Sha512};

/// A tiny RPC the test drives over one environment variable: `op,arg,arg,…`
/// records separated by `;`, byte arguments in hex with "-" for empty. One
/// output line per record, so responses pair with requests positionally.
const SOURCE: &str = r#"
IMPORT collections
IMPORT crypto
IMPORT encoding
IMPORT io
IMPORT os
IMPORT strings

FUNC decodeField(value AS String) AS List OF Byte
  MUT out AS List OF Byte = []
  IF value <> "-" THEN
    out = encoding::hexDecode(value)
  END IF
  RETURN out
END FUNC

FUNC hexField(value AS List OF Byte) AS String
  LET encoded AS String = encoding::hexEncode(value)
  IF encoded = "" THEN
    RETURN "-"
  END IF
  RETURN encoded
END FUNC

FUNC certOf(label AS String) AS crypto::Certificate
  IF label = "ed25519" THEN
    RETURN crypto::Certificate.Ed25519
  END IF
  IF label = "ed448" THEN
    RETURN crypto::Certificate.Ed448
  END IF
  IF label = "x25519" THEN
    RETURN crypto::Certificate.X25519
  END IF
  IF label = "x448" THEN
    RETURN crypto::Certificate.X448
  END IF
  IF label = "p256" THEN
    RETURN crypto::Certificate.P256
  END IF
  IF label = "p384" THEN
    RETURN crypto::Certificate.P384
  END IF
  RETURN crypto::Certificate.P521
END FUNC

SUB doGen(label AS String)
  LET k AS crypto::KeyPair = crypto::generate(certOf(label)) TRAP(e)
    io::print("genfail " & label & " " & toString(e.code))
    EXIT SUB
  END TRAP
  io::print("gen " & label & " " & hexField(k.privateKey) & " " & hexField(k.publicKey))
END SUB

SUB doSign(label AS String, privHex AS String, msgHex AS String)
  LET sig AS List OF Byte = crypto::sign(certOf(label), decodeField(privHex), decodeField(msgHex)) TRAP(e)
    io::print("signfail " & label & " " & toString(e.code))
    EXIT SUB
  END TRAP
  io::print("sign " & label & " " & hexField(sig))
END SUB

SUB doVerify(label AS String, pubHex AS String, msgHex AS String, sigHex AS String)
  LET ok AS Boolean = crypto::verify(certOf(label), decodeField(pubHex), decodeField(msgHex), decodeField(sigHex)) TRAP(e)
    io::print("verifyfail " & label & " " & toString(e.code))
    EXIT SUB
  END TRAP
  io::print("verify " & label & " " & toString(ok))
END SUB

SUB doExchange(label AS String, privHex AS String, peerHex AS String)
  LET secret AS List OF Byte = crypto::exchange(certOf(label), decodeField(privHex), decodeField(peerHex)) TRAP(e)
    io::print("exchangefail " & label & " " & toString(e.code))
    EXIT SUB
  END TRAP
  io::print("exchange " & label & " " & hexField(secret))
END SUB

SUB doConvert(label AS String, privHex AS String, pubHex AS String)
  MUT conv AS crypto::KeyConvert = crypto::KeyConvert.Ed25519ToX25519
  IF label = "ed448-x448" THEN
    conv = crypto::KeyConvert.Ed448ToX448
  END IF
  LET pair AS crypto::KeyPair = crypto::KeyPair[decodeField(privHex), decodeField(pubHex)]
  LET k AS crypto::KeyPair = crypto::convert(conv, pair) TRAP(e)
    io::print("convertfail " & label & " " & toString(e.code))
    EXIT SUB
  END TRAP
  io::print("convert " & label & " " & hexField(k.privateKey) & " " & hexField(k.publicKey))
END SUB

SUB dispatch(spec AS String)
  LET f AS List OF String = strings::split(spec, ",")
  LET op AS String = collections::get(f, 0)
  IF op = "gen" THEN
    doGen(collections::get(f, 1))
  END IF
  IF op = "sign" THEN
    doSign(collections::get(f, 1), collections::get(f, 2), collections::get(f, 3))
  END IF
  IF op = "verify" THEN
    doVerify(collections::get(f, 1), collections::get(f, 2), collections::get(f, 3), collections::get(f, 4))
  END IF
  IF op = "exchange" THEN
    doExchange(collections::get(f, 1), collections::get(f, 2), collections::get(f, 3))
  END IF
  IF op = "convert" THEN
    doConvert(collections::get(f, 1), collections::get(f, 2), collections::get(f, 3))
  END IF
END SUB

SUB main()
  LET job AS String = os::getEnvOr("MFB_KEY_JOB", "")
  IF job = "" THEN
    EXIT SUB
  END IF
  LET records AS List OF String = strings::split(job, ";")
  FOR EACH one IN records
    IF one <> "" THEN
      dispatch(one)
    END IF
  NEXT
END SUB
"#;

fn unhex(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    assert!(s.len() % 2 == 0, "odd-length hex field: {s}");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex field"))
        .collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Build once, then ask the program a batch of questions. Every call is one
/// process, so a test that needs a second round (because it must see MFB's
/// answers before it can form the next request) simply calls this again.
struct Mfb {
    exe: std::path::PathBuf,
}

impl Mfb {
    fn new(name: &str) -> Self {
        Mfb {
            exe: build_project(&temp_project(name, SOURCE)),
        }
    }

    /// One reply line per request, in order.
    fn ask(&self, requests: &[String]) -> Vec<String> {
        let job = requests.join(";");
        let (code, stdout, stderr) = run_capture_with_env(&self.exe, &[("MFB_KEY_JOB", job)]);
        assert_eq!(
            code, 0,
            "program failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        let lines: Vec<String> = stdout.lines().map(str::to_string).collect();
        assert_eq!(
            lines.len(),
            requests.len(),
            "asked {} question(s), got {} answer(s):\n{stdout}",
            requests.len(),
            lines.len()
        );
        lines
    }
}

/// `gen <label> <priv> <pub>` -> (priv, pub), failing loudly on `genfail`.
fn parse_gen(line: &str) -> (Vec<u8>, Vec<u8>) {
    let f: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(f[0], "gen", "expected a generated key pair, got: {line}");
    (unhex(f[2]), unhex(f[3]))
}

fn parse_one(kind: &str, line: &str) -> Vec<u8> {
    let f: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(f[0], kind, "expected a {kind} reply, got: {line}");
    unhex(f[f.len() - 1])
}

fn parse_verify(line: &str) -> bool {
    let f: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(f[0], "verify", "expected a verify reply, got: {line}");
    match f[2] {
        "TRUE" => true,
        "FALSE" => false,
        other => panic!("unexpected verify result {other} in: {line}"),
    }
}

/// `clamp(SHA-512(seed)[0..32])` — the X25519 private key an Ed25519 seed maps
/// to, computed here rather than taken from `crypto::convert`'s own output.
fn ed25519_seed_to_x25519_secret(seed: &[u8]) -> [u8; 32] {
    let mut h = [0u8; 32];
    h.copy_from_slice(&Sha512::digest(seed)[..32]);
    h[0] &= 248;
    h[31] &= 127;
    h[31] |= 64;
    h
}

fn x25519(secret: &[u8], point: &[u8]) -> Vec<u8> {
    let (mut k, mut p) = ([0u8; 32], [0u8; 32]);
    k.copy_from_slice(secret);
    p.copy_from_slice(point);
    curve25519_dalek::MontgomeryPoint(p)
        .mul_clamped(k)
        .to_bytes()
        .to_vec()
}

fn x25519_base(secret: &[u8]) -> Vec<u8> {
    let mut k = [0u8; 32];
    k.copy_from_slice(secret);
    curve25519_dalek::MontgomeryPoint::mul_base_clamped(k)
        .to_bytes()
        .to_vec()
}

const MESSAGE: &[u8] = b"the message both implementations must agree about";

#[test]
fn ed25519_x25519_and_convert_interoperate_both_ways() {
    let mfb = Mfb::new("crypto_key_interop_25519");

    // --- round 1: everything that needs MFB to go first ---------------------
    // A Rust-side Ed25519 pair and X25519 pair, so the same run can also answer
    // the "verify what the foreign side made" questions.
    let rust_ed = SigningKey::from_bytes(&[7u8; 32]);
    let rust_ed_pub = rust_ed.verifying_key().to_bytes();
    let rust_ed_sig = rust_ed.sign(MESSAGE).to_bytes();
    let mut bad_sig = rust_ed_sig;
    bad_sig[0] ^= 1;
    let rust_x_secret = [9u8; 32];
    let rust_x_public = x25519_base(&rust_x_secret);

    let replies = mfb.ask(&[
        "gen,ed25519".to_string(),
        "gen,x25519".to_string(),
        format!(
            "verify,ed25519,{},{},{}",
            hex(&rust_ed_pub),
            hex(MESSAGE),
            hex(&rust_ed_sig)
        ),
        format!(
            "verify,ed25519,{},{},{}",
            hex(&rust_ed_pub),
            hex(MESSAGE),
            hex(&bad_sig)
        ),
    ]);
    let (ed_seed, ed_pub) = parse_gen(&replies[0]);
    let (x_secret, x_public) = parse_gen(&replies[1]);

    // MFB accepts a signature made entirely elsewhere, and rejects a corrupted
    // one. Without the second, a `verify` that returned TRUE unconditionally
    // would pass.
    assert!(
        parse_verify(&replies[2]),
        "MFB rejected an ed25519-dalek signature over a dalek key"
    );
    assert!(
        !parse_verify(&replies[3]),
        "MFB ACCEPTED a corrupted ed25519 signature -- verify is not checking"
    );

    // --- the key pairs MFB generated must be internally consistent ----------
    assert_eq!(ed_seed.len(), 32, "ed25519 private key is a 32-byte seed");
    assert_eq!(ed_pub.len(), 32, "ed25519 public key is 32 bytes");
    let derived = SigningKey::from_bytes(&ed_seed.clone().try_into().unwrap())
        .verifying_key()
        .to_bytes();
    assert_eq!(
        hex(&derived),
        hex(&ed_pub),
        "the ed25519 public key MFB reported is not the one its seed derives"
    );
    assert_eq!(
        hex(&x25519_base(&x_secret)),
        hex(&x_public),
        "the x25519 public key MFB reported is not its private key's base multiple"
    );

    // --- round 2: sign, exchange and convert with what MFB just made --------
    let replies = mfb.ask(&[
        format!("sign,ed25519,{},{}", hex(&ed_seed), hex(MESSAGE)),
        format!("exchange,x25519,{},{}", hex(&x_secret), hex(&rust_x_public)),
        format!("convert,ed25519-x25519,{},{}", hex(&ed_seed), hex(&ed_pub)),
    ]);
    let mfb_sig = parse_one("sign", &replies[0]);
    let mfb_shared = parse_one("exchange", &replies[1]);
    let convert_fields: Vec<&str> = replies[2].split_whitespace().collect();
    assert_eq!(
        convert_fields[0], "convert",
        "convert failed: {}",
        replies[2]
    );
    let (conv_secret, conv_public) = (unhex(convert_fields[2]), unhex(convert_fields[3]));

    // Ed25519 is deterministic, so the signature is not merely valid -- it is
    // THE signature. A round trip would accept a signer that picked its nonce
    // differently and still verified.
    let dalek_sig = SigningKey::from_bytes(&ed_seed.clone().try_into().unwrap())
        .sign(MESSAGE)
        .to_bytes();
    assert_eq!(
        hex(&mfb_sig),
        hex(&dalek_sig),
        "MFB's ed25519 signature differs from ed25519-dalek's over the same key"
    );
    // ... and it verifies under two independent verifiers.
    VerifyingKey::from_bytes(&ed_pub.clone().try_into().unwrap())
        .expect("dalek public key")
        .verify(
            MESSAGE,
            &Signature::from_bytes(&mfb_sig.clone().try_into().unwrap()),
        )
        .expect("ed25519-dalek rejected MFB's signature");
    UnparsedPublicKey::new(&ED25519, &ed_pub)
        .verify(MESSAGE, &mfb_sig)
        .expect("ring rejected MFB's ed25519 signature");

    // ECDH: each side used its own private key and the other's public key, and
    // the two must land on the same secret. This is the property, not a byte
    // comparison of one side against itself.
    assert_eq!(
        hex(&mfb_shared),
        hex(&x25519(&rust_x_secret, &x_public)),
        "X25519 shared secrets disagree between MFB and curve25519-dalek"
    );
    assert_ne!(
        hex(&mfb_shared),
        hex(&[0u8; 32]),
        "an all-zero X25519 secret means the exchange degenerated"
    );

    // convert: both halves re-derived from the Ed25519 pair, independently.
    assert_eq!(
        hex(&conv_secret),
        hex(&ed25519_seed_to_x25519_secret(&ed_seed)),
        "converted X25519 private key is not clamp(SHA-512(seed)[0..32])"
    );
    // The public half checked two ways: the birational edwards->montgomery map
    // of the Ed25519 public key, and the base multiple of the converted secret.
    let mapped = curve25519_dalek::edwards::CompressedEdwardsY(ed_pub.clone().try_into().unwrap())
        .decompress()
        .expect("ed25519 public key is a curve point")
        .to_montgomery()
        .to_bytes();
    assert_eq!(
        hex(&conv_public),
        hex(&mapped),
        "converted X25519 public key is not the birational map of the Ed25519 one"
    );
    assert_eq!(
        hex(&conv_public),
        hex(&x25519_base(&conv_secret)),
        "converted X25519 pair is not self-consistent"
    );

    // The converted pair must actually work as an X25519 key, not merely look
    // like one -- a conversion that produced well-formed nonsense would pass
    // every check above if both halves were wrong in the same way.
    let replies = mfb.ask(&[format!(
        "exchange,x25519,{},{}",
        hex(&conv_secret),
        hex(&rust_x_public)
    )]);
    assert_eq!(
        hex(&parse_one("exchange", &replies[0])),
        hex(&x25519(&rust_x_secret, &conv_public)),
        "the converted key pair does not agree with dalek in an exchange"
    );
}

/// The NIST curves bind the platform key API (SecKey / EVP_PKEY / CNG) rather
/// than an MFBASIC core, so what is under test here is the *encoding contract*:
/// a 65/97-byte `04‖X‖Y` public key and an ASN.1 DER signature, both of which
/// `ring` must accept, and `ring`'s own must be accepted back.
///
/// ECDSA signing is randomized, so there is nothing to compare byte for byte --
/// unlike Ed25519 above, only the round trip through the foreign implementation
/// is available, in both directions.
#[test]
fn nist_ecdsa_interoperates_both_ways() {
    let mfb = Mfb::new("crypto_key_interop_nist");
    let rng = SystemRandom::new();

    struct Curve {
        label: &'static str,
        signing: &'static ring::signature::EcdsaSigningAlgorithm,
        verifying: &'static ring::signature::EcdsaVerificationAlgorithm,
        public_len: usize,
        private_len: usize,
    }
    let curves = [
        Curve {
            label: "p256",
            signing: &ECDSA_P256_SHA256_ASN1_SIGNING,
            verifying: &ECDSA_P256_SHA256_ASN1,
            public_len: 65,
            private_len: 97,
        },
        Curve {
            label: "p384",
            signing: &ECDSA_P384_SHA384_ASN1_SIGNING,
            verifying: &ECDSA_P384_SHA384_ASN1,
            public_len: 97,
            private_len: 145,
        },
    ];

    for curve in &curves {
        // ring generates a pair and signs, so the same round can ask MFB to
        // verify a signature made entirely elsewhere.
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(curve.signing, &rng).expect("ring generate");
        let ring_pair =
            EcdsaKeyPair::from_pkcs8(curve.signing, pkcs8.as_ref(), &rng).expect("ring from_pkcs8");
        let ring_public = ring_pair.public_key().as_ref().to_vec();
        let ring_sig = ring_pair
            .sign(&rng, MESSAGE)
            .expect("ring sign")
            .as_ref()
            .to_vec();
        let mut bad_sig = ring_sig.clone();
        let last = bad_sig.len() - 1;
        bad_sig[last] ^= 1;

        let replies = mfb.ask(&[
            format!("gen,{}", curve.label),
            format!(
                "verify,{},{},{},{}",
                curve.label,
                hex(&ring_public),
                hex(MESSAGE),
                hex(&ring_sig)
            ),
            format!(
                "verify,{},{},{},{}",
                curve.label,
                hex(&ring_public),
                hex(MESSAGE),
                hex(&bad_sig)
            ),
        ]);
        let (mfb_private, mfb_public) = parse_gen(&replies[0]);

        assert!(
            parse_verify(&replies[1]),
            "{}: MFB rejected a ring-made signature over a ring-made key",
            curve.label
        );
        assert!(
            !parse_verify(&replies[2]),
            "{}: MFB ACCEPTED a corrupted ECDSA signature",
            curve.label
        );

        // The platform's external representation is `04‖X‖Y` for the public key
        // and `04‖X‖Y‖d` for the private one, so the public key must be a literal
        // prefix of the private key. That is an encoding claim worth pinning:
        // if it ever stops holding, every consumer slicing these bytes breaks.
        assert_eq!(
            mfb_public.len(),
            curve.public_len,
            "{}: public key length",
            curve.label
        );
        assert_eq!(
            mfb_private.len(),
            curve.private_len,
            "{}: private key length",
            curve.label
        );
        assert_eq!(
            hex(&mfb_private[..curve.public_len]),
            hex(&mfb_public),
            "{}: the private key does not carry its own public key as a prefix",
            curve.label
        );
        assert_eq!(
            mfb_public[0], 0x04,
            "{}: uncompressed point marker",
            curve.label
        );

        // MFB signs with the pair it generated; ring must accept both the key
        // encoding and the DER signature.
        let replies = mfb.ask(&[format!(
            "sign,{},{},{}",
            curve.label,
            hex(&mfb_private),
            hex(MESSAGE)
        )]);
        let mfb_sig = parse_one("sign", &replies[0]);
        UnparsedPublicKey::new(curve.verifying, &mfb_public)
            .verify(MESSAGE, &mfb_sig)
            .unwrap_or_else(|_| {
                panic!(
                    "{}: ring rejected MFB's signature ({} bytes) under MFB's public key",
                    curve.label,
                    mfb_sig.len()
                )
            });

        // And ring must REJECT it over a different message, or "verified" would
        // mean nothing.
        assert!(
            UnparsedPublicKey::new(curve.verifying, &mfb_public)
                .verify(b"a different message", &mfb_sig)
                .is_err(),
            "{}: ring accepted MFB's signature over the wrong message",
            curve.label
        );
    }
}
