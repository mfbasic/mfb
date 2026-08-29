//! plan-109-E/F interop proof for `crypto::encrypt` / `crypto::decrypt`: the wire
//! value is RFC 9180 HPKE base mode, `enc ‖ ct`, and it interoperates in BOTH
//! directions with an independent implementation — one written here from the
//! RFC over `curve25519-dalek` (X25519) and `ring` (HMAC/HKDF, AES-256-GCM,
//! ChaCha20Poly1305); none of it shares code with the MFB package.
//!
//! For each profile the test (1) seals a message with a random ephemeral key in
//! Rust and has an MFB program open it, and (2) has the same MFB program seal a
//! message (its own random ephemeral key) and opens it here. The recipient is the
//! fixed RFC 8032 §7.1 test-1 Ed25519 seed, converted to the KEM curve exactly as
//! `crypto::convert(KeyConvert.Ed25519ToX25519, …)` does
//! (`clamp(SHA-512(seed)[0..32])`), so both sides derive the same key pair without
//! any private helper. The Rust side is itself checked against the RFC 9180
//! Appendix A.1 vector (`hpke_rust_side_reproduces_rfc9180_a1`) before it is
//! trusted as the oracle.

mod common;
use common::{build_project, run_capture_with_env, temp_project};
use ring::aead::{
    Aad, LessSafeKey, Nonce, UnboundKey, AES_128_GCM, AES_256_GCM, CHACHA20_POLY1305,
};
use ring::hkdf::{Prk, HKDF_SHA256, HKDF_SHA512};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha512};

// ---------------------------------------------------------------------------
// RFC 9180 profile table (explicit properties, no ordinal arithmetic).
// ---------------------------------------------------------------------------
#[derive(Clone, Copy)]
struct Profile {
    mfb_name: &'static str,
    kem_id: u16,
    kdf_id: u16,
    aead_id: u16,
    nenc: usize,
    nsecret: usize,
    nk: usize,
}

const PROFILES: &[Profile] = &[
    Profile {
        mfb_name: "Ed25519_AES256GCM",
        kem_id: 0x0020,
        kdf_id: 0x0001,
        aead_id: 0x0002,
        nenc: 32,
        nsecret: 32,
        nk: 32,
    },
    Profile {
        mfb_name: "Ed25519_CHACHA20POLY1305",
        kem_id: 0x0020,
        kdf_id: 0x0001,
        aead_id: 0x0003,
        nenc: 32,
        nsecret: 32,
        nk: 32,
    },
];

// RFC 8032 §7.1 test-1 Ed25519 seed (the recipient identity) and its public key.
const ED25519_SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const ED25519_PUB: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

struct Len(usize);
impl ring::hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn hmac_alg(kdf_id: u16) -> hmac::Algorithm {
    match kdf_id {
        0x0001 => hmac::HMAC_SHA256,
        0x0003 => hmac::HMAC_SHA512,
        other => panic!("unknown KDF id {other:#06x}"),
    }
}

fn hkdf_alg(kdf_id: u16) -> ring::hkdf::Algorithm {
    match kdf_id {
        0x0001 => HKDF_SHA256,
        0x0003 => HKDF_SHA512,
        other => panic!("unknown KDF id {other:#06x}"),
    }
}

/// RFC 9180 `LabeledExtract` — HKDF-Extract is one HMAC under `salt` (an empty
/// salt is `HashLen` zero bytes, which `hmac::Key::new` zero-pads to anyway).
fn labeled_extract(kdf_id: u16, suite_id: &[u8], salt: &[u8], label: &[u8], ikm: &[u8]) -> Vec<u8> {
    let mut labeled = b"HPKE-v1".to_vec();
    labeled.extend_from_slice(suite_id);
    labeled.extend_from_slice(label);
    labeled.extend_from_slice(ikm);
    let key = hmac::Key::new(hmac_alg(kdf_id), salt);
    hmac::sign(&key, &labeled).as_ref().to_vec()
}

/// RFC 9180 `LabeledExpand` over ring's HKDF-Expand.
fn labeled_expand(
    kdf_id: u16,
    prk: &[u8],
    suite_id: &[u8],
    label: &[u8],
    info: &[u8],
    len: usize,
) -> Vec<u8> {
    let mut labeled = (len as u16).to_be_bytes().to_vec();
    labeled.extend_from_slice(b"HPKE-v1");
    labeled.extend_from_slice(suite_id);
    labeled.extend_from_slice(label);
    labeled.extend_from_slice(info);
    let prk = Prk::new_less_safe(hkdf_alg(kdf_id), prk);
    let mut out = vec![0u8; len];
    prk.expand(&[&labeled], Len(len))
        .expect("expand")
        .fill(&mut out)
        .expect("fill");
    out
}

fn kem_suite_id(p: &Profile) -> Vec<u8> {
    let mut id = b"KEM".to_vec();
    id.extend_from_slice(&p.kem_id.to_be_bytes());
    id
}

fn hpke_suite_id(p: &Profile) -> Vec<u8> {
    let mut id = b"HPKE".to_vec();
    id.extend_from_slice(&p.kem_id.to_be_bytes());
    id.extend_from_slice(&p.kdf_id.to_be_bytes());
    id.extend_from_slice(&p.aead_id.to_be_bytes());
    id
}

// ---------------------------------------------------------------------------
// KEM curve: X25519 via curve25519-dalek (clamps the scalar itself).
// ---------------------------------------------------------------------------
fn x25519(sk: &[u8], u: &[u8]) -> Vec<u8> {
    let mut k = [0u8; 32];
    k.copy_from_slice(sk);
    let mut p = [0u8; 32];
    p.copy_from_slice(u);
    curve25519_dalek::MontgomeryPoint(p)
        .mul_clamped(k)
        .to_bytes()
        .to_vec()
}

fn dh(p: &Profile, sk: &[u8], pk: &[u8]) -> Vec<u8> {
    match p.kem_id {
        0x0020 => x25519(sk, pk),
        other => panic!("unknown KEM id {other:#06x}"),
    }
}

fn base_point(p: &Profile) -> Vec<u8> {
    let mut b = vec![0u8; p.nenc];
    b[0] = match p.kem_id {
        0x0020 => 9,
        other => panic!("unknown KEM id {other:#06x}"),
    };
    b
}

/// The recipient's KEM key pair from its Ed25519 seed, exactly as
/// `crypto::convert(KeyConvert.Ed25519ToX25519, …)` derives it.
fn recipient_keys(p: &Profile) -> (Vec<u8>, Vec<u8>) {
    let seed = hex_decode(ED25519_SEED);
    let sk = match p.kem_id {
        0x0020 => {
            let mut h = Sha512::digest(&seed)[..32].to_vec();
            h[0] &= 248;
            h[31] &= 127;
            h[31] |= 64;
            h
        }
        other => panic!("unknown KEM id {other:#06x}"),
    };
    let pk = dh(p, &sk, &base_point(p));
    (sk, pk)
}

fn extract_and_expand(p: &Profile, dh_out: &[u8], kem_context: &[u8]) -> Vec<u8> {
    let suite = kem_suite_id(p);
    let prk = labeled_extract(p.kdf_id, &suite, &[], b"eae_prk", dh_out);
    labeled_expand(
        p.kdf_id,
        &prk,
        &suite,
        b"shared_secret",
        kem_context,
        p.nsecret,
    )
}

/// Base-mode key schedule with empty `info` and no PSK: (key, base_nonce).
fn key_schedule(p: &Profile, shared_secret: &[u8], info: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let suite = hpke_suite_id(p);
    let psk_id_hash = labeled_extract(p.kdf_id, &suite, &[], b"psk_id_hash", &[]);
    let info_hash = labeled_extract(p.kdf_id, &suite, &[], b"info_hash", info);
    let mut context = vec![0u8];
    context.extend_from_slice(&psk_id_hash);
    context.extend_from_slice(&info_hash);
    let secret = labeled_extract(p.kdf_id, &suite, shared_secret, b"secret", &[]);
    let key = labeled_expand(p.kdf_id, &secret, &suite, b"key", &context, p.nk);
    let base_nonce = labeled_expand(p.kdf_id, &secret, &suite, b"base_nonce", &context, 12);
    (key, base_nonce)
}

fn aead_key(p: &Profile, key: &[u8]) -> LessSafeKey {
    let alg = match (p.aead_id, p.nk) {
        (0x0001, 16) => &AES_128_GCM,
        (0x0002, 32) => &AES_256_GCM,
        (0x0003, 32) => &CHACHA20_POLY1305,
        other => panic!("unknown AEAD {other:?}"),
    };
    LessSafeKey::new(UnboundKey::new(alg, key).expect("aead key"))
}

/// RFC 9180 single-shot base-mode `Seal` with the given ephemeral private key.
fn hpke_seal(p: &Profile, pk_r: &[u8], sk_e: &[u8], info: &[u8], aad: &[u8], pt: &[u8]) -> Vec<u8> {
    let enc = dh(p, sk_e, &base_point(p));
    let dh_out = dh(p, sk_e, pk_r);
    let mut kem_context = enc.clone();
    kem_context.extend_from_slice(pk_r);
    let shared = extract_and_expand(p, &dh_out, &kem_context);
    let (key, base_nonce) = key_schedule(p, &shared, info);
    let mut in_out = pt.to_vec();
    let nonce = Nonce::try_assume_unique_for_key(&base_nonce).expect("nonce");
    aead_key(p, &key)
        .seal_in_place_append_tag(nonce, Aad::from(aad), &mut in_out)
        .expect("seal");
    let mut out = enc;
    out.extend_from_slice(&in_out);
    out
}

/// RFC 9180 single-shot base-mode `Open`; `None` on any authentication failure.
fn hpke_open(p: &Profile, sk_r: &[u8], info: &[u8], aad: &[u8], boxed: &[u8]) -> Option<Vec<u8>> {
    if boxed.len() < p.nenc + 16 {
        return None;
    }
    let enc = &boxed[..p.nenc];
    let pk_r = dh(p, sk_r, &base_point(p));
    let dh_out = dh(p, sk_r, enc);
    let mut kem_context = enc.to_vec();
    kem_context.extend_from_slice(&pk_r);
    let shared = extract_and_expand(p, &dh_out, &kem_context);
    let (key, base_nonce) = key_schedule(p, &shared, info);
    let mut in_out = boxed[p.nenc..].to_vec();
    let nonce = Nonce::try_assume_unique_for_key(&base_nonce).ok()?;
    let pt = aead_key(p, &key)
        .open_in_place(nonce, Aad::from(aad), &mut in_out)
        .ok()?;
    Some(pt.to_vec())
}

/// The Rust side must reproduce the RFC 9180 Appendix A.1 vector (DHKEM(X25519,
/// HKDF-SHA256), HKDF-SHA256, AES-128-GCM) before it is trusted as the oracle.
#[test]
fn hpke_rust_side_reproduces_rfc9180_a1() {
    let p = Profile {
        mfb_name: "",
        kem_id: 0x0020,
        kdf_id: 0x0001,
        aead_id: 0x0001,
        nenc: 32,
        nsecret: 32,
        nk: 16,
    };
    let sk_e = hex_decode("52c4a758a802cd8b936eceea314432798d5baf2d7e9235dc084ab1b9cfa2f736");
    let pk_r = hex_decode("3948cfe0ad1ddb695d780e59077195da6c56506b027329794ab02bca80815c4d");
    let sk_r = hex_decode("4612c550263fc8ad58375df3f557aac531d26850903e55a9f23f21d8534e8ac8");
    let info = hex_decode("4f6465206f6e2061204772656369616e2055726e");
    let boxed = hpke_seal(
        &p,
        &pk_r,
        &sk_e,
        &info,
        b"Count-0",
        b"Beauty is truth, truth beauty",
    );
    assert_eq!(
        hex_encode(&boxed[..32]),
        "37fda3567bdbd628e88668c3c8d7e97d1d1253b6d4ea6d44c150f741f1bf4431"
    );
    assert_eq!(
        hex_encode(&boxed[32..]),
        "f938558b5d72f1a23810b4be2ab4f84331acc02fc97babc53a52ae8218a355a96d8770ac83d07bea87e13c512a"
    );
    assert_eq!(
        hpke_open(&p, &sk_r, &info, b"Count-0", &boxed).as_deref(),
        Some(&b"Beauty is truth, truth beauty"[..])
    );
    // The recipient identity: the RFC 8032 seed's public key is what the MFB
    // program is handed.
    assert_eq!(hex_decode(ED25519_PUB).len(), 32);
}

/// Both directions for every profile through the public MFB surface.
#[test]
fn hpke_boxes_interoperate_with_independent_implementation_both_ways() {
    let rng = SystemRandom::new();
    let pt_in = b"HPKE interop: independent -> MFB".to_vec();
    let aad_in = b"interop-aad".to_vec();
    let pt_out = b"HPKE interop: MFB -> independent".to_vec();
    let aad_out = b"mfb-aad".to_vec();

    // Independent implementation seals one box per profile (random ephemeral key).
    let mut boxes_in = Vec::new();
    for p in PROFILES {
        let (_, pk_r) = recipient_keys(p);
        let mut sk_e = vec![0u8; p.nenc];
        rng.fill(&mut sk_e).expect("rng");
        boxes_in.push(hpke_seal(p, &pk_r, &sk_e, &[], &aad_in, &pt_in));
    }

    let mut source = String::from("IMPORT crypto\nIMPORT encoding\nIMPORT io\n\nSUB main()\n");
    source.push_str(&format!(
        "  LET seed AS List OF Byte = encoding::hexDecode(\"{ED25519_SEED}\")\n"
    ));
    source.push_str(&format!(
        "  LET pub AS List OF Byte = encoding::hexDecode(\"{ED25519_PUB}\")\n"
    ));
    source.push_str(&format!(
        "  LET aadIn AS List OF Byte = encoding::hexDecode(\"{}\")\n",
        hex_encode(&aad_in)
    ));
    source.push_str(&format!(
        "  LET aadOut AS List OF Byte = encoding::hexDecode(\"{}\")\n",
        hex_encode(&aad_out)
    ));
    source.push_str(&format!(
        "  LET ptOut AS List OF Byte = encoding::hexDecode(\"{}\")\n",
        hex_encode(&pt_out)
    ));
    for (p, boxed) in PROFILES.iter().zip(&boxes_in) {
        let name = p.mfb_name;
        source.push_str(&format!(
            "  io::print(\"open-{name}=\" & encoding::hexEncode(crypto::decrypt(AsymmetricCipher.{name}, seed, encoding::hexDecode(\"{}\"), aadIn)))\n",
            hex_encode(boxed)
        ));
        source.push_str(&format!(
            "  io::print(\"seal-{name}=\" & encoding::hexEncode(crypto::encrypt(AsymmetricCipher.{name}, pub, ptOut, aadOut)))\n"
        ));
    }
    source.push_str("END SUB\n");

    let project = temp_project("hpke_interop", &source);
    let exe = build_project(&project);
    let (code, stdout, stderr) = run_capture_with_env(&exe, &[]);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");

    for p in PROFILES {
        let name = p.mfb_name;
        let opened = stdout
            .lines()
            .find_map(|l| l.strip_prefix(&format!("open-{name}=")))
            .unwrap_or_else(|| panic!("no open-{name} line in:\n{stdout}"));
        assert_eq!(
            hex_decode(opened),
            pt_in,
            "MFB must open the independent {name} box"
        );
        let sealed = stdout
            .lines()
            .find_map(|l| l.strip_prefix(&format!("seal-{name}=")))
            .unwrap_or_else(|| panic!("no seal-{name} line in:\n{stdout}"));
        let boxed = hex_decode(sealed);
        assert_eq!(
            boxed.len(),
            p.nenc + pt_out.len() + 16,
            "{name}: enc || ct || tag"
        );
        let (sk_r, _) = recipient_keys(p);
        assert_eq!(
            hpke_open(p, &sk_r, &[], &aad_out, &boxed).as_deref(),
            Some(&pt_out[..]),
            "independent implementation must open the MFB {name} box"
        );
        // A different aad or a flipped byte must fail on the independent side too,
        // proving the tag binds the caller's aad.
        assert!(hpke_open(p, &sk_r, &[], b"other", &boxed).is_none());
        let mut flipped = boxed.clone();
        flipped[p.nenc] ^= 1;
        assert!(hpke_open(p, &sk_r, &[], &aad_out, &flipped).is_none());
    }
    let _ = std::fs::remove_dir_all(project);
}
