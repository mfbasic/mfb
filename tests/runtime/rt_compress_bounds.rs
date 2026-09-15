//! `compress` decoders bound hostile input (plan-137-B §4.4).
//!
//! **A decompression bomb fails at the limit, in bounded memory.** A zlib stream of 64 MiB + 1 zero
//! bytes (≈65 KB compressed) is decoded under the default `maxBytes` (67,108,864). The decoder checks
//! the limit before every write, so it raises `ErrTooLarge` after producing exactly 64 MiB and never
//! builds the larger result.
//!
//! The RSS ceiling is derived from bug-621's fixed growth rule, not measured and padded. The output
//! is a `List OF Byte`, a fixed-width element, so its data capacity is `capacity × 1 B`, and
//! `emit_geometric_step` doubles the capacity below its taper and multiplies it by 1.5 above. At the
//! refusal the list holds 64 MiB, so its last capacity is at most 1.5 × 64 MiB = 96 MiB. Assume the
//! worst case — every block the list ever grew through stays resident — and the output costs at most
//! the geometric sum 96 × (1 + 2/3 + 4/9 + …) = 3 × 96 = 288 MiB. A further 32 MiB covers the
//! input, the Huffman tables and the runtime: **320 MiB**. With bug-621 open the same list reserved
//! ≈19 data bytes per output byte (about 1.2 GiB for 64 MiB), so the ceiling pins the fix.
//!
//! **Decode time is linear in output size.** The same content at n and 4n bytes, decoded in one
//! process with `datetime::monotonicNanos` around each decode, interleaved over five rounds: the 4n
//! median must be at most 4.4× the n median.

#[path = "../common/mod.rs"]
mod common;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;
use std::time::Duration;

/// `ErrTooLarge`, the code a decode past `maxBytes` raises.
const ERR_TOO_LARGE: &str = "77050027";

const BOMB: &str = r#"IMPORT compress
IMPORT fs
IMPORT io

SUB main()
  RES h AS fs::File = fs::openFile("bomb.z", "r")
  LET data AS List OF Byte = fs::readAllBytes(h)
  fs::close(h)
  LET out AS List OF Byte = compress::zlibDecode(data) TRAP(e)
    io::print("raised " & toString(e.code))
    EXIT SUB
  END TRAP
  io::print("decoded " & toString(len(out)))
END SUB
"#;

fn zlib(data: &[u8], level: u32) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::new(level));
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

#[cfg(unix)]
#[test]
fn a_decompression_bomb_is_refused_at_the_limit_in_bounded_memory() {
    let project = common::temp_project("compress_bounds_bomb", BOMB);
    let binary = common::build_project(&project);
    let bomb = zlib(&vec![0u8; 64 * 1024 * 1024 + 1], 9);
    std::fs::write(binary.parent().unwrap().join("bomb.z"), &bomb).expect("write bomb");
    let (status, stdout, rss) = common::run_bounded_with_rss(
        &binary,
        Duration::from_secs(120),
        "decoding a 64 MiB + 1 zlib bomb did not finish",
    );
    let _ = std::fs::remove_dir_all(&project);
    assert!(
        status.success(),
        "{}:\n{stdout}",
        common::exit_description(&status)
    );
    assert_eq!(
        stdout.trim(),
        format!("raised {ERR_TOO_LARGE}"),
        "bomb of {} compressed bytes",
        bomb.len()
    );
    let rss = rss.expect("unix reports ru_maxrss");
    assert!(
        rss < 320 * 1024 * 1024,
        "refusing a 64 MiB + 1 bomb peaked at {} MiB of resident memory (ceiling 320 MiB, derivation in the module doc)",
        rss / (1024 * 1024),
    );
}

const LINEAR: &str = r#"IMPORT compress
IMPORT datetime
IMPORT fs
IMPORT io

FUNC load(path AS String) AS List OF Byte
  RES h AS fs::File = fs::openFile(path, "r")
  LET data AS List OF Byte = fs::readAllBytes(h)
  fs::close(h)
  RETURN data
END FUNC

FUNC timed(data AS List OF Byte) AS Integer
  LET started AS Integer = datetime::monotonicNanos()
  LET out AS List OF Byte = compress::zlibDecode(data)
  LET elapsed AS Integer = datetime::monotonicNanos() - started
  IF len(out) = 0 THEN
    RETURN -1
  END IF
  RETURN elapsed
END FUNC

SUB main()
  LET small AS List OF Byte = load("n.z")
  LET large AS List OF Byte = load("4n.z")
  MUT round AS Integer = 0
  WHILE round < 5
    io::print("n " & toString(timed(small)))
    io::print("4n " & toString(timed(large)))
    round = round + 1
  END WHILE
END SUB
"#;

/// Text lines with varying numbers and a sprinkle of pseudo-random bytes: literals and matches both.
fn content(bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes);
    let mut state = 0x0137_5CA1u64;
    let mut i = 0u64;
    while out.len() < bytes {
        out.extend(
            format!(
                "line {i}: value {} of the quick brown fox\n",
                (i * 7919) % 100_003
            )
            .bytes(),
        );
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        out.push((state >> 33) as u8);
        i += 1;
    }
    out.truncate(bytes);
    out
}

fn median(mut v: Vec<u64>) -> u64 {
    v.sort_unstable();
    v[v.len() / 2]
}

#[test]
fn decode_time_is_linear_in_output_size() {
    let project = common::temp_project("compress_bounds_linear", LINEAR);
    let binary = common::build_project(&project);
    let n = 2 * 1024 * 1024;
    let dir = binary.parent().unwrap();
    std::fs::write(dir.join("n.z"), zlib(&content(n), 6)).expect("write n");
    std::fs::write(dir.join("4n.z"), zlib(&content(4 * n), 6)).expect("write 4n");
    let (status, stdout) = common::run_bounded(
        &binary,
        Duration::from_secs(240),
        "linearity decode did not finish",
    );
    let _ = std::fs::remove_dir_all(&project);
    assert!(
        status.success(),
        "{}:\n{stdout}",
        common::exit_description(&status)
    );
    let times = |label: &str| -> Vec<u64> {
        stdout
            .lines()
            .filter_map(|l| l.strip_prefix(label))
            .map(|t| t.trim().parse::<i64>().expect("nanoseconds"))
            .map(|t| u64::try_from(t).expect("decode produced output"))
            .collect()
    };
    let (small, large) = (times("n "), times("4n "));
    assert_eq!((small.len(), large.len()), (5, 5), "{stdout}");
    let ratio = median(large.clone()) as f64 / median(small.clone()) as f64;
    assert!(
        ratio <= 4.4,
        "decoding 4n took {ratio:.2}x as long as n (limit 4.4); n runs {small:?} ns, 4n runs {large:?} ns"
    );
}
