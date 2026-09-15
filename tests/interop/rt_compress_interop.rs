//! Interop proof for the `compress::` package against implementations that share no
//! code with the MFBASIC one.
//!
//! **CRC-32.** `compress::crc32` is compared with `flate2::Crc` — `crc32fast::Hasher`
//! underneath, because this tree does not enable `flate2`'s `zlib-rs` feature — over a
//! generated corpus: every length 0–17 (each slicing-by-8 tail length, with and without
//! a preceding eight-byte step) and 200 seeded pseudo-random lengths up to 100,000
//! bytes. Each case is checked twice: in one call, and chained across a seeded split
//! point (`crc32(tail, crc32(head))`), so a `running` that was folded wrongly fails even
//! when the one-shot value is right. The `running` range check is probed at both ends.
//!
//! The Rust side generates the bytes and writes them to a job file; the MFB program
//! reads the job and prints one result line per case. Neither side derives the other's
//! inputs from its outputs.
//!
//! **Decoders.** `compress::inflate`, `zlibDecode` and `gzipDecode` are checked against `flate2`'s
//! miniz_oxide backend: every level of all three formats decodes; 80 seeded single-byte flips of
//! zlib and gzip streams each get `flate2`'s own verdict; and the refusal classes of plan-137-B §1
//! are pinned by name. Where each class is covered (this file, the rt-error fixtures under
//! `tests/rt-error/compress/`, or `tools/oracles/compress/probe.sh` + `mutate`, which run offline):
//!
//! | refusal class (plan-137-B §1) | covered by |
//! |---|---|
//! | over-subscribed code set | `dec57_dec58_regressions_are_refused` (literal set); `probe.sh` (distance set) |
//! | incomplete literal/length set | `dec57_dec58_regressions_are_refused` |
//! | distance before the start of output | `dec57_dec58_regressions_are_refused` |
//! | stored `LEN` ≠ `~NLEN` | `decided_behaviours_hold` ("bad NLEN, ignoreChecksum") |
//! | reserved `BTYPE = 11`, invalid length/distance symbols, bad repeats, missing end-of-block | `probe.sh`; `mutate` (0 lenient cases) |
//! | input ending mid-stream | `tampered_checksummed_streams_get_flate2s_verdict`; `mutate` |
//! | zlib `FDICT` | `decided_behaviours_hold`; `compress-zlib-decode-preset-dictionary-invalid` |
//! | zlib `CM`/`CINFO`/`FCHECK` | `probe.sh` (`zlib-cinfo-8`); `mutate` header edits |
//! | gzip magic / `CM` / reserved flags, truncated header or trailer | `decided_behaviours_hold` ("1f 8b then garbage"); `probe.sh` (`gzip-reserved-flag`) |
//! | wrong Adler-32 / CRC-32 / `ISIZE` / `FHCRC` (unless `ignoreChecksum`) | `decided_behaviours_hold`; `compress-zlib-decode-bad-checksum-invalid` |
//! | output past `maxBytes` (`ErrTooLarge`) | `compress-inflate-too-large-invalid`; `tests/runtime/rt_compress_bounds.rs` |
//! | negative `maxBytes` (`ErrInvalidArgument`) | `compress-inflate-max-bytes-negative-invalid` |
//!
//! **Encoders.** `compress::deflate`, `zlibEncode` and `gzipEncode` compress seven payloads (the
//! plan-137-D §1 edge sizes, a long zero run and seeded text) at every level. `flate2` must decode
//! every output back to its payload. Each case is compressed twice in one run and the two outputs
//! must match, and a second run of the program must write byte-identical output.
//!
//! Lives in `tests/` rather than `tools/oracles/compress/` because `flate2` is already a
//! dependency through `image` → `png`: no new compiled code, and it runs on every
//! `cargo test`. See `.ai/testing-gates.md` on where an oracle lives.

#[path = "../common/mod.rs"]
mod common;
use common::{build_project, run_capture_with_env, temp_project};
use flate2::Crc;

/// Counted from the corpus definition below, not from the program's output: a count
/// taken from the producer is true by construction.
const EXPECTED_CRC32_CASES: usize = 18 + 200; // lengths 0..=17, then 200 seeded lengths

/// `ErrInvalidArgument`, the code an out-of-range `running` must raise.
const ERR_INVALID_ARGUMENT: &str = "77050002";

/// Reads the job named by `MFB_COMPRESS_JOB`: a little-endian `u32` case count, then a
/// `(length, split)` pair of `u32`s per case, then every case's bytes back to back.
const SOURCE: &str = r#"
IMPORT collections
IMPORT compress
IMPORT fs
IMPORT io
IMPORT os

FUNC u32At(b AS List OF Byte, at AS Integer) AS Integer
  RETURN toInt(collections::get(b, at)) + 256 * toInt(collections::get(b, at + 1)) + 65536 * toInt(collections::get(b, at + 2)) + 16777216 * toInt(collections::get(b, at + 3))
END FUNC

FUNC probeRunning(running AS Integer) AS String
  LET r AS Integer = compress::crc32([], running) TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
  RETURN "ok " & toString(r)
END FUNC

SUB main()
  RES h AS fs::File = fs::openFile(os::getEnv("MFB_COMPRESS_JOB"), "r")
  LET job AS List OF Byte = fs::readAllBytes(h)
  fs::close(h)
  LET count AS Integer = u32At(job, 0)
  MUT offset AS Integer = 4 + 8 * count
  MUT i AS Integer = 0
  WHILE i < count
    LET n AS Integer = u32At(job, 4 + 8 * i)
    LET split AS Integer = u32At(job, 8 + 8 * i)
    LET data AS List OF Byte = collections::mid(job, offset, n)
    LET whole AS Integer = compress::crc32(data)
    LET head AS Integer = compress::crc32(collections::mid(data, 0, split))
    LET chained AS Integer = compress::crc32(collections::mid(data, split, n - split), head)
    io::print("case " & toString(i) & " " & toString(n) & " " & toString(whole) & " " & toString(chained))
    offset = offset + n
    i = i + 1
  END WHILE
  io::print("running-max " & probeRunning(4294967295))
  io::print("running-over " & probeRunning(4294967296))
  io::print("running-negative " & probeRunning(-1))
END SUB
"#;

/// PCG-style 64-bit LCG (Knuth's MMIX constants); the high bits are the output.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn reference_crc32(bytes: &[u8]) -> u32 {
    let mut crc = Crc::new();
    crc.update(bytes);
    crc.sum()
}

#[test]
fn crc32_matches_crc32fast_over_a_generated_corpus() {
    let mut rng = Lcg(0x0137_A0C3_2C32);
    let mut lengths: Vec<usize> = (0..18).collect();
    for _ in 0..200 {
        lengths.push((rng.next() % 100_001) as usize);
    }
    let cases: Vec<(Vec<u8>, usize)> = lengths
        .into_iter()
        .map(|len| {
            let data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            let split = (rng.next() % (len as u64 + 1)) as usize;
            (data, split)
        })
        .collect();
    assert_eq!(cases.len(), EXPECTED_CRC32_CASES);

    let project = temp_project("rt_compress_crc32_interop", SOURCE);
    let exe = build_project(&project);
    let mut job = Vec::new();
    job.extend((cases.len() as u32).to_le_bytes());
    for (data, split) in &cases {
        job.extend((data.len() as u32).to_le_bytes());
        job.extend((*split as u32).to_le_bytes());
    }
    for (data, _) in &cases {
        job.extend(data);
    }
    let job_path = project.join("job.bin");
    std::fs::write(&job_path, &job).expect("write job file");

    let (code, stdout, stderr) =
        run_capture_with_env(&exe, &[("MFB_COMPRESS_JOB", job_path.display().to_string())]);
    assert_eq!(code, 0, "program failed\nstdout:\n{stdout}\nstderr:\n{stderr}");

    let mut seen = 0;
    let mut failures = Vec::new();
    for line in stdout.lines().filter_map(|l| l.strip_prefix("case ")) {
        let fields: Vec<&str> = line.split(' ').collect();
        let index: usize = fields[0].parse().expect("case index");
        let (data, split) = &cases[index];
        assert_eq!(fields[1], data.len().to_string(), "case {index} read the wrong length");
        let expected = reference_crc32(data).to_string();
        if fields[2] != expected || fields[3] != expected {
            failures.push(format!(
                "case {index}: len={} split={split} whole={} chained={} crc32fast={expected}",
                data.len(),
                fields[2],
                fields[3]
            ));
        }
        seen += 1;
    }
    assert_eq!(seen, EXPECTED_CRC32_CASES, "program stopped early\nstderr:\n{stderr}");
    assert!(failures.is_empty(), "{} disagreement(s):\n{}", failures.len(), failures.join("\n"));

    assert!(stdout.contains("running-max ok 4294967295\n"), "{stdout}");
    assert!(
        stdout.contains(&format!("running-over raised {ERR_INVALID_ARGUMENT}\n")),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("running-negative raised {ERR_INVALID_ARGUMENT}\n")),
        "{stdout}"
    );
}

// ---------------------------------------------------------------------------------------------
// Decoders: `compress::inflate` / `zlibDecode` / `gzipDecode` against `flate2` (miniz_oxide).
// ---------------------------------------------------------------------------------------------

use flate2::read::{MultiGzDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use flate2::Compression;
use std::io::{Read, Write};

/// `ErrInvalidFormat`, the code every malformed-input refusal raises.
const ERR_INVALID_FORMAT: &str = "77050003";

/// Case counts, each derived from the case list it names — never from the program's output.
const EXPECTED_ENCODE_CASES: usize = 3 * 10; // raw / zlib / gzip x levels 0..=9
const EXPECTED_TAMPER_CASES: usize = 2 * 40; // zlib / gzip x 40 seeded single-byte flips
const EXPECTED_REGRESSION_CASES: usize = 4;
const EXPECTED_DECIDED_CASES: usize = 23;

/// Job format bits: the low nibble selects the decoder, `LENIENT` passes `ignoreChecksum := TRUE`.
const RAW: u32 = 0;
const ZLIB: u32 = 1;
const GZIP: u32 = 2;
const LENIENT: u32 = 16;

/// Reads the job named by `MFB_COMPRESS_JOB` — a little-endian `u32` case count, a `(length, aux)`
/// pair per case, then the cases' bytes — and prints `case <i> ok <len> <crc32>` or
/// `case <i> err <code>` per case.
const DECODE_SOURCE: &str = r#"
IMPORT collections
IMPORT compress
IMPORT fs
IMPORT io
IMPORT os

FUNC u32At(b AS List OF Byte, at AS Integer) AS Integer
  RETURN toInt(collections::get(b, at)) + 256 * toInt(collections::get(b, at + 1)) + 65536 * toInt(collections::get(b, at + 2)) + 16777216 * toInt(collections::get(b, at + 3))
END FUNC

FUNC described(out AS List OF Byte) AS String
  RETURN "ok " & toString(len(out)) & " " & toString(compress::crc32(out))
END FUNC

FUNC rawCase(data AS List OF Byte) AS String
  LET out AS List OF Byte = compress::inflate(data) TRAP(e)
    RETURN "err " & toString(e.code)
  END TRAP
  RETURN described(out)
END FUNC

FUNC zlibCase(data AS List OF Byte, lenient AS Boolean) AS String
  LET out AS List OF Byte = compress::zlibDecode(data, 67108864, lenient) TRAP(e)
    RETURN "err " & toString(e.code)
  END TRAP
  RETURN described(out)
END FUNC

FUNC gzipCase(data AS List OF Byte, lenient AS Boolean) AS String
  LET out AS List OF Byte = compress::gzipDecode(data, 67108864, lenient) TRAP(e)
    RETURN "err " & toString(e.code)
  END TRAP
  RETURN described(out)
END FUNC

SUB main()
  RES h AS fs::File = fs::openFile(os::getEnv("MFB_COMPRESS_JOB"), "r")
  LET job AS List OF Byte = fs::readAllBytes(h)
  fs::close(h)
  LET count AS Integer = u32At(job, 0)
  MUT offset AS Integer = 4 + 8 * count
  MUT i AS Integer = 0
  WHILE i < count
    LET n AS Integer = u32At(job, 4 + 8 * i)
    LET aux AS Integer = u32At(job, 8 + 8 * i)
    LET data AS List OF Byte = collections::mid(job, offset, n)
    LET fmt AS Integer = aux MOD 16
    LET lenient AS Boolean = aux >= 16
    MUT line AS String = "err unknown-format"
    IF fmt = 0 THEN
      line = rawCase(data)
    ELSEIF fmt = 1 THEN
      line = zlibCase(data, lenient)
    ELSEIF fmt = 2 THEN
      line = gzipCase(data, lenient)
    END IF
    io::print("case " & toString(i) & " " & line)
    offset = offset + n
    i = i + 1
  END WHILE
END SUB
"#;

/// Build the decode program once per test and run it over `cases`, returning each case's
/// `ok <len> <crc32>` / `err <code>` in order.
fn decode_with_mfb(name: &str, cases: &[(Vec<u8>, u32)]) -> Vec<String> {
    let project = temp_project(name, DECODE_SOURCE);
    let exe = build_project(&project);
    let mut job = Vec::new();
    job.extend((cases.len() as u32).to_le_bytes());
    for (data, aux) in cases {
        job.extend((data.len() as u32).to_le_bytes());
        job.extend(aux.to_le_bytes());
    }
    for (data, _) in cases {
        job.extend(data);
    }
    let job_path = project.join("job.bin");
    std::fs::write(&job_path, &job).expect("write job file");
    let (code, stdout, stderr) =
        run_capture_with_env(&exe, &[("MFB_COMPRESS_JOB", job_path.display().to_string())]);
    assert_eq!(code, 0, "decode program failed\nstdout:\n{stdout}\nstderr:\n{stderr}");
    let lines: Vec<String> = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("case "))
        .map(|l| l.split_once(' ').expect("case index").1.to_string())
        .collect();
    assert_eq!(lines.len(), cases.len(), "program stopped early\nstderr:\n{stderr}");
    lines
}

fn ok(bytes: &[u8]) -> String {
    format!("ok {} {}", bytes.len(), reference_crc32(bytes))
}

fn refused() -> String {
    format!("err {ERR_INVALID_FORMAT}")
}

fn encode(format: u32, level: u32, data: &[u8]) -> Vec<u8> {
    let c = Compression::new(level);
    match format {
        RAW => {
            let mut e = DeflateEncoder::new(Vec::new(), c);
            e.write_all(data).unwrap();
            e.finish().unwrap()
        }
        ZLIB => {
            let mut e = ZlibEncoder::new(Vec::new(), c);
            e.write_all(data).unwrap();
            e.finish().unwrap()
        }
        _ => {
            let mut e = GzEncoder::new(Vec::new(), c);
            e.write_all(data).unwrap();
            e.finish().unwrap()
        }
    }
}

/// A seeded mix with different statistics: text lines, incompressible bytes, and a long run.
fn decode_corpus() -> Vec<u8> {
    let mut rng = Lcg(0x0137_B00D_C0DE);
    let mut out = Vec::new();
    for i in 0..600 {
        out.extend(format!("record {i}: the quick brown fox {} jumps\n", i % 97).bytes());
    }
    out.extend((0..8000).map(|_| rng.next() as u8));
    out.extend(std::iter::repeat_n(0u8, 6000));
    out
}

/// A gzip member whose header carries `FHCRC`; `crc_delta` damages the header CRC-16.
fn gzip_with_fhcrc(data: &[u8], crc_delta: u16) -> Vec<u8> {
    let mut member = vec![0x1f, 0x8b, 8, 0x02, 0, 0, 0, 0, 0, 255];
    let header_crc = (reference_crc32(&member) & 0xFFFF) as u16 ^ crc_delta;
    member.extend(header_crc.to_le_bytes());
    member.extend(encode(RAW, 6, data));
    member.extend(reference_crc32(data).to_le_bytes());
    member.extend((data.len() as u32).to_le_bytes());
    member
}

#[test]
fn flate2_streams_decode_at_every_level_and_format() {
    let corpus = decode_corpus();
    let mut cases = Vec::new();
    for format in [RAW, ZLIB, GZIP] {
        for level in 0..=9 {
            cases.push((encode(format, level, &corpus), format));
        }
    }
    assert_eq!(cases.len(), EXPECTED_ENCODE_CASES);
    let got = decode_with_mfb("rt_compress_decode_levels", &cases);
    for (i, line) in got.iter().enumerate() {
        assert_eq!(line, &ok(&corpus), "case {i}: format {} level {}", i / 10, i % 10);
    }
}

/// A decoder that ignored a checksum, or accepted a stream an independent decoder refuses, fails
/// here: every seeded single-byte flip of a zlib or gzip stream must get `flate2`'s verdict —
/// refused, or decoded to the same bytes.
#[test]
fn tampered_checksummed_streams_get_flate2s_verdict() {
    let corpus = decode_corpus();
    let mut rng = Lcg(0x0137_7A3F);
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for (format, first) in [(ZLIB, 2usize), (GZIP, 10usize)] {
        let stream = encode(format, 6, &corpus);
        for _ in 0..40 {
            let mut t = stream.clone();
            let at = first + (rng.next() as usize) % (t.len() - first);
            t[at] ^= 1 + (rng.next() % 255) as u8;
            let mut decoded = Vec::new();
            let verdict = if format == ZLIB {
                ZlibDecoder::new(&t[..]).read_to_end(&mut decoded)
            } else {
                MultiGzDecoder::new(&t[..]).read_to_end(&mut decoded)
            };
            expected.push(match verdict {
                Ok(_) => ok(&decoded),
                Err(_) => refused(),
            });
            cases.push((t, format));
        }
    }
    assert_eq!(cases.len(), EXPECTED_TAMPER_CASES);
    let got = decode_with_mfb("rt_compress_decode_tamper", &cases);
    let disagreements: Vec<String> = got
        .iter()
        .zip(&expected)
        .enumerate()
        .filter(|(_, (mine, theirs))| mine != theirs)
        .map(|(i, (mine, theirs))| format!("case {i}: mfb `{mine}`, flate2 `{theirs}`"))
        .collect();
    assert!(disagreements.is_empty(), "{}", disagreements.join("\n"));
}

/// audit-3 DEC-58 (over-subscribed Huffman codes accepted) and DEC-57's Adler-32 half, plus two
/// neighbouring code-set refusals. The raw streams are `tools/oracles/compress/python/probe_streams.py`
/// cases, hex-dumped with `python3 -c "from probe_streams import cases; ..."`; Python zlib 1.2.12
/// refuses each with the message named beside it.
#[test]
fn dec57_dec58_regressions_are_refused() {
    let corpus = decode_corpus();
    let mut bad_adler = encode(ZLIB, 6, &corpus);
    let last = bad_adler.len() - 1;
    bad_adler[last] ^= 0x01;
    let cases = vec![
        // "invalid literal/lengths set" — three 1-bit codes
        (common::decode_hex("edc001040000000010000000000000000000000000030000000000000000000000000000000000008000000000"), RAW),
        // "invalid literal/lengths set" — three 2-bit codes, incomplete
        (common::decode_hex("ed800104000000400000000000000000000000000c000000000000000000000000000000000000000200000004"), RAW),
        // "invalid distance too far back"
        (common::decode_hex("edc2010800000082200000000000000000000000000000000000000000000000003e00000000000000000000000000000000000000000000000000000000000000000000000000008002000000000000401701"), RAW),
        // "incorrect data check"
        (bad_adler, ZLIB),
    ];
    assert_eq!(cases.len(), EXPECTED_REGRESSION_CASES);
    let got = decode_with_mfb("rt_compress_decode_regressions", &cases);
    for (i, line) in got.iter().enumerate() {
        assert_eq!(line, &refused(), "regression case {i}");
    }
}

/// The behaviours plan-137-A §Decisions settled: checksum comparisons are skipped only with
/// `ignoreChecksum := TRUE` and never skip structure; a preset dictionary is refused; bytes after a
/// stream are ignored; gzip members concatenate; a `1f 8b` that is not a member is refused.
#[test]
fn decided_behaviours_hold() {
    let corpus = decode_corpus();
    let second = b"second member".to_vec();
    let zlib = encode(ZLIB, 6, &corpus);
    let gzip = encode(GZIP, 6, &corpus);
    let raw = encode(RAW, 6, &corpus);

    let mut bad_adler = zlib.clone();
    let n = bad_adler.len();
    bad_adler[n - 1] ^= 1;
    let mut bad_crc = gzip.clone();
    let n = bad_crc.len();
    bad_crc[n - 8] ^= 1;
    let mut bad_isize = gzip.clone();
    let n = bad_isize.len();
    bad_isize[n - 4] ^= 1;
    // A stored block whose NLEN is not LEN's complement, zlib-wrapped with a correct Adler-32.
    let bad_nlen = common::decode_hex("7801010300fcfe616263024d0127");
    // FDICT set: CMF 0x78, FLG 0x20 plus FCHECK, a DICTID, then the deflate data and Adler-32.
    let fdict_flg = 0x20u8 + (31 - ((0x78u32 * 256 + 0x20) % 31) % 31) as u8;
    let mut fdict = vec![0x78, fdict_flg, 0x12, 0x34, 0x56, 0x78];
    fdict.extend(&raw);
    fdict.extend(&zlib[zlib.len() - 4..]);

    let mut cases: Vec<(Vec<u8>, u32, String, &str)> = vec![
        (bad_adler.clone(), ZLIB, refused(), "bad Adler-32"),
        (bad_adler, ZLIB | LENIENT, ok(&corpus), "bad Adler-32, ignoreChecksum"),
        (bad_crc.clone(), GZIP, refused(), "bad CRC-32"),
        (bad_crc, GZIP | LENIENT, ok(&corpus), "bad CRC-32, ignoreChecksum"),
        (bad_isize.clone(), GZIP, refused(), "bad ISIZE"),
        (bad_isize, GZIP | LENIENT, ok(&corpus), "bad ISIZE, ignoreChecksum"),
        (gzip_with_fhcrc(&corpus, 0), GZIP, ok(&corpus), "correct FHCRC"),
        (gzip_with_fhcrc(&corpus, 1), GZIP, refused(), "bad FHCRC"),
        (gzip_with_fhcrc(&corpus, 1), GZIP | LENIENT, ok(&corpus), "bad FHCRC, ignoreChecksum"),
        (bad_nlen, ZLIB | LENIENT, refused(), "bad NLEN, ignoreChecksum"),
        (fdict.clone(), ZLIB, refused(), "FDICT"),
        (fdict, ZLIB | LENIENT, refused(), "FDICT, ignoreChecksum"),
    ];
    for extra in [1usize, 7, 1000] {
        let junk: Vec<u8> = (0..extra).map(|i| 0x5a ^ (i as u8)).collect();
        for (stream, format, label) in [(&raw, RAW, "raw"), (&zlib, ZLIB, "zlib"), (&gzip, GZIP, "gzip")] {
            let mut s = stream.clone();
            s.extend(&junk);
            cases.push((s, format, ok(&corpus), label));
        }
    }
    let mut two = gzip.clone();
    two.extend(encode(GZIP, 1, &second));
    let mut joined = corpus.clone();
    joined.extend(&second);
    cases.push((two, GZIP, ok(&joined), "two gzip members"));
    let mut false_member = gzip.clone();
    false_member.extend([0x1f, 0x8b, 0x00, 0x00]);
    false_member.extend(b"junk");
    cases.push((false_member, GZIP, refused(), "1f 8b then garbage"));
    assert_eq!(cases.len(), EXPECTED_DECIDED_CASES);

    let job: Vec<(Vec<u8>, u32)> = cases.iter().map(|(d, aux, _, _)| (d.clone(), *aux)).collect();
    let got = decode_with_mfb("rt_compress_decode_decided", &job);
    let wrong: Vec<String> = got
        .iter()
        .zip(&cases)
        .filter(|(mine, (_, _, want, _))| *mine != want)
        .map(|(mine, (_, _, want, label))| format!("{label}: got `{mine}`, want `{want}`"))
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// Encoders: `compress::deflate` / `zlibEncode` / `gzipEncode`, decoded by `flate2` (miniz_oxide).
// ---------------------------------------------------------------------------------------------

use flate2::read::DeflateDecoder;

/// raw / zlib / gzip x levels 0..=9 x the seven payloads of `encode_payloads`.
const EXPECTED_ENCODED_CASES: usize = 3 * 10 * 7;

/// Reads the job named by `MFB_COMPRESS_JOB` — a `(length, format * 16 + level)` pair per case —
/// compresses each case twice, prints `case <i> <TRUE when both calls gave the same bytes>`, and
/// writes the first output of every case, in the same job layout, to `MFB_COMPRESS_OUT`.
const ENCODE_SOURCE: &str = r#"
IMPORT collections
IMPORT compress
IMPORT fs
IMPORT io
IMPORT os

FUNC u32At(b AS List OF Byte, at AS Integer) AS Integer
  RETURN toInt(collections::get(b, at)) + 256 * toInt(collections::get(b, at + 1)) + 65536 * toInt(collections::get(b, at + 2)) + 16777216 * toInt(collections::get(b, at + 3))
END FUNC

FUNC encodeOnce(data AS List OF Byte, fmt AS Integer, level AS Integer) AS List OF Byte
  IF fmt = 0 THEN
    RETURN compress::deflate(data, level)
  ELSEIF fmt = 1 THEN
    RETURN compress::zlibEncode(data, level)
  END IF
  RETURN compress::gzipEncode(data, level)
END FUNC

SUB main()
  RES h AS fs::File = fs::openFile(os::getEnv("MFB_COMPRESS_JOB"), "r")
  LET job AS List OF Byte = fs::readAllBytes(h)
  fs::close(h)
  LET count AS Integer = u32At(job, 0)
  MUT offset AS Integer = 4 + 8 * count
  MUT fields AS List OF Integer = [count]
  MUT produced AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < count
    LET n AS Integer = u32At(job, 4 + 8 * i)
    LET aux AS Integer = u32At(job, 8 + 8 * i)
    LET data AS List OF Byte = collections::mid(job, offset, n)
    LET packed AS List OF Byte = encodeOnce(data, aux / 16, aux MOD 16)
    LET again AS List OF Byte = encodeOnce(data, aux / 16, aux MOD 16)
    MUT same AS Boolean = len(packed) = len(again)
    MUT k AS Integer = 0
    WHILE same AND k < len(packed)
      same = collections::get(packed, k) = collections::get(again, k)
      k = k + 1
    END WHILE
    io::print("case " & toString(i) & " " & toString(same))
    fields = collections::append(fields, len(packed))
    fields = collections::append(fields, aux)
    k = 0
    WHILE k < len(packed)
      produced = collections::append(produced, collections::get(packed, k))
      k = k + 1
    END WHILE
    offset = offset + n
    i = i + 1
  END WHILE
  MUT out AS List OF Byte = []
  MUT f AS Integer = 0
  WHILE f < len(fields)
    LET v AS Integer = collections::get(fields, f)
    out = collections::append(out, toByte(v MOD 256))
    out = collections::append(out, toByte((v / 256) MOD 256))
    out = collections::append(out, toByte((v / 65536) MOD 256))
    out = collections::append(out, toByte((v / 16777216) MOD 256))
    f = f + 1
  END WHILE
  f = 0
  WHILE f < len(produced)
    out = collections::append(out, collections::get(produced, f))
    f = f + 1
  END WHILE
  fs::writeBytes(os::getEnv("MFB_COMPRESS_OUT"), out)
END SUB
"#;

/// plan-137-D §1's edge sizes (0, 1, 2, and around the 65,535-byte stored-block limit), a run of
/// zeros long enough for many 258-byte matches, and seeded text.
fn encode_payloads() -> Vec<Vec<u8>> {
    let mut rng = Lcg(0x137d);
    let mut random = |n: usize| (0..n).map(|_| rng.next() as u8).collect::<Vec<u8>>();
    let below = random(65_535);
    let above = random(65_537);
    let text: Vec<u8> = (0..2_000)
        .flat_map(|i| format!("line {i} of the interop corpus, bucket {}\n", i % 13).into_bytes())
        .collect();
    vec![Vec::new(), vec![b'a'], b"ab".to_vec(), below, above, vec![0u8; 70_000], text]
}

/// Split a job file into its cases' bytes and `aux` values.
fn read_job(blob: &[u8]) -> Vec<(Vec<u8>, u32)> {
    let u32_at = |at: usize| u32::from_le_bytes(blob[at..at + 4].try_into().unwrap());
    let count = u32_at(0) as usize;
    let mut offset = 4 + 8 * count;
    (0..count)
        .map(|i| {
            let n = u32_at(4 + 8 * i) as usize;
            let case = (blob[offset..offset + n].to_vec(), u32_at(8 + 8 * i));
            offset += n;
            case
        })
        .collect()
}

#[test]
fn encoders_output_decodes_with_flate2_and_is_deterministic() {
    let payloads = encode_payloads();
    let mut cases = Vec::new();
    for payload in 0..payloads.len() {
        for format in [RAW, ZLIB, GZIP] {
            for level in 0..=9u32 {
                cases.push((payload, format, level));
            }
        }
    }
    assert_eq!(cases.len(), EXPECTED_ENCODED_CASES);

    let project = temp_project("compress_encode_interop", ENCODE_SOURCE);
    let exe = build_project(&project);
    let mut job = Vec::new();
    job.extend((cases.len() as u32).to_le_bytes());
    for &(payload, format, level) in &cases {
        job.extend((payloads[payload].len() as u32).to_le_bytes());
        job.extend((format * 16 + level).to_le_bytes());
    }
    for &(payload, _, _) in &cases {
        job.extend(&payloads[payload]);
    }
    let job_path = project.join("job.bin");
    std::fs::write(&job_path, &job).expect("write job file");

    let run = |name: &str| -> (Vec<String>, Vec<u8>) {
        let out_path = project.join(name);
        let (code, stdout, stderr) = run_capture_with_env(
            &exe,
            &[
                ("MFB_COMPRESS_JOB", job_path.display().to_string()),
                ("MFB_COMPRESS_OUT", out_path.display().to_string()),
            ],
        );
        assert_eq!(code, 0, "encode program failed\nstdout:\n{stdout}\nstderr:\n{stderr}");
        let lines = stdout
            .lines()
            .filter_map(|l| l.strip_prefix("case "))
            .map(|l| l.split_once(' ').expect("case index").1.to_string())
            .collect();
        (lines, std::fs::read(&out_path).expect("read encoder output"))
    };
    let (lines, first) = run("produced-1.bin");
    let (_, second) = run("produced-2.bin");
    assert_eq!(lines.len(), cases.len(), "program stopped early");
    assert!(first == second, "two runs of the encoders wrote different bytes");

    let produced = read_job(&first);
    assert_eq!(produced.len(), cases.len());
    let mut failures = Vec::new();
    for (i, &(payload, format, level)) in cases.iter().enumerate() {
        let (bytes, aux) = &produced[i];
        assert_eq!(*aux, format * 16 + level);
        // MFBASIC prints a Boolean as `TRUE` / `FALSE`.
        if lines[i] != "TRUE" {
            failures.push(format!("case {i} (format {format}, level {level}): two calls differ"));
        }
        let mut decoded = Vec::new();
        let result = match format {
            RAW => DeflateDecoder::new(&bytes[..]).read_to_end(&mut decoded),
            ZLIB => ZlibDecoder::new(&bytes[..]).read_to_end(&mut decoded),
            _ => MultiGzDecoder::new(&bytes[..]).read_to_end(&mut decoded),
        };
        match result {
            Err(e) => failures.push(format!(
                "case {i} (format {format}, level {level}): flate2 refused: {e}"
            )),
            Ok(_) if decoded != payloads[payload] => failures.push(format!(
                "case {i} (format {format}, level {level}): decoded {} bytes, payload {}",
                decoded.len(),
                payloads[payload].len()
            )),
            Ok(_) => {}
        }
    }
    assert!(
        failures.is_empty(),
        "{} failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
