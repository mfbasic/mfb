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
    Profile {
        mfb_name: "Ed448_AES256GCM",
        kem_id: 0x0021,
        kdf_id: 0x0003,
        aead_id: 0x0002,
        nenc: 56,
        nsecret: 64,
        nk: 32,
    },
    Profile {
        mfb_name: "Ed448_CHACHA20POLY1305",
        kem_id: 0x0021,
        kdf_id: 0x0003,
        aead_id: 0x0003,
        nenc: 56,
        nsecret: 64,
        nk: 32,
    },
];

// RFC 8032 §7.1 test-1 Ed25519 seed (the recipient identity) and its public key.
const ED25519_SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const ED25519_PUB: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
// RFC 8032 §7.4 test-1 Ed448 seed and its public key.
const ED448_SEED: &str = "6c82a562cb808d10d632be89c8513ebf6c929f34ddfa8c9f63c9960ef6e348a3528c8a3fcc2f044e39a3fc5b94492f8f032e7549a20098f95b";
const ED448_PUB: &str = "5fd7449b59b461fd2ce787ec616ad46a1da1342485a70e1f8a0ea75d80e96778edf124769b46c7061bd6783df1e50f6cd1fa1abeafe8256180";

// ---------------------------------------------------------------------------
// X448 from RFC 7748 (independent of the MFB package): GF(2^448 − 2^224 − 1) as
// 16 × 28-bit limbs in u64/u128 arithmetic, the Montgomery ladder with a24 =
// 39081, checked against the RFC's iteration vectors below.
// ---------------------------------------------------------------------------
mod x448 {
    const MASK: u64 = (1 << 28) - 1;
    type Fe = [u64; 16];

    fn carry(mut r: [u128; 16]) -> Fe {
        // Two passes: limbs to 28 bits, the 2^448 overflow folding into limbs 0 and 8.
        for _ in 0..2 {
            let mut c: u128 = 0;
            for limb in r.iter_mut() {
                let v = *limb + c;
                *limb = v & MASK as u128;
                c = v >> 28;
            }
            r[0] += c;
            r[8] += c;
        }
        let mut out = [0u64; 16];
        for (o, v) in out.iter_mut().zip(r.iter()) {
            *o = *v as u64;
        }
        out
    }

    fn add(a: &Fe, b: &Fe) -> Fe {
        let mut r = [0u128; 16];
        for i in 0..16 {
            r[i] = a[i] as u128 + b[i] as u128;
        }
        carry(r)
    }

    fn sub(a: &Fe, b: &Fe) -> Fe {
        let mut r = [0u128; 16];
        for i in 0..16 {
            let bias: u128 = if i == 8 { 536_870_908 } else { 536_870_910 };
            r[i] = a[i] as u128 + bias - b[i] as u128;
        }
        carry(r)
    }

    fn mul(a: &Fe, b: &Fe) -> Fe {
        let mut c = [0u128; 31];
        for i in 0..16 {
            for j in 0..16 {
                c[i + j] += a[i] as u128 * b[j] as u128;
            }
        }
        let mut r = [0u128; 16];
        for n in 0..16 {
            let mut v = c[n];
            if n <= 14 {
                v += c[n + 16];
            }
            if n >= 8 {
                v += c[n + 8];
                if n <= 14 {
                    v += c[n + 16];
                }
            }
            if n <= 6 {
                v += c[n + 24];
            }
            r[n] = v;
        }
        carry(r)
    }

    fn mul_small(a: &Fe, k: u64) -> Fe {
        let mut r = [0u128; 16];
        for i in 0..16 {
            r[i] = a[i] as u128 * k as u128;
        }
        carry(r)
    }

    fn inv(a: &Fe) -> Fe {
        // a^(p−2); p − 2 = 2^448 − 2^224 − 3 has zero bits at 224 and 1 only.
        let mut c = *a;
        for i in (0..=446).rev() {
            c = mul(&c, &c);
            if i != 1 && i != 224 {
                c = mul(&c, a);
            }
        }
        c
    }

    fn unpack(b: &[u8]) -> Fe {
        let mut o = [0u64; 16];
        for g in 0..8 {
            let mut v: u64 = 0;
            for i in 0..7 {
                v |= (b[7 * g + i] as u64) << (8 * i);
            }
            o[2 * g] = v & MASK;
            o[2 * g + 1] = v >> 28;
        }
        o
    }

    fn pack(n: &Fe) -> Vec<u8> {
        let mut t = carry({
            let mut r = [0u128; 16];
            for i in 0..16 {
                r[i] = n[i] as u128;
            }
            r
        });
        t = carry({
            let mut r = [0u128; 16];
            for i in 0..16 {
                r[i] = t[i] as u128;
            }
            r
        });
        // Canonical: subtract p once when t >= p.
        let mut m = [0u64; 16];
        let mut borrow: i64 = 0;
        for i in 0..16 {
            let pi: i64 = if i == 8 { 268_435_454 } else { 268_435_455 };
            let d = t[i] as i64 - pi - borrow;
            borrow = (d >> 28) & 1;
            m[i] = (d & MASK as i64) as u64;
        }
        let r = if borrow == 1 { t } else { m };
        let mut out = Vec::with_capacity(56);
        for g in 0..8 {
            let v = r[2 * g] | (r[2 * g + 1] << 28);
            out.extend_from_slice(&v.to_le_bytes()[..7]);
        }
        out
    }

    fn cswap(a: &mut Fe, b: &mut Fe, swap: u64) {
        let mask = 0u64.wrapping_sub(swap);
        for i in 0..16 {
            let t = mask & (a[i] ^ b[i]);
            a[i] ^= t;
            b[i] ^= t;
        }
    }

    pub fn x448(scalar: &[u8], u: &[u8]) -> Vec<u8> {
        let mut k = scalar.to_vec();
        k[0] &= 252;
        k[55] |= 128;
        let x1 = unpack(u);
        let mut x2: Fe = [0; 16];
        x2[0] = 1;
        let mut z2: Fe = [0; 16];
        let mut x3 = x1;
        let mut z3: Fe = [0; 16];
        z3[0] = 1;
        let mut swap = 0u64;
        for t in (0..448).rev() {
            let kt = ((k[t / 8] >> (t % 8)) & 1) as u64;
            swap ^= kt;
            cswap(&mut x2, &mut x3, swap);
            cswap(&mut z2, &mut z3, swap);
            swap = kt;
            let a = add(&x2, &z2);
            let aa = mul(&a, &a);
            let b = sub(&x2, &z2);
            let bb = mul(&b, &b);
            let e = sub(&aa, &bb);
            let c = add(&x3, &z3);
            let d = sub(&x3, &z3);
            let da = mul(&d, &a);
            let cb = mul(&c, &b);
            let s = add(&da, &cb);
            x3 = mul(&s, &s);
            let f = sub(&da, &cb);
            z3 = mul(&x1, &mul(&f, &f));
            x2 = mul(&aa, &bb);
            z2 = mul(&e, &add(&aa, &mul_small(&e, 39081)));
        }
        cswap(&mut x2, &mut x3, swap);
        cswap(&mut z2, &mut z3, swap);
        pack(&mul(&x2, &inv(&z2)))
    }
}

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
        0x0021 => x448::x448(sk, pk),
        other => panic!("unknown KEM id {other:#06x}"),
    }
}

fn base_point(p: &Profile) -> Vec<u8> {
    let mut b = vec![0u8; p.nenc];
    b[0] = match p.kem_id {
        0x0020 => 9,
        0x0021 => 5,
        other => panic!("unknown KEM id {other:#06x}"),
    };
    b
}

/// Keccak-f[1600] (FIPS 202 §3) on 25 lanes, from the round-constant LFSR and the
/// rho offsets rather than any crate — the harness's own SHAKE256.
fn keccak_f1600(a: &mut [u64; 25]) {
    const RHO: [u32; 25] = [
        0, 1, 62, 28, 27, 36, 44, 6, 55, 20, 3, 10, 43, 25, 39, 41, 45, 15, 21, 8, 18, 2, 61, 56,
        14,
    ];
    let mut lfsr: u8 = 1;
    for _ in 0..24 {
        // Round constant: bit 2^j − 1 of RC is the LFSR output at step 7·round + j.
        let mut rc: u64 = 0;
        for j in 0..7 {
            if lfsr & 1 == 1 {
                rc |= 1 << ((1u32 << j) - 1);
            }
            let carry = lfsr & 0x80 != 0;
            lfsr <<= 1;
            if carry {
                lfsr ^= 0x71;
            }
        }
        // theta
        let mut c = [0u64; 5];
        for x in 0..5 {
            c[x] = a[x] ^ a[x + 5] ^ a[x + 10] ^ a[x + 15] ^ a[x + 20];
        }
        for x in 0..5 {
            let d = c[(x + 4) % 5] ^ c[(x + 1) % 5].rotate_left(1);
            for y in 0..5 {
                a[x + 5 * y] ^= d;
            }
        }
        // rho + pi
        let mut b = [0u64; 25];
        for x in 0..5 {
            for y in 0..5 {
                b[y + 5 * ((2 * x + 3 * y) % 5)] = a[x + 5 * y].rotate_left(RHO[x + 5 * y]);
            }
        }
        // chi
        for y in 0..5 {
            for x in 0..5 {
                a[x + 5 * y] = b[x + 5 * y] ^ (!b[(x + 1) % 5 + 5 * y] & b[(x + 2) % 5 + 5 * y]);
            }
        }
        // iota
        a[0] ^= rc;
    }
}

/// SHAKE256 of `seed`, `n` bytes — Ed448's key expansion (rate 136, suffix 0x1f).
fn shake256(seed: &[u8], n: usize) -> Vec<u8> {
    const RATE: usize = 136;
    let mut padded = seed.to_vec();
    padded.push(0x1f);
    while padded.len() % RATE != 0 {
        padded.push(0);
    }
    *padded.last_mut().unwrap() |= 0x80;
    let mut state = [0u64; 25];
    for block in padded.chunks(RATE) {
        for (lane, bytes) in block.chunks(8).enumerate() {
            state[lane] ^= u64::from_le_bytes(bytes.try_into().unwrap());
        }
        keccak_f1600(&mut state);
    }
    let mut out = Vec::with_capacity(n);
    loop {
        for lane in state.iter().take(RATE / 8) {
            out.extend_from_slice(&lane.to_le_bytes());
        }
        if out.len() >= n {
            out.truncate(n);
            return out;
        }
        keccak_f1600(&mut state);
    }
}

/// The harness's SHAKE256 must reproduce FIPS 202 known answers (empty and "abc")
/// and the Ed448 seed's expansion (Python `hashlib.shake_256`) before its output
/// is trusted as the recipient's X448 private key.
#[test]
fn shake256_rust_side_reproduces_known_answers() {
    assert_eq!(
        hex_encode(&shake256(b"", 32)),
        "46b9dd2b0ba88d13233b3feb743eeb243fcd52ea62b81b82b50c27646ed5762f"
    );
    assert_eq!(
        hex_encode(&shake256(b"abc", 32)),
        "483366601360a8771c6863080cc4114d8db44530f8f1e1ee4f94ea37e78b5739"
    );
    assert_eq!(
        hex_encode(&shake256(&hex_decode(ED448_SEED), 56)),
        "eb3930a0cea0808ec7ed6667f472a588b411f0545ba4f3ee75025e1d38519cb905c036d81eeed17483f9f56615ceee4fa70501a71fc0bbb7"
    );
    // Multi-block absorb and squeeze-past-one-block paths.
    let long = vec![0x61u8; 300];
    let full = shake256(&long, 400);
    assert_eq!(shake256(&long, 137), full[..137]);
}

/// The recipient's KEM key pair from its Ed25519 / Ed448 seed, exactly as
/// `crypto::convert(KeyConvert.Ed25519ToX25519 / Ed448ToX448, …)` derives it.
fn recipient_keys(p: &Profile) -> (Vec<u8>, Vec<u8>) {
    let sk = match p.kem_id {
        0x0020 => {
            let mut h = Sha512::digest(hex_decode(ED25519_SEED))[..32].to_vec();
            h[0] &= 248;
            h[31] &= 127;
            h[31] |= 64;
            h
        }
        0x0021 => shake256(&hex_decode(ED448_SEED), 56),
        other => panic!("unknown KEM id {other:#06x}"),
    };
    let pk = dh(p, &sk, &base_point(p));
    (sk, pk)
}

/// The from-scratch X448 must reproduce the RFC 7748 §5.2 iteration vectors
/// (1 and 1000 iterations from `k = u = 5`) and the §6.2 Alice/Bob exchange
/// before it is trusted as the oracle's KEM.
#[test]
fn x448_rust_side_reproduces_rfc7748_vectors() {
    let base = base_point(&PROFILES[2]);
    let (mut k, mut u) = (base.clone(), base.clone());
    for i in 1..=1000 {
        let next = x448::x448(&k, &u);
        u = k;
        k = next;
        if i == 1 {
            assert_eq!(
                hex_encode(&k),
                "3f482c8a9f19b01e6c46ee9711d9dc14fd4bf67af30765c2ae2b846a4d23a8cd0db897086239492caf350b51f833868b9bc2b3bca9cf4113"
            );
        }
    }
    assert_eq!(
        hex_encode(&k),
        "aa3b4749d55b9daf1e5b00288826c467274ce3ebbdd5c17b975e09d4af6c67cf10d087202db88286e2b79fceea3ec353ef54faa26e219f38"
    );
    let alice = hex_decode("9a8f4925d1519f5775cf46b04b5800d4ee9ee8bae8bc5565d498c28dd9c9baf574a9419744897391006382a6f127ab1d9ac2d8c0a598726b");
    let bob = hex_decode("1c306a7ac2a0e2e0990b294470cba339e6453772b075811d8fad0d1d6927c120bb5ee8972b0d3e21374c9c921b09d1b0366f10b65173992d");
    let alice_pub = x448::x448(&alice, &base);
    let bob_pub = x448::x448(&bob, &base);
    assert_eq!(
        hex_encode(&alice_pub),
        "9b08f7cc31b7e3e67d22d5aea121074a273bd2b83de09c63faa73d2c22c5d9bbc836647241d953d40c5b12da88120d53177f80e532c41fa0"
    );
    assert_eq!(
        hex_encode(&x448::x448(&alice, &bob_pub)),
        "07fff4181ac6cc95ec1c16a94a0f74d12da232ce40a77552281d282bb60c0b56fd2464c335543936521c24403085d59a449a5037514a879d"
    );
    assert_eq!(x448::x448(&alice, &bob_pub), x448::x448(&bob, &alice_pub));
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
    // The recipient identities: the RFC 8032 seeds' public keys are what the MFB
    // program is handed.
    assert_eq!(hex_decode(ED25519_PUB).len(), 32);
    assert_eq!(hex_decode(ED448_PUB).len(), 57);
    assert_eq!(hex_decode(ED448_SEED).len(), 57);
}

/// The Rust side must also reproduce the RFC 9180 Appendix A.6.1 base-mode vector
/// (DHKEM(X448, HKDF-SHA512), HKDF-SHA512, AES-256-GCM — the package's
/// `Ed448_AES256GCM` profile, with the RFC's non-empty `info`) so the from-scratch
/// X448 KEM is pinned to an official vector, not only to RFC 7748.
#[test]
fn hpke_rust_side_reproduces_rfc9180_a6() {
    let p = &PROFILES[2];
    assert_eq!((p.kem_id, p.kdf_id, p.aead_id), (0x0021, 0x0003, 0x0002));
    let sk_e = hex_decode("9abfbdf9132c22e95f4d25dc6ae16ca1269d3692e75f32e3aeecd4aee7cb8edb4e26da9422afb940c42caf388a1d1215b405795a28d43a60");
    let pk_r = hex_decode("d920db89afdb25df110a44cf0d7dc4e4d4b74f09ceaba5e76a12d3cafefcd962e244804a58bfd12303732be21d511f877ddc2ed694447b3d");
    let sk_r = hex_decode("c4e72a57af1640806c01617b947ee6d1bbe5eb1a5b4616fb705a5d2ed30b7f4317365c504249750e090805d44a2ddc2970172414a90a09e5");
    let info = hex_decode("4f6465206f6e2061204772656369616e2055726e");
    assert_eq!(dh(p, &sk_r, &base_point(p)), pk_r, "skRm -> pkRm");
    let boxed = hpke_seal(
        p,
        &pk_r,
        &sk_e,
        &info,
        b"Count-0",
        b"Beauty is truth, truth beauty",
    );
    assert_eq!(
        hex_encode(&boxed[..56]),
        "390f2971ca97d513915a2bc5aac0cb81b832d9424d2264eaa9e868d80862edd7918276883a8d0434309e049408fec2340ae5799702f948d7"
    );
    assert_eq!(
        hex_encode(&boxed[56..]),
        "6a5ef0f8c88a17c6d26bee63b4468cd43360eb69804fb392d8c9b8eba2f9bd806726c7d99cb9073022000ce41a"
    );
    let mut kem_context = boxed[..56].to_vec();
    kem_context.extend_from_slice(&pk_r);
    let shared = extract_and_expand(p, &dh(p, &sk_e, &pk_r), &kem_context);
    assert_eq!(
        hex_encode(&shared),
        "081f8572019ac78daca420cf23c5183027e9bdaa7fe4b5f8e55b2ff24bc5cdc8bf4362965e6ccd2b832af12b0ed6f2f669b15b42cb6f4361d36d99b88b7dc5a6"
    );
    let (key, base_nonce) = key_schedule(p, &shared, &info);
    assert_eq!(
        hex_encode(&key),
        "5011eed55726d94fae0cd116b80e7832ecde3a457ef816a4a42f862ec2820ade"
    );
    assert_eq!(hex_encode(&base_nonce), "c9899ce0c487a96933695f69");
    assert_eq!(
        hpke_open(p, &sk_r, &info, b"Count-0", &boxed).as_deref(),
        Some(&b"Beauty is truth, truth beauty"[..])
    );
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
        "  LET seed25519 AS List OF Byte = encoding::hexDecode(\"{ED25519_SEED}\")\n"
    ));
    source.push_str(&format!(
        "  LET pub25519 AS List OF Byte = encoding::hexDecode(\"{ED25519_PUB}\")\n"
    ));
    source.push_str(&format!(
        "  LET seed448 AS List OF Byte = encoding::hexDecode(\"{ED448_SEED}\")\n"
    ));
    source.push_str(&format!(
        "  LET pub448 AS List OF Byte = encoding::hexDecode(\"{ED448_PUB}\")\n"
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
        let curve = if p.kem_id == 0x0021 { "448" } else { "25519" };
        source.push_str(&format!(
            "  io::print(\"open-{name}=\" & encoding::hexEncode(crypto::decrypt(crypto::AsymmetricCipher.{name}, seed{curve}, encoding::hexDecode(\"{}\"), aadIn)))\n",
            hex_encode(boxed)
        ));
        source.push_str(&format!(
            "  io::print(\"seal-{name}=\" & encoding::hexEncode(crypto::encrypt(crypto::AsymmetricCipher.{name}, pub{curve}, ptOut, aadOut)))\n"
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
