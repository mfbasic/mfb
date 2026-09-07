//! Interop proof for `crypto::hmac`, `crypto::hkdf` and `crypto::pbkdf2`: every
//! value the MFBASIC cores produce is re-derived here by `ring`, an
//! implementation that shares no code with them.
//!
//! These three are pure functions -- bytes in, bytes out, no nonce and no key
//! generation -- so the check is a byte comparison rather than a round trip.
//! What makes it a check rather than a restatement is that the expected values
//! are computed, not committed: a golden file full of digests this package
//! produced would ratify the package, and would keep passing if the core and
//! the golden were both wrong.
//!
//! This lives in `tests/` rather than in `tools/oracles/crypto/` because it is
//! affordable here: `ring` is already a dev-dependency (added for
//! `rt_crypto_hpke_interop`), so this costs no new compiled code and runs on
//! every `cargo test`. Per `.ai/testing-gates.md` that is the best of the three
//! oracle homes, and the only one CI executes.
//!
//! SHA-224 uses RustCrypto `hmac` + `sha2` (both already in the lockfile too)
//! because `ring` has no SHA-224. The SHA-3 selectors of these three members
//! have no `ring` equivalent at all and are covered by
//! `tools/oracles/crypto/mac-kdf/`, which can pin the extra crates.

#[path = "../common/mod.rs"]
mod common;
use common::{build_project, run_capture_with_env, temp_project};
use hmac::{Hmac, Mac};
use ring::hkdf::{Salt, HKDF_SHA1_FOR_LEGACY_USE_ONLY, HKDF_SHA256, HKDF_SHA384, HKDF_SHA512};
use ring::pbkdf2::{PBKDF2_HMAC_SHA1, PBKDF2_HMAC_SHA256, PBKDF2_HMAC_SHA384, PBKDF2_HMAC_SHA512};
use sha2::Sha224;
use std::num::NonZeroU32;

/// The number of `case` lines the MFB program below must emit, counted from its
/// source rather than from its output. A count taken from the producer is true
/// by construction: if the program died after 20 cases, "20 of 20 agreed" would
/// pass. Keep it in step with `main()`:
///   9 hmac inputs x 5 hashes + 4 hkdf inputs x 4 hashes + 4 pbkdf2 inputs x 4.
const EXPECTED_CASES: usize = 9 * 5 + 4 * 4 + 4 * 4;

/// Emits one `case` line per (member, hash, input). The line carries the inputs
/// it used, so the Rust side re-derives from those rather than from a table
/// duplicated here -- the two cannot drift onto different bytes and still agree.
/// An empty field is written "-", since an empty token would shift the rest of
/// the line left when it is split on spaces.
const SOURCE: &str = r#"
IMPORT collections
IMPORT crypto
IMPORT encoding
IMPORT io
IMPORT strings

FUNC pattern(n AS Integer) AS List OF Byte
  MUT out AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < n
    out = collections::append(out, toByte(i MOD 251))
    i = i + 1
  END WHILE
  RETURN out
END FUNC

FUNC hexField(value AS List OF Byte) AS String
  LET encoded AS String = encoding::hexEncode(value)
  IF encoded = "" THEN
    RETURN "-"
  END IF
  RETURN encoded
END FUNC

SUB emit(kind AS String, algo AS String, params AS String, out AS List OF Byte)
  io::print("case " & kind & " " & algo & " " & params & " " & encoding::hexEncode(out))
END SUB

SUB hmacAll(key AS List OF Byte, data AS List OF Byte)
  LET p AS String = hexField(key) & " " & hexField(data)
  emit("hmac", "sha1", p, crypto::hmac(crypto::Hash.SHA1, key, data))
  emit("hmac", "sha2-224", p, crypto::hmac(crypto::Hash.SHA2_224, key, data))
  emit("hmac", "sha2-256", p, crypto::hmac(crypto::Hash.SHA2_256, key, data))
  emit("hmac", "sha2-384", p, crypto::hmac(crypto::Hash.SHA2_384, key, data))
  emit("hmac", "sha2-512", p, crypto::hmac(crypto::Hash.SHA2_512, key, data))
END SUB

SUB hkdfAll(ikm AS List OF Byte, salt AS List OF Byte, info AS List OF Byte, length AS Integer)
  MUT p AS String = hexField(ikm) & " " & hexField(salt)
  p = p & " " & hexField(info) & " " & toString(length)
  emit("hkdf", "sha1", p, crypto::hkdf(crypto::Hash.SHA1, ikm, salt, info, length))
  emit("hkdf", "sha2-256", p, crypto::hkdf(crypto::Hash.SHA2_256, ikm, salt, info, length))
  emit("hkdf", "sha2-384", p, crypto::hkdf(crypto::Hash.SHA2_384, ikm, salt, info, length))
  emit("hkdf", "sha2-512", p, crypto::hkdf(crypto::Hash.SHA2_512, ikm, salt, info, length))
END SUB

SUB pbkdf2All(password AS List OF Byte, salt AS List OF Byte, iterations AS Integer, length AS Integer)
  MUT p AS String = hexField(password) & " " & hexField(salt)
  p = p & " " & toString(iterations) & " " & toString(length)
  emit("pbkdf2", "sha1", p, crypto::pbkdf2(crypto::Hash.SHA1, password, salt, iterations, length))
  emit("pbkdf2", "sha2-256", p, crypto::pbkdf2(crypto::Hash.SHA2_256, password, salt, iterations, length))
  emit("pbkdf2", "sha2-384", p, crypto::pbkdf2(crypto::Hash.SHA2_384, password, salt, iterations, length))
  emit("pbkdf2", "sha2-512", p, crypto::pbkdf2(crypto::Hash.SHA2_512, password, salt, iterations, length))
END SUB

SUB main()
  ' HMAC key lengths straddle the block size (64 bytes for SHA-1/224/256, 128 for
  ' SHA-384/512). A key longer than the block is HASHED first, and that reduction
  ' is the step most likely to be wrong -- so probe each block boundary either
  ' side, and cross the two families' boundaries with one set of keys.
  hmacAll(pattern(0), strings::toBytes("message"))
  hmacAll(pattern(1), strings::toBytes("message"))
  hmacAll(pattern(32), pattern(0))
  hmacAll(pattern(63), pattern(7))
  hmacAll(pattern(64), pattern(64))
  hmacAll(pattern(65), pattern(200))
  hmacAll(pattern(127), pattern(1))
  hmacAll(pattern(128), pattern(128))
  hmacAll(pattern(129), pattern(3))

  ' HKDF: an empty salt is not the same as no salt (RFC 5869 substitutes
  ' HashLen zero bytes), and an output length that is not a multiple of the hash
  ' length exercises the final truncated block of Expand.
  hkdfAll(pattern(22), pattern(13), pattern(10), 42)
  hkdfAll(pattern(32), pattern(0), pattern(0), 32)
  hkdfAll(pattern(1), pattern(64), pattern(80), 100)
  hkdfAll(pattern(80), pattern(1), pattern(1), 1)

  ' PBKDF2: iteration counts 1 and 2 separate "did the loop run" from "did it run
  ' the right number of times", and a length past one hash block exercises the
  ' multi-block concatenation.
  pbkdf2All(strings::toBytes("password"), strings::toBytes("salt"), 1, 20)
  pbkdf2All(strings::toBytes("password"), strings::toBytes("salt"), 2, 20)
  pbkdf2All(strings::toBytes("passwordPASSWORDpassword"), strings::toBytes("saltSALTsaltSALTsalt"), 10, 25)
  pbkdf2All(pattern(0), pattern(16), 1000, 64)
END SUB
"#;

struct Len(usize);
impl ring::hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

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

fn ring_hmac_alg(algo: &str) -> ring::hmac::Algorithm {
    match algo {
        "sha1" => ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
        "sha2-256" => ring::hmac::HMAC_SHA256,
        "sha2-384" => ring::hmac::HMAC_SHA384,
        "sha2-512" => ring::hmac::HMAC_SHA512,
        other => panic!("no ring HMAC algorithm for {other}"),
    }
}

fn hkdf_alg(algo: &str) -> ring::hkdf::Algorithm {
    match algo {
        "sha1" => HKDF_SHA1_FOR_LEGACY_USE_ONLY,
        "sha2-256" => HKDF_SHA256,
        "sha2-384" => HKDF_SHA384,
        "sha2-512" => HKDF_SHA512,
        other => panic!("no ring HKDF algorithm for {other}"),
    }
}

fn pbkdf2_alg(algo: &str) -> ring::pbkdf2::Algorithm {
    match algo {
        "sha1" => PBKDF2_HMAC_SHA1,
        "sha2-256" => PBKDF2_HMAC_SHA256,
        "sha2-384" => PBKDF2_HMAC_SHA384,
        "sha2-512" => PBKDF2_HMAC_SHA512,
        other => panic!("no ring PBKDF2 algorithm for {other}"),
    }
}

/// The independent answer for one emitted case. `fields` is everything the MFB
/// line carried after the algorithm label, minus the digest.
fn reference(kind: &str, algo: &str, fields: &[&str]) -> Vec<u8> {
    match kind {
        "hmac" => {
            let (key, data) = (unhex(fields[0]), unhex(fields[1]));
            if algo == "sha2-224" {
                // ring has no SHA-224; RustCrypto covers this one selector.
                let mut mac = Hmac::<Sha224>::new_from_slice(&key).expect("hmac key");
                mac.update(&data);
                mac.finalize().into_bytes().to_vec()
            } else {
                let key = ring::hmac::Key::new(ring_hmac_alg(algo), &key);
                ring::hmac::sign(&key, &data).as_ref().to_vec()
            }
        }
        "hkdf" => {
            let (ikm, salt, info) = (unhex(fields[0]), unhex(fields[1]), unhex(fields[2]));
            let len: usize = fields[3].parse().expect("hkdf length");
            let prk = Salt::new(hkdf_alg(algo), &salt).extract(&ikm);
            let mut out = vec![0u8; len];
            prk.expand(&[&info], Len(len))
                .expect("hkdf expand")
                .fill(&mut out)
                .expect("hkdf fill");
            out
        }
        "pbkdf2" => {
            let (password, salt) = (unhex(fields[0]), unhex(fields[1]));
            let iterations: u32 = fields[2].parse().expect("pbkdf2 iterations");
            let len: usize = fields[3].parse().expect("pbkdf2 length");
            let mut out = vec![0u8; len];
            ring::pbkdf2::derive(
                pbkdf2_alg(algo),
                NonZeroU32::new(iterations).expect("nonzero iterations"),
                &salt,
                &password,
                &mut out,
            );
            out
        }
        other => panic!("unknown case kind {other}"),
    }
}

#[test]
fn hmac_hkdf_pbkdf2_match_an_independent_implementation() {
    let project = temp_project("crypto_mac_kdf_interop", SOURCE);
    let exe = build_project(&project);
    let (code, stdout, stderr) = run_capture_with_env(&exe, &[]);
    assert_eq!(
        code, 0,
        "program failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let mut ran = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("case ") else {
            continue;
        };
        let f: Vec<&str> = rest.split_whitespace().collect();
        assert!(f.len() >= 4, "malformed case line: {line}");
        let (kind, algo) = (f[0], f[1]);
        let mine = f[f.len() - 1];
        let theirs = hex(&reference(kind, algo, &f[2..f.len() - 1]));
        ran += 1;
        if mine != theirs {
            failures.push(format!(
                "{kind}/{algo} over [{}]\n     mfb: {mine}\n  {:>6}: {theirs}",
                f[2..f.len() - 1].join(" "),
                "ring"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {ran} case(s) disagreed with the reference:\n{}",
        failures.len(),
        failures.join("\n")
    );
    // Only meaningful AFTER the comparison: it catches a program that stopped
    // early rather than one that produced wrong answers.
    assert_eq!(
        ran, EXPECTED_CASES,
        "ran {ran} case(s) but the source declares {EXPECTED_CASES} -- \
         a short run must not read as a pass"
    );
}
