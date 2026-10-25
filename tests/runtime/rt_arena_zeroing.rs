//! Freed arena memory is zero when it is handed out again, and fresh memory from the
//! operating system is never zeroed by hand (plan-183-A).
//!
//! **What these tests measure.** A `--debug` build started with `MFB_ARENA_CHECK_ZERO`
//! set checks every chunk `arena_alloc` hands out and counts, in the report, the chunks
//! checked (`zero_checks`) and those holding a non-zero byte past their first 16 bytes
//! (`zero_violations`). A freed chunk's first 16 bytes still hold its free-list node,
//! which the allocator does not clear when it hands the chunk back out. `fill_grow_bytes`
//! counts the bytes the allocator zeroed in freshly mapped blocks.
//!
//! The program frees a short string (a 32-byte chunk), a 360-element list (about
//! 3 KiB) and a 655,360-element list (about 5 MiB) every iteration and allocates the
//! same sizes again in the next, each filled with a non-zero value, so every reused
//! chunk last held data. On AArch64 the 5 MiB chunk goes through the `dc zva` path, on
//! x86-64 through `rep stosb`, on riscv64 through the unrolled store loop.
//!
//! Set `MFB_TEST_REMOTE` to run the programs on another box (`tests/common/remote.rs`).

#[path = "../common/mod.rs"]
mod common;

const REUSE: &str = "\
IMPORT collections
IMPORT io
IMPORT strings

FUNC main AS Integer
  MUT total AS Integer = 0
  FOR i = 1 TO 40
    LET small AS String = strings::repeat(\"ab\", 4 + (i MOD 2))
    LET mid AS List OF Integer = collections::repeat([], i + 1000, 360)
    LET huge AS List OF Integer = collections::repeat([], i + 2000, 655360)
    total = total + len(small) + len(mid) + len(huge)
  NEXT
  io::print(toString(total))
  RETURN 0
END FUNC
";

/// `exe`'s stdout and main-arena report lines, run with `envs`.
fn run(name: &str, exe: &std::path::Path, envs: &[(&str, &str)]) -> (String, Vec<String>) {
    let output = common::remote::output(exe, &[], envs);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{name}: {}\n{stdout}\n{stderr}",
        common::exit_description(&output.status)
    );
    (stdout, common::debug_report::arena_lines(name, &stderr))
}

/// Every reused chunk reads zero past its free-list node, and nothing was zeroed on the
/// grow path.
#[test]
fn freed_chunks_come_back_zero() {
    let name = "arena_zeroing_reuse";
    let exe = common::debug_report::build_debug(name, REUSE);
    let (out, lines) = run(name, &exe, &[("MFB_ARENA_CHECK_ZERO", "1")]);
    assert_eq!(out.trim(), "26229160", "the program's total length");
    let counter = |key: &str| common::debug_report::counter(name, &lines, 0, key);
    // 40 iterations of three values: the first allocates fresh, the rest reuse.
    assert!(
        counter("hit_quick_bin") + counter("hit_large_bin") >= 3 * 39,
        "the freed chunks were not reused: {lines:?}"
    );
    assert_eq!(
        counter("zero_checks"),
        counter("alloc_calls"),
        "every allocation is checked with MFB_ARENA_CHECK_ZERO set"
    );
    assert_eq!(
        counter("zero_violations"),
        0,
        "a reused chunk held a non-zero byte past its first 16: {lines:?}"
    );
    assert_eq!(
        counter("fill_grow_bytes"),
        0,
        "freshly mapped memory is zero from the OS and must not be filled by hand"
    );
    assert_eq!(counter("fill_grow_calls"), 0);
    assert!(counter("fill_free_bytes") > 0, "frees scrub their chunks");
}

/// Without the switch nothing is checked: the check costs a normal `--debug` run nothing.
#[test]
fn the_zero_check_is_off_unless_asked_for() {
    let name = "arena_zeroing_off";
    let exe = common::debug_report::build_debug(name, REUSE);
    let (_, lines) = run(name, &exe, &[]);
    assert_eq!(
        common::debug_report::counter(name, &lines, 0, "zero_checks"),
        0
    );
    let (_, lines) = run(name, &exe, &[("MFB_ARENA_CHECK_ZERO", "")]);
    assert_eq!(
        common::debug_report::counter(name, &lines, 0, "zero_checks"),
        0,
        "an empty value leaves the check off"
    );
}
