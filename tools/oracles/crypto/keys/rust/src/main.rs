//! The DRIVER for the key-interop oracle: it asks the MFBASIC program in
//! `../mfb` to generate keys, sign, verify, exchange and convert, and checks
//! every answer against implementations that share no code with it.
//!
//! Why a driver and not a reference. The other oracles here are one-shot: the
//! MFB program emits every case and a shell script compares. Key interop cannot
//! work that way, because the questions depend on the answers -- you have to SEE
//! a key MFB generated before you can ask it to sign with that key, and you have
//! to see its public half before you can compute the matching shared secret.
//!
//! What "both directions" means concretely, per curve:
//!
//!   * MFB generates    -> we re-derive its public key from its private key.
//!                         A pair that does not satisfy its own definition is
//!                         broken however well it round-trips.
//!   * MFB signs        -> we verify. For the deterministic schemes (Ed25519,
//!                         Ed448) we also compare the signature BYTE FOR BYTE,
//!                         which a round trip cannot do: a signer that chose its
//!                         nonce differently would still verify.
//!   * we sign          -> MFB verifies, and MFB REJECTS a corrupted signature.
//!                         Without the second half, a `verify` that always
//!                         returned true would pass.
//!   * ECDH             -> each side uses its own private key and the other's
//!                         public key; the secrets must match.
//!
//! Coverage is the full `crypto::Certificate` matrix plus both `KeyConvert`
//! directions. `tests/rt_crypto_key_interop.rs` checks the Ed25519 / X25519 /
//! P-256 / P-384 subset on every `cargo test` (those crates were already in the
//! compiler's lockfile); Ed448, X448 and P-521 are only reachable here, and the
//! overlap is deliberate -- if the two references ever disagreed, this would say
//! so.
//!
//! Usage: keysref <path-to-the-built-mfb-program>

// `ed25519-dalek` re-exports the RustCrypto `signature` crate's traits, and the
// p256/p384/p521 ECDSA keys implement those same traits -- so this one import
// brings `sign`/`verify` into scope for every scheme here. Importing them again
// via `p256::ecdsa::signature` is the identical trait and warns as unused.
use ed25519_dalek::{Signer as _, Verifier as _};
use sha2::{Digest, Sha512};
use sha3::digest::{ExtendableOutput, Update as _, XofReader};
use std::process::Command;

const MESSAGE: &[u8] = b"the message both implementations must agree about";
/// RFC 7748's X448 base point: u = 5.
const X448_BASE: [u8; 56] = {
    let mut b = [0u8; 56];
    b[0] = 5;
    b
};

// ---------------------------------------------------------------------------
// Talking to the subject
// ---------------------------------------------------------------------------

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    assert!(s.len() % 2 == 0, "odd-length hex from the subject: {s}");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex from the subject"))
        .collect()
}

/// One reply line per request, in order. A short or failed run is fatal rather
/// than a finding: it means the harness is broken, not the implementation.
fn ask(mfb: &str, requests: &[String]) -> Vec<String> {
    let out = Command::new(mfb)
        .env("MFB_KEY_JOB", requests.join(";"))
        .output()
        .unwrap_or_else(|e| panic!("cannot run {mfb}: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        panic!(
            "the subject exited {:?}\nstdout:\n{stdout}\nstderr:\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
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

struct Report {
    ok: usize,
    failures: Vec<String>,
}

impl Report {
    fn new() -> Self {
        Report {
            ok: 0,
            failures: Vec::new(),
        }
    }

    fn check(&mut self, what: &str, pass: bool, detail: impl FnOnce() -> String) {
        if pass {
            self.ok += 1;
            println!("OK   {what}");
        } else {
            let d = detail();
            println!("FAIL {what}\n  {d}");
            self.failures.push(format!("{what}: {d}"));
        }
    }

    fn eq(&mut self, what: &str, mine: &[u8], theirs: &[u8]) {
        self.check(what, mine == theirs, || {
            format!("mfb:       {}\n  reference: {}", hex(mine), hex(theirs))
        });
    }
}

/// `gen <label> <priv> <pub>` — a `genfail` is a finding, not a panic.
fn parse_gen(line: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.first() != Some(&"gen") {
        return None;
    }
    Some((unhex(f[2]), unhex(f[3])))
}

fn parse_tail(kind: &str, line: &str) -> Option<Vec<u8>> {
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.first() != Some(&kind) {
        return None;
    }
    Some(unhex(f[f.len() - 1]))
}

fn parse_verify(line: &str) -> Option<bool> {
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.first() != Some(&"verify") {
        return None;
    }
    match f[2] {
        "TRUE" => Some(true),
        "FALSE" => Some(false),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The reference implementations. None of these is written here.
// ---------------------------------------------------------------------------

fn x25519_base(secret: &[u8]) -> Vec<u8> {
    let mut k = [0u8; 32];
    k.copy_from_slice(secret);
    curve25519_dalek::MontgomeryPoint::mul_base_clamped(k)
        .to_bytes()
        .to_vec()
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

fn x448_of(secret: &[u8], point: &[u8]) -> Vec<u8> {
    let (mut k, mut p) = ([0u8; 56], [0u8; 56]);
    k.copy_from_slice(secret);
    p.copy_from_slice(point);
    x448::x448_unchecked(k, p).to_vec()
}

fn shake256(data: &[u8], n: usize) -> Vec<u8> {
    let mut x = sha3::Shake256::default();
    x.update(data);
    let mut out = vec![0u8; n];
    x.finalize_xof().read(&mut out);
    out
}

fn ed448_private(seed: &[u8]) -> ed448_rust::PrivateKey {
    let mut s = [0u8; 57];
    s.copy_from_slice(seed);
    ed448_rust::PrivateKey::from(&s)
}

// ---------------------------------------------------------------------------
// Per-curve checks
// ---------------------------------------------------------------------------

/// Ed25519 and Ed448: deterministic signatures, so the signature itself is
/// comparable, not merely verifiable.
fn eddsa(mfb: &str, r: &mut Report) {
    // --- Ed25519 ------------------------------------------------------------
    let rust_ed = ed25519_dalek::SigningKey::from_bytes(&[0x11; 32]);
    let rust_pub = rust_ed.verifying_key().to_bytes();
    let rust_sig = rust_ed.sign(MESSAGE).to_bytes();
    let mut bad = rust_sig;
    bad[0] ^= 1;

    let a = ask(
        mfb,
        &[
            "gen,ed25519".into(),
            format!("verify,ed25519,{},{},{}", hex(&rust_pub), hex(MESSAGE), hex(&rust_sig)),
            format!("verify,ed25519,{},{},{}", hex(&rust_pub), hex(MESSAGE), hex(&bad)),
        ],
    );
    let Some((seed, public)) = parse_gen(&a[0]) else {
        r.check("ed25519 generate", false, || a[0].clone());
        return;
    };
    r.check("ed25519 accepts a dalek signature", parse_verify(&a[1]) == Some(true), || a[1].clone());
    r.check("ed25519 rejects a corrupted signature", parse_verify(&a[2]) == Some(false), || {
        format!("{} -- verify is not checking", a[2])
    });

    let derived = ed25519_dalek::SigningKey::from_bytes(&seed.clone().try_into().unwrap())
        .verifying_key()
        .to_bytes();
    r.eq("ed25519 public key matches its seed", &public, &derived);

    let b = ask(mfb, &[format!("sign,ed25519,{},{}", hex(&seed), hex(MESSAGE))]);
    match parse_tail("sign", &b[0]) {
        Some(sig) => {
            let theirs = ed25519_dalek::SigningKey::from_bytes(&seed.try_into().unwrap())
                .sign(MESSAGE)
                .to_bytes();
            r.eq("ed25519 signature is byte-identical to dalek's", &sig, &theirs);
            let vk = ed25519_dalek::VerifyingKey::from_bytes(&public.try_into().unwrap());
            r.check("dalek verifies mfb's ed25519 signature", {
                vk.is_ok()
                    && sig.len() == 64
                    && vk.unwrap()
                        .verify(
                            MESSAGE,
                            &ed25519_dalek::Signature::from_bytes(&sig.try_into().unwrap()),
                        )
                        .is_ok()
            }, || "dalek rejected it".into());
        }
        None => r.check("ed25519 sign", false, || b[0].clone()),
    }

    // --- Ed448 --------------------------------------------------------------
    let rust448 = ed448_private(&[0x22; 57]);
    let rust448_pub = ed448_rust::PublicKey::from(&rust448).as_byte();
    let rust448_sig = rust448.sign(MESSAGE, None).expect("ed448 sign");
    let mut bad448 = rust448_sig;
    bad448[0] ^= 1;

    let a = ask(
        mfb,
        &[
            "gen,ed448".into(),
            format!("verify,ed448,{},{},{}", hex(&rust448_pub), hex(MESSAGE), hex(&rust448_sig)),
            format!("verify,ed448,{},{},{}", hex(&rust448_pub), hex(MESSAGE), hex(&bad448)),
        ],
    );
    let Some((seed, public)) = parse_gen(&a[0]) else {
        r.check("ed448 generate", false, || a[0].clone());
        return;
    };
    r.check("ed448 accepts an ed448-rust signature", parse_verify(&a[1]) == Some(true), || a[1].clone());
    r.check("ed448 rejects a corrupted signature", parse_verify(&a[2]) == Some(false), || {
        format!("{} -- verify is not checking", a[2])
    });

    if seed.len() == 57 {
        let derived = ed448_rust::PublicKey::from(&ed448_private(&seed)).as_byte();
        r.eq("ed448 public key matches its seed", &public, &derived);

        let b = ask(mfb, &[format!("sign,ed448,{},{}", hex(&seed), hex(MESSAGE))]);
        match parse_tail("sign", &b[0]) {
            Some(sig) => {
                let theirs = ed448_private(&seed).sign(MESSAGE, None).expect("ed448 sign");
                r.eq("ed448 signature is byte-identical to ed448-rust's", &sig, &theirs);
                // TRAP: ed448-rust has NO constructor from encoded public-key
                // bytes. Every `From`/`TryFrom` over a byte array runs
                // `BigInt::from_bytes_le` and then a base-point scalar
                // multiplication -- it treats the bytes as a SECRET SCALAR, and
                // `From<BigInt>` is even marked "internal use only". Handing it
                // MFB's public key silently builds a DIFFERENT key, and the
                // perfectly good signature below then "fails to verify".
                //
                // Derive the verifier from the seed instead. That is not a
                // weaker claim: `ed448 public key matches its seed` above already
                // pins `as_byte()` of this very object to the public key MFB
                // reported, so the two checks together say what one construction
                // from MFB's bytes would have said.
                let verifier = ed448_rust::PublicKey::from(&ed448_private(&seed));
                r.check(
                    "ed448-rust verifies mfb's signature",
                    verifier.verify(MESSAGE, &sig, None).is_ok(),
                    || "ed448-rust rejected it".into(),
                );
            }
            None => r.check("ed448 sign", false, || b[0].clone()),
        }
    } else {
        r.check("ed448 private key is a 57-byte seed", false, || {
            format!("got {} bytes", seed.len())
        });
    }
}

/// X25519 and X448: the property is that both sides reach the SAME secret from
/// opposite halves of the two key pairs.
fn ecdh(mfb: &str, r: &mut Report) {
    let peer25519_secret = [0x33u8; 32];
    let peer25519_public = x25519_base(&peer25519_secret);
    let peer448_secret = [0x44u8; 56];
    let peer448_public = x448_of(&peer448_secret, &X448_BASE);

    let a = ask(mfb, &["gen,x25519".into(), "gen,x448".into()]);
    let (Some((s25, p25)), Some((s448, p448))) = (parse_gen(&a[0]), parse_gen(&a[1])) else {
        r.check("x25519/x448 generate", false, || format!("{} / {}", a[0], a[1]));
        return;
    };
    r.eq("x25519 public key is its private key's base multiple", &p25, &x25519_base(&s25));
    r.eq("x448 public key is its private key's base multiple", &p448, &x448_of(&s448, &X448_BASE));

    let b = ask(
        mfb,
        &[
            format!("exchange,x25519,{},{}", hex(&s25), hex(&peer25519_public)),
            format!("exchange,x448,{},{}", hex(&s448), hex(&peer448_public)),
        ],
    );
    match parse_tail("exchange", &b[0]) {
        Some(secret) => {
            r.eq("x25519 shared secret agrees with dalek", &secret, &x25519(&peer25519_secret, &p25));
            r.check("x25519 secret is not all zero", secret != vec![0u8; 32], || {
                "the exchange degenerated".into()
            });
        }
        None => r.check("x25519 exchange", false, || b[0].clone()),
    }
    match parse_tail("exchange", &b[1]) {
        Some(secret) => {
            r.eq("x448 shared secret agrees with the x448 crate", &secret, &x448_of(&peer448_secret, &p448));
            r.check("x448 secret is not all zero", secret != vec![0u8; 56], || {
                "the exchange degenerated".into()
            });
        }
        None => r.check("x448 exchange", false, || b[1].clone()),
    }
}

/// Both `KeyConvert` directions, with each half re-derived independently, and
/// then the converted pair put to work in a real exchange -- a conversion that
/// produced well-formed nonsense would pass a shape check.
fn convert(mfb: &str, r: &mut Report) {
    let a = ask(mfb, &["gen,ed25519".into(), "gen,ed448".into()]);
    let (Some((seed25, pub25)), Some((seed448, pub448))) = (parse_gen(&a[0]), parse_gen(&a[1]))
    else {
        r.check("convert: generate the source pairs", false, || {
            format!("{} / {}", a[0], a[1])
        });
        return;
    };

    let b = ask(
        mfb,
        &[
            format!("convert,ed25519-x25519,{},{}", hex(&seed25), hex(&pub25)),
            format!("convert,ed448-x448,{},{}", hex(&seed448), hex(&pub448)),
        ],
    );

    // Ed25519 -> X25519: clamp(SHA-512(seed)[0..32]), and the birational map of
    // the Ed25519 public key.
    let f: Vec<&str> = b[0].split_whitespace().collect();
    if f.first() == Some(&"convert") {
        let (secret, public) = (unhex(f[2]), unhex(f[3]));
        let mut want = [0u8; 32];
        want.copy_from_slice(&Sha512::digest(&seed25)[..32]);
        want[0] &= 248;
        want[31] &= 127;
        want[31] |= 64;
        r.eq("ed25519->x25519 private is clamp(SHA-512(seed)[0..32])", &secret, &want);

        let mapped = curve25519_dalek::edwards::CompressedEdwardsY(pub25.clone().try_into().unwrap())
            .decompress()
            .map(|p| p.to_montgomery().to_bytes().to_vec());
        match mapped {
            Some(m) => r.eq("ed25519->x25519 public is the birational map", &public, &m),
            None => r.check("ed25519 public key is a curve point", false, || hex(&pub25)),
        }

        let peer = [0x55u8; 32];
        let c = ask(mfb, &[format!("exchange,x25519,{},{}", hex(&secret), hex(&x25519_base(&peer)))]);
        match parse_tail("exchange", &c[0]) {
            Some(s) => r.eq("the converted x25519 pair works in an exchange", &s, &x25519(&peer, &public)),
            None => r.check("converted x25519 exchange", false, || c[0].clone()),
        }
    } else {
        r.check("ed25519->x25519 convert", false, || b[0].clone());
    }

    // Ed448 -> X448: SHAKE256(seed)[0..56] for the private half. The public half
    // is checked as the base multiple of that private half, which is the same
    // claim the RFC 7748 s4.2 map makes for a matching pair.
    let f: Vec<&str> = b[1].split_whitespace().collect();
    if f.first() == Some(&"convert") {
        let (secret, public) = (unhex(f[2]), unhex(f[3]));
        r.eq("ed448->x448 private is SHAKE256(seed)[0..56]", &secret, &shake256(&seed448, 56));
        r.eq("ed448->x448 public is its private key's base multiple", &public, &x448_of(&secret, &X448_BASE));

        let peer = [0x66u8; 56];
        let c = ask(mfb, &[format!("exchange,x448,{},{}", hex(&secret), hex(&x448_of(&peer, &X448_BASE)))]);
        match parse_tail("exchange", &c[0]) {
            Some(s) => r.eq("the converted x448 pair works in an exchange", &s, &x448_of(&peer, &public)),
            None => r.check("converted x448 exchange", false, || c[0].clone()),
        }
    } else {
        r.check("ed448->x448 convert", false, || b[1].clone());
    }
}

/// The NIST curves bind the platform key API, so what is under test is the
/// ENCODING contract: `04‖X‖Y` public keys, `04‖X‖Y‖d` private keys, and ASN.1
/// DER signatures. ECDSA signing is randomized, so there is nothing to compare
/// byte for byte -- only the round trip, in both directions.
///
/// The scalar check is the one the CI test cannot do for these curves: split `d`
/// off the private key and confirm `d·G` is the reported public key.
macro_rules! nist_curve {
    ($fn_name:ident, $krate:ident, $label:literal, $publen:expr, $scalarlen:expr) => {
        fn $fn_name(mfb: &str, r: &mut Report) {
            use $krate::ecdsa::{Signature, SigningKey, VerifyingKey};

            // A fixed scalar, so this side is reproducible. The leading byte is
            // zeroed because a scalar must be below the curve order: 0x77
            // repeated is fine for P-256 and P-384, but for P-521 the order is
            // just under 2^521 while the encoding is 66 bytes (528 bits), so a
            // full-width pattern overflows it and `from_slice` rejects it.
            let mut scalar = [0x77u8; $scalarlen];
            scalar[0] = 0x00;
            let rust_key = SigningKey::from_slice(&scalar).expect("scalar below the curve order");
            let rust_pub = VerifyingKey::from(&rust_key)
                .to_encoded_point(false)
                .as_bytes()
                .to_vec();
            let sig: Signature = rust_key.sign(MESSAGE);
            let der = sig.to_der().as_bytes().to_vec();
            let mut bad = der.clone();
            let last = bad.len() - 1;
            bad[last] ^= 1;

            let a = ask(
                mfb,
                &[
                    format!("gen,{}", $label),
                    format!("verify,{},{},{},{}", $label, hex(&rust_pub), hex(MESSAGE), hex(&der)),
                    format!("verify,{},{},{},{}", $label, hex(&rust_pub), hex(MESSAGE), hex(&bad)),
                ],
            );
            let Some((private, public)) = parse_gen(&a[0]) else {
                r.check(concat!($label, " generate"), false, || a[0].clone());
                return;
            };
            r.check(concat!($label, " accepts a RustCrypto signature"),
                parse_verify(&a[1]) == Some(true), || a[1].clone());
            r.check(concat!($label, " rejects a corrupted signature"),
                parse_verify(&a[2]) == Some(false), || format!("{} -- verify is not checking", a[2]));

            r.check(concat!($label, " public key is an uncompressed point"),
                public.len() == $publen && public.first() == Some(&0x04), || {
                    format!("{} byte(s), first = {:?}", public.len(), public.first())
                });
            r.check(concat!($label, " private key carries its public key as a prefix"),
                private.len() == $publen + $scalarlen && private[..$publen] == public[..], || {
                    format!("private is {} byte(s)", private.len())
                });

            // d·G must be the reported public key.
            if private.len() == $publen + $scalarlen {
                let scalar = &private[$publen..];
                match SigningKey::from_slice(scalar) {
                    Ok(k) => {
                        let derived = VerifyingKey::from(&k).to_encoded_point(false).as_bytes().to_vec();
                        r.eq(concat!($label, " public key is d*G"), &public, &derived);
                    }
                    Err(e) => r.check(concat!($label, " private scalar is valid"), false, || e.to_string()),
                }
            }

            let b = ask(mfb, &[format!("sign,{},{},{}", $label, hex(&private), hex(MESSAGE))]);
            match parse_tail("sign", &b[0]) {
                Some(mfb_sig) => {
                    let vk = VerifyingKey::from_sec1_bytes(&public);
                    let parsed = Signature::from_der(&mfb_sig);
                    r.check(concat!($label, ": RustCrypto verifies mfb's DER signature"),
                        matches!((&vk, &parsed), (Ok(_), Ok(_)))
                            && vk.as_ref().unwrap().verify(MESSAGE, parsed.as_ref().unwrap()).is_ok(),
                        || format!("signature was {} byte(s): {}", mfb_sig.len(), hex(&mfb_sig)));
                    // ... and must NOT verify over a different message, or
                    // "verified" would mean nothing.
                    if let (Ok(vk), Ok(p)) = (vk, parsed) {
                        r.check(concat!($label, ": that signature does not verify another message"),
                            vk.verify(b"a different message", &p).is_err(),
                            || "it verified the wrong message".into());
                    }
                }
                None => r.check(concat!($label, " sign"), false, || b[0].clone()),
            }
        }
    };
}

nist_curve!(p256_checks, p256, "p256", 65, 32);
nist_curve!(p384_checks, p384, "p384", 97, 48);
nist_curve!(p521_checks, p521, "p521", 133, 66);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(mfb) = args.get(1) else {
        eprintln!("usage: keysref <path-to-the-built-mfb-program>");
        std::process::exit(2);
    };

    let mut r = Report::new();
    eddsa(mfb, &mut r);
    ecdh(mfb, &mut r);
    convert(mfb, &mut r);
    p256_checks(mfb, &mut r);
    p384_checks(mfb, &mut r);
    p521_checks(mfb, &mut r);

    let ran = r.ok + r.failures.len();
    println!("crypto keys mfb-vs-rust: {ran} check(s), {} failure(s)", r.failures.len());
    // A run that checked nothing must not read as green.
    if ran == 0 {
        eprintln!("ran NOTHING -- that is a harness bug, not a pass");
        std::process::exit(2);
    }
    if !r.failures.is_empty() {
        std::process::exit(1);
    }
}
