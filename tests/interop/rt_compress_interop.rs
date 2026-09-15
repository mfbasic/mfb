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
