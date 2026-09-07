//! Interop proof for `crypto::seal` / `crypto::open`: the AEAD wire values are
//! AES-256-GCM (NIST SP 800-38D) and ChaCha20-Poly1305 (RFC 8439), and they
//! interoperate in BOTH directions with `ring`, which shares no code with the
//! MFBASIC cores.
//!
//! Sealing is deterministic given (key, nonce, data, aad), so this can do
//! something the public-key tests cannot: compare the ciphertext and tag byte
//! for byte, not merely round-trip them. Three claims are checked, and the third
//! is the one a round trip alone would miss:
//!
//! 1. **MFB seals, ring re-seals** — identical ciphertext and tag. A round trip
//!    inside one implementation would pass even if both halves shared a wrong
//!    constant; re-deriving the bytes elsewhere would not.
//! 2. **ring seals, MFB opens** — ring picks a random key, nonce, message and
//!    aad, and the MFB program recovers the plaintext. This is the direction
//!    that proves MFB reads a foreign box, not just its own.
//! 3. **ring seals a TAMPERED box, MFB refuses it** — a decryptor that ignored
//!    the tag would pass (1) and (2) perfectly and still be catastrophically
//!    broken, so the fail-closed path is checked against a foreign forgery
//!    rather than only against a self-made one.
//!
//! Lives in `tests/` rather than `tools/oracles/crypto/` because `ring` is
//! already a dev-dependency: no new compiled code, and it runs on every
//! `cargo test`. See `.ai/testing-gates.md` on where an oracle lives.

mod common;
use common::{build_project, run_capture_with_env, temp_project};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, CHACHA20_POLY1305};
use ring::rand::{SecureRandom, SystemRandom};

/// Counted from the source below, not from its output: a count taken from the
/// producer is true by construction, so a program that stopped after four cases
/// would otherwise report "4 of 4 agreed".
const EXPECTED_SEAL_CASES: usize = 8 * 2; // 8 (key, nonce, data, aad) sets x 2 ciphers
const EXPECTED_OPEN_OK: usize = 6; // ring-sealed boxes MFB must open
const EXPECTED_OPEN_FAIL: usize = 4; // tampered boxes MFB must refuse

/// `crypto::ErrAuthenticationFailed`, the code `open` must raise on a bad tag.
const ERR_AUTHENTICATION_FAILED: &str = "77050016";

/// Phase A (always) seals a spread and prints it. Phase B (only when
/// `MFB_AEAD_JOB` is set) opens boxes this program did not produce.
///
/// The job is `record;record;…`, each `cipher,key,nonce,ciphertext,tag,aad`
/// in hex with "-" for empty — an empty field would otherwise vanish when the
/// record is split.
const SOURCE: &str = r#"
IMPORT collections
IMPORT crypto
IMPORT encoding
IMPORT io
IMPORT os
IMPORT strings

FUNC pattern(n AS Integer) AS List OF Byte
  MUT out AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < n
    out = collections::append(out, toByte((i * 7 + 3) MOD 251))
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

FUNC decodeField(value AS String) AS List OF Byte
  MUT out AS List OF Byte = []
  IF value <> "-" THEN
    out = encoding::hexDecode(value)
  END IF
  RETURN out
END FUNC

SUB sealOne(label AS String, c AS crypto::SymmetricCipher, key AS List OF Byte, nonce AS List OF Byte, data AS List OF Byte, aad AS List OF Byte)
  LET box AS crypto::Sealed = crypto::seal(c, key, nonce, data, aad)
  MUT line AS String = "case seal " & label & " " & hexField(key) & " " & hexField(nonce)
  line = line & " " & hexField(data) & " " & hexField(aad)
  line = line & " " & hexField(box.ciphertext) & " " & hexField(box.tag)
  io::print(line)
END SUB

SUB sealBoth(key AS List OF Byte, nonce AS List OF Byte, data AS List OF Byte, aad AS List OF Byte)
  sealOne("aes256gcm", crypto::SymmetricCipher.AES256GCM, key, nonce, data, aad)
  sealOne("chacha20poly1305", crypto::SymmetricCipher.CHACHA20POLY1305, key, nonce, data, aad)
END SUB

SUB openOne(spec AS String)
  LET f AS List OF String = strings::split(spec, ",")
  MUT c AS crypto::SymmetricCipher = crypto::SymmetricCipher.CHACHA20POLY1305
  IF collections::get(f, 0) = "aes256gcm" THEN
    c = crypto::SymmetricCipher.AES256GCM
  END IF
  LET key AS List OF Byte = decodeField(collections::get(f, 1))
  LET nonce AS List OF Byte = decodeField(collections::get(f, 2))
  LET ct AS List OF Byte = decodeField(collections::get(f, 3))
  LET tag AS List OF Byte = decodeField(collections::get(f, 4))
  LET aad AS List OF Byte = decodeField(collections::get(f, 5))
  LET plain AS List OF Byte = crypto::open(c, key, nonce, ct, tag, aad) TRAP(e)
    io::print("openfail " & toString(e.code))
    EXIT SUB
  END TRAP
  io::print("opened " & hexField(plain))
END SUB

SUB main()
  ' Sealed lengths straddle the 16-byte AES block: an empty message (tag over the
  ' aad alone), one byte, and either side of one and four blocks. The aad
  ' alternates empty and not, because aad is authenticated but not encrypted and
  ' a core that dropped it would still produce a plausible ciphertext.
  sealBoth(pattern(32), pattern(12), pattern(0), pattern(0))
  sealBoth(pattern(32), pattern(12), pattern(1), pattern(0))
  sealBoth(pattern(32), pattern(12), pattern(15), pattern(3))
  sealBoth(pattern(32), pattern(12), pattern(16), pattern(16))
  sealBoth(pattern(32), pattern(12), pattern(17), pattern(0))
  sealBoth(pattern(32), pattern(12), pattern(64), pattern(20))
  sealBoth(pattern(32), pattern(12), pattern(65), pattern(0))
  sealBoth(pattern(32), pattern(12), pattern(200), pattern(100))

  ' Phase B: open whatever the caller sealed elsewhere.
  LET job AS String = os::getEnvOr("MFB_AEAD_JOB", "")
  IF job <> "" THEN
    LET records AS List OF String = strings::split(job, ";")
    FOR EACH one IN records
      IF one <> "" THEN
        openOne(one)
      END IF
    NEXT
  END IF
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

/// "-" for empty, matching what the MFB side writes and reads.
fn field(b: &[u8]) -> String {
    if b.is_empty() {
        "-".to_string()
    } else {
        hex(b)
    }
}

fn ring_key(cipher: &str, key: &[u8]) -> LessSafeKey {
    let alg = match cipher {
        "aes256gcm" => &AES_256_GCM,
        "chacha20poly1305" => &CHACHA20_POLY1305,
        other => panic!("unknown cipher {other}"),
    };
    LessSafeKey::new(UnboundKey::new(alg, key).expect("ring key"))
}

/// (ciphertext, tag) for the same inputs the MFB side was given.
fn ring_seal(
    cipher: &str,
    key: &[u8],
    nonce: &[u8],
    data: &[u8],
    aad: &[u8],
) -> (Vec<u8>, Vec<u8>) {
    let mut buf = data.to_vec();
    let tag = ring_key(cipher, key)
        .seal_in_place_separate_tag(
            Nonce::try_assume_unique_for_key(nonce).expect("12-byte nonce"),
            Aad::from(aad),
            &mut buf,
        )
        .expect("ring seal");
    (buf, tag.as_ref().to_vec())
}

fn ring_open(
    cipher: &str,
    key: &[u8],
    nonce: &[u8],
    ct: &[u8],
    tag: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, ()> {
    let mut buf = ct.to_vec();
    buf.extend_from_slice(tag);
    ring_key(cipher, key)
        .open_in_place(
            Nonce::try_assume_unique_for_key(nonce).expect("12-byte nonce"),
            Aad::from(aad),
            &mut buf,
        )
        .map(|plain| plain.to_vec())
        .map_err(|_| ())
}

fn random(rng: &SystemRandom, n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    rng.fill(&mut v).expect("random bytes");
    v
}

#[test]
fn aead_boxes_interoperate_with_ring_both_ways() {
    let project = temp_project("crypto_aead_interop", SOURCE);
    let exe = build_project(&project);
    let rng = SystemRandom::new();

    // ---- direction 2 and 3: build the boxes MFB will be asked to open -------
    // Done before the single run so both directions are exercised in one build.
    struct Job {
        cipher: &'static str,
        key: Vec<u8>,
        nonce: Vec<u8>,
        ct: Vec<u8>,
        tag: Vec<u8>,
        aad: Vec<u8>,
        plaintext: Vec<u8>,
        tampered: bool,
    }
    let mut jobs: Vec<Job> = Vec::new();
    for cipher in ["aes256gcm", "chacha20poly1305"] {
        for (msg_len, aad_len) in [(0usize, 0usize), (23, 0), (140, 17)] {
            let (key, nonce) = (random(&rng, 32), random(&rng, 12));
            let (msg, aad) = (random(&rng, msg_len), random(&rng, aad_len));
            let (ct, tag) = ring_seal(cipher, &key, &nonce, &msg, &aad);
            jobs.push(Job {
                cipher,
                key,
                nonce,
                ct,
                tag,
                aad,
                plaintext: msg,
                tampered: false,
            });
        }
        // A flipped tag bit and a flipped ciphertext bit. Both must be refused;
        // a decryptor that ignored the tag passes every honest case above.
        for corrupt_tag in [true, false] {
            let (key, nonce) = (random(&rng, 32), random(&rng, 12));
            let (msg, aad) = (random(&rng, 48), random(&rng, 8));
            let (mut ct, mut tag) = ring_seal(cipher, &key, &nonce, &msg, &aad);
            if corrupt_tag {
                tag[0] ^= 1;
            } else {
                ct[0] ^= 0x80;
            }
            jobs.push(Job {
                cipher,
                key,
                nonce,
                ct,
                tag,
                aad,
                plaintext: msg,
                tampered: true,
            });
        }
    }
    let job_env = jobs
        .iter()
        .map(|j| {
            format!(
                "{},{},{},{},{},{}",
                j.cipher,
                field(&j.key),
                field(&j.nonce),
                field(&j.ct),
                field(&j.tag),
                field(&j.aad)
            )
        })
        .collect::<Vec<_>>()
        .join(";");

    let (code, stdout, stderr) = run_capture_with_env(&exe, &[("MFB_AEAD_JOB", job_env)]);
    assert_eq!(
        code, 0,
        "program failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    // ---- direction 1: MFB sealed, ring re-seals and also opens --------------
    let mut seal_cases = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("case seal ") else {
            continue;
        };
        let f: Vec<&str> = rest.split_whitespace().collect();
        assert_eq!(f.len(), 7, "malformed seal line: {line}");
        let (cipher, key, nonce) = (f[0], unhex(f[1]), unhex(f[2]));
        let (data, aad) = (unhex(f[3]), unhex(f[4]));
        let (mfb_ct, mfb_tag) = (unhex(f[5]), unhex(f[6]));
        seal_cases += 1;

        let (ct, tag) = ring_seal(cipher, &key, &nonce, &data, &aad);
        if ct != mfb_ct || tag != mfb_tag {
            failures.push(format!(
                "{cipher} seal of {} byte(s) with {} byte(s) aad\n  mfb:  {} / {}\n  ring: {} / {}",
                data.len(),
                aad.len(),
                hex(&mfb_ct),
                hex(&mfb_tag),
                hex(&ct),
                hex(&tag)
            ));
            continue;
        }
        // The bytes match, so ring must also be able to OPEN what MFB sealed --
        // identical output and a verifying tag are separate claims.
        match ring_open(cipher, &key, &nonce, &mfb_ct, &mfb_tag, &aad) {
            Ok(plain) if plain == data => {}
            Ok(plain) => failures.push(format!(
                "{cipher}: ring opened MFB's box to the wrong plaintext: {} != {}",
                hex(&plain),
                hex(&data)
            )),
            Err(()) => failures.push(format!(
                "{cipher}: ring refused MFB's box for {} byte(s)",
                data.len()
            )),
        }
    }

    // ---- directions 2 and 3: what MFB made of ring's boxes ------------------
    let results: Vec<&str> = stdout
        .lines()
        .filter(|l| l.starts_with("opened ") || l.starts_with("openfail "))
        .collect();
    assert_eq!(
        results.len(),
        jobs.len(),
        "MFB answered {} of {} open job(s)\nstdout:\n{stdout}",
        results.len(),
        jobs.len()
    );

    let (mut opened_ok, mut refused) = (0usize, 0usize);
    for (job, line) in jobs.iter().zip(&results) {
        match (job.tampered, line.strip_prefix("opened ")) {
            (false, Some(got)) => {
                if got == field(&job.plaintext) {
                    opened_ok += 1;
                } else {
                    failures.push(format!(
                        "{}: MFB opened a ring box to the wrong plaintext\n  mfb:  {got}\n  ring: {}",
                        job.cipher,
                        field(&job.plaintext)
                    ));
                }
            }
            (false, None) => failures.push(format!(
                "{}: MFB refused an honest ring box ({line})",
                job.cipher
            )),
            (true, Some(_)) => failures.push(format!(
                "{}: MFB ACCEPTED a tampered ring box -- the tag is not being checked",
                job.cipher
            )),
            (true, None) => {
                let code = line.strip_prefix("openfail ").unwrap_or("");
                if code == ERR_AUTHENTICATION_FAILED {
                    refused += 1;
                } else {
                    failures.push(format!(
                        "{}: MFB refused a tampered box with {code}, expected \
                         ErrAuthenticationFailed ({ERR_AUTHENTICATION_FAILED})",
                        job.cipher
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} interop failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
    // Counts last: they catch a short run, not a wrong answer.
    assert_eq!(
        seal_cases, EXPECTED_SEAL_CASES,
        "sealed {seal_cases} case(s) but the source declares {EXPECTED_SEAL_CASES}"
    );
    assert_eq!(
        opened_ok, EXPECTED_OPEN_OK,
        "MFB opened {opened_ok} ring box(es)"
    );
    assert_eq!(
        refused, EXPECTED_OPEN_FAIL,
        "MFB refused {refused} tampered box(es)"
    );
}
