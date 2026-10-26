//! bug-774: `compress::crc32` and `compress::inflate` throughput, and the code shape
//! that delivers it.
//!
//! **Timing.** One release build times, interleaved over five rounds in one
//! process, `compress::crc32` of a 128 KiB text, `compress::inflate` of its level-6
//! DEFLATE stream, a 5-byte `inflate` (the per-call floor), and a reference: a
//! `collections::mid` copy of the same 128 KiB list, a native word-copy loop none
//! of the bug-774 changes touch. Each median is compared with the reference's, so
//! the bound calibrates for the machine's speed. The ratios are only meaningful on
//! the machine class they were measured on — a word copy and a table-driven CRC
//! scale differently across microarchitectures — so the timing test runs on
//! macOS AArch64 only. Measured there (`mfb build -O 1`, medians of 5):
//!
//! | ratio to the reference | before (`e089c1c85`) | after | bound |
//! |---|---|---|---|
//! | 8 × `crc32` / 8 × `mid` | 9.78 | 5.22 | 6.5 |
//! | 8 × `inflate` / 8 × `mid` | 41.1 | 13.6 | 24 |
//! | 200 × tiny `inflate` / 8 × `mid` | 23.5 | 1.15 | 5 |
//!
//! The arena zero-fill change (`4bb55ea8c`) subsequently sped up the allocating
//! reference. A/B on the same machine at its parent and HEAD, using this PROGRAM:
//! CRC medians were 926/926 us, while reference medians were 179/62 us (2.89x).
//! Inflate improved from 2420 to 2027 us and tiny inflate from 203 to 90 us.
//! Scale only the now-invalid CRC/inflate bounds by that reference speedup
//! (rounded down to 18.5/68); the tiny bound still holds. The original pre-fix
//! ratios scaled by the same factor are 28.3/118.8, so these bounds still reject
//! the bug-774 regressions. Code-shape assertions below remain independent of
//! allocator performance.
//!
//! The plan-183 arena redesign (`260efd0d7`..`1c84f6ef3`, A–D) sped the reference up
//! again. A/B of this PROGRAM's release binaries from `f89707d6c` (before plan-183),
//! `1c84f6ef3` and `7ae9c0675`, three interleaved runs each: CRC medians 905–929 us at
//! all three, reference medians 62–67 us before and 50–53 us after (1.29x, 67/52).
//! The CRC ratio went from 13.5–14.9 to 17.1–18.1, inside 18.5 by too little to
//! survive the four code-shape builds this binary runs beside it (18.96x and 19.68x
//! measured). Scale only the CRC bound by that speedup (rounded down to 23.5); the
//! inflate (40–42) and tiny (2.0–2.1) ratios still hold theirs. The pre-fix CRC ratio
//! scaled the same way is 36.5, 1.55x above the new bound.
//!
//! Every bound is at least 1.5x below the pre-fix ratio.
//!
//! **Shape.** On every target: the CRC loop's constant-count shifts carry no range
//! check, the decoder copies a back-reference with the in-place self-slice append
//! (no `mid` temporary), and the fixed Huffman tables are built once per program,
//! not on every decoder call.

#[path = "../common/mod.rs"]
mod common;
use common::{build_ncode, temp_project};
use serde_json::Value;

const PROGRAM: &str = r#"IMPORT io
IMPORT compress
IMPORT collections
IMPORT datetime
IMPORT strings

FUNC mkInput(target AS Integer) AS List OF Byte
  LET words AS List OF String = ["the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog", "compress", "deflate", "window", "huffman", "literal", "length", "distance", "block"]
  MUT parts AS List OF String = []
  MUT total AS Integer = 0
  MUT x AS Integer = 12345
  WHILE total < target
    x = (x * 1103515245 + 12345) MOD 2147483648
    LET w AS String = collections::get(words, (x / 65536) MOD 16)
    MUT sep AS String = " "
    IF (x / 1024) MOD 13 = 0 THEN
      sep = "\n"
    END IF
    parts = collections::append(parts, w & sep)
    total = total + len(w) + 1
  END WHILE
  RETURN strings::toBytes(strings::join(parts, ""))
END FUNC

FUNC main() AS Nothing
  LET x AS List OF Byte = mkInput(131072)
  LET d AS List OF Byte = compress::deflate(x, 6)
  LET tiny AS List OF Byte = compress::deflate(strings::toBytes("hello"), 6)
  MUT check AS Integer = 0
  FOR r = 1 TO 5
    MUT t0 AS Integer = datetime::monotonicNanos()
    FOR rep = 1 TO 8
      check = check + compress::crc32(x)
    NEXT
    io::print("crc " & toString(datetime::monotonicNanos() - t0))
    t0 = datetime::monotonicNanos()
    FOR rep = 1 TO 8
      check = check + len(compress::inflate(d))
    NEXT
    io::print("inflate " & toString(datetime::monotonicNanos() - t0))
    t0 = datetime::monotonicNanos()
    FOR rep = 1 TO 200
      check = check + len(compress::inflate(tiny))
    NEXT
    io::print("tiny " & toString(datetime::monotonicNanos() - t0))
    t0 = datetime::monotonicNanos()
    FOR rep = 1 TO 8
      check = check + len(collections::mid(x, 1, 131000))
    NEXT
    io::print("ref " & toString(datetime::monotonicNanos() - t0))
  NEXT
  io::print("check " & toString(check))
END FUNC
"#;

// Only the macOS-aarch64-gated test below uses this; the gate keeps other builds warning-free.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn median(mut v: Vec<u64>) -> u64 {
    v.sort_unstable();
    v[v.len() / 2]
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn crc32_and_inflate_run_within_their_bounds_of_a_native_copy() {
    use std::time::Duration;
    let project = temp_project("compress_throughput", PROGRAM);
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &binary,
        Duration::from_secs(240),
        "the compress throughput probe did not finish",
    );
    let _ = std::fs::remove_dir_all(&project);
    assert!(
        status.success(),
        "{}:\n{stdout}",
        common::exit_description(&status)
    );
    // Every round decodes the same bytes: 5 × (8 × CRC + 8 × 131,073 + 200 × 5 +
    // 8 × 131,000). The CRC of the text is fixed, so the sum pins the values too.
    assert!(
        stdout.contains("check 141266448080"),
        "the probe computed different values:\n{stdout}"
    );
    let times = |label: &str| -> Vec<u64> {
        stdout
            .lines()
            .filter_map(|l| l.strip_prefix(label))
            .map(|t| t.trim().parse::<u64>().expect("nanoseconds"))
            .collect()
    };
    let reference = median(times("ref ")) as f64;
    for (label, bound) in [("crc ", 23.5), ("inflate ", 68.0), ("tiny ", 5.0)] {
        let runs = times(label);
        assert_eq!(runs.len(), 5, "{stdout}");
        let ratio = median(runs.clone()) as f64 / reference;
        assert!(
            ratio <= bound,
            "{}took {ratio:.2}x the reference copy (bound {bound}); runs {runs:?} ns, \
             reference runs {:?} ns",
            label,
            times("ref "),
        );
    }
}

fn function<'a>(ncode: &'a Value, name: &str) -> &'a Value {
    ncode["functions"]
        .as_array()
        .expect("functions")
        .iter()
        .find(|f| f["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("no function `{name}` in the -ncode dump"))
}

fn labels<'a>(function: &'a Value, prefix: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    function["instructions"]
        .as_array()
        .expect("instructions")
        .iter()
        .filter(|i| i["op"].as_str() == Some("label"))
        .filter_map(|i| i["name"].as_str())
        .filter(move |name| name.starts_with(prefix))
}

fn calls<'a>(function: &'a Value) -> impl Iterator<Item = &'a str> + 'a {
    function["instructions"]
        .as_array()
        .expect("instructions")
        .iter()
        .filter(|i| i["op"].as_str() == Some("bl"))
        .filter_map(|i| i["target"].as_str())
}

fn assert_shape(target: &str) {
    let project = temp_project("compress_shape", PROGRAM);
    let ncode = build_ncode(&project, target, "compress_shape");
    let _ = std::fs::remove_dir_all(&project);
    let crc = function(&ncode, "#compress_crc32Run");
    assert_eq!(
        labels(crc, "bits_shift_out_of_range").count(),
        0,
        "{target}: the CRC loop's shifts are all by literal counts and need no range check"
    );
    let core = function(&ncode, "#compress_inflateCore");
    assert!(
        labels(core, "self_slice_append_realloc").count() >= 2,
        "{target}: both decode loops copy a back-reference in place with no `mid` temporary"
    );
    let rebuilt: Vec<&str> = calls(core)
        .filter(|target| target.contains("fixedLitLens"))
        .collect();
    assert!(
        rebuilt.is_empty(),
        "{target}: the decoder rebuilds the fixed literal table on every call: {rebuilt:?}"
    );
}

#[test]
fn compress_hot_loops_have_the_bug_774_shape_on_macos_aarch64() {
    assert_shape("macos-aarch64");
}

#[test]
fn compress_hot_loops_have_the_bug_774_shape_on_linux_x86_64() {
    assert_shape("linux-x86_64");
}

#[test]
fn compress_hot_loops_have_the_bug_774_shape_on_linux_riscv64() {
    assert_shape("linux-riscv64");
}

#[test]
fn compress_hot_loops_have_the_bug_774_shape_on_windows_x86_64() {
    assert_shape("windows-x86_64");
}
