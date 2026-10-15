//! bug-992: a `strings` member must not leave a block with no owner.
//!
//! `strings::graphemesCount` and the copying `strings::graphemeAt` each built the
//! whole `List OF String` of their argument's grapheme clusters, read one word or
//! one entry out of it, and dropped it with nothing to free it: one list per call,
//! `round16(40 + 40 × clusters + byteLen)` bytes. An unbound `strings::fromScalars`
//! result leaked too, because its producer never marked the block fresh.
//!
//! Each case runs one `--debug` build at N and 2N calls and requires the main
//! arena's counters to say the loop freed what it allocated: `live_bytes` zero at
//! exit, `alloc_calls == free_calls`, and no double free. `graphemesCount` must
//! not allocate at all, so its `alloc_calls` may not grow with N. The flat
//! siblings are pinned as controls, and every case prints a value so a fix that
//! changed a result would fail there.
#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use common::debug_report::{arena_lines, build_debug, counter};

struct Run {
    allocs: u64,
    frees: u64,
    live: u64,
    stdout: String,
}

fn run_at(name: &str, exe: &std::path::Path, n: u32) -> Run {
    let (status, stdout, stderr) = common::run_capture_with_env(exe, &[("N", n.to_string())]);
    assert_eq!(status, 0, "{name} N={n} failed:\n{stdout}\n{stderr}");
    let lines = arena_lines(name, &stderr);
    assert_eq!(
        counter(name, &lines, 0, "double_free_skips"),
        0,
        "{name} N={n}: the arena skipped a double free"
    );
    Run {
        allocs: counter(name, &lines, 0, "alloc_calls"),
        frees: counter(name, &lines, 0, "free_calls"),
        live: counter(name, &lines, 0, "live_bytes"),
        stdout,
    }
}

/// A program that evaluates `expression` (an `Integer` over `s`, the record field
/// `b.lore`) N times and prints the sum and one sample.
fn program(expression: &str) -> String {
    format!(
        r#"IMPORT io
IMPORT os
IMPORT strings
IMPORT collections

TYPE Beast
  lore AS String
END TYPE

FUNC probe(b AS Beast) AS Integer
  LET s AS String = b.lore
  RETURN {expression}
END FUNC

FUNC main() AS Integer
  LET b = Beast[lore := os::getEnvOr("LORE", "The quick brown fox jumps over the lazy dog a😀b👍🏽c")]
  MUT total AS Integer = 0
  FOR i = 1 TO toInt(os::getEnv("N"))
    total = total + probe(b)
  NEXT
  io::print(toString(total) & " " & toString(probe(b)))
  RETURN 0
END FUNC
"#
    )
}

/// Build `expression`'s program, run it at 100 and 200 calls, require a balanced
/// arena at both, and return `(runs, per-call value)`.
fn balanced(name: &str, expression: &str) -> (Run, Run, String) {
    let exe = build_debug(name, &program(expression));
    let low = run_at(name, &exe, 100);
    let high = run_at(name, &exe, 200);
    for (n, run) in [(100, &low), (200, &high)] {
        assert_eq!(
            (run.allocs, run.live),
            (run.frees, 0),
            "{name} N={n}: alloc_calls {} free_calls {} live_bytes {}",
            run.allocs,
            run.frees,
            run.live
        );
    }
    let sample = low
        .stdout
        .split_whitespace()
        .nth(1)
        .unwrap_or_else(|| panic!("{name}: no sample in {:?}", low.stdout))
        .to_string();
    (low, high, sample)
}

/// Allocations per call of `expression`'s loop, from the two runs.
fn allocs_per_call(low: &Run, high: &Run) -> u64 {
    (high.allocs - low.allocs) / 100
}

/// What the loop allocates per call around the expression itself (`len(s)`
/// allocates nothing of its own).
fn baseline_allocs_per_call() -> u64 {
    let (low, high, sample) = balanced("b992_baseline", "len(s)");
    assert_eq!(sample, "50");
    allocs_per_call(&low, &high)
}

// 44 ASCII scalars, then `a😀b👍🏽c`: 5 clusters, 6 scalars, 15 bytes.
const CLUSTERS: &str = "49";

#[test]
fn graphemes_count_allocates_nothing() {
    let (low, high, sample) = balanced("b992_count", "strings::graphemesCount(s)");
    assert_eq!(sample, CLUSTERS);
    assert_eq!(
        allocs_per_call(&low, &high),
        baseline_allocs_per_call(),
        "graphemesCount must not allocate"
    );
}

#[test]
fn grapheme_at_frees_everything_but_its_result() {
    // Cluster 47 is `👍🏽`: one cluster, two scalars.
    let (low, high, sample) = balanced("b992_at", "len(strings::graphemeAt(s, 47))");
    assert_eq!(sample, "2");
    assert_eq!(
        allocs_per_call(&low, &high),
        baseline_allocs_per_call() + 1,
        "graphemeAt allocates its result and nothing else"
    );
    let (_, _, sample) = balanced(
        "b992_at_bytes",
        "strings::byteLen(strings::graphemeAt(s, 45))",
    );
    assert_eq!(sample, "4", "cluster 45 is the four-byte emoji");
}

#[test]
fn an_unbound_from_scalars_result_is_freed() {
    let (_, _, sample) = balanced(
        "b992_from_scalars",
        "len(strings::fromScalars(strings::toScalars(s)))",
    );
    assert_eq!(sample, "50");
}

/// Found by this bug's audit sweep: the copying `normalizeNfc` allocates a scalar
/// array (8 bytes per scalar) for a non-ASCII string and never freed it. The probe
/// string is non-ASCII, so this takes that path.
#[test]
fn normalize_nfc_frees_its_scalar_array() {
    let (low, high, sample) = balanced("b992_nfc", "len(strings::normalizeNfc(s))");
    assert_eq!(sample, "50");
    assert_eq!(
        allocs_per_call(&low, &high),
        baseline_allocs_per_call() + 2,
        "the scalar array and the result"
    );
    // A composing sequence still composes: `e` + U+0301 is one scalar afterwards.
    let exe = build_debug(
        "b992_nfc_value",
        r#"IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET s AS String = io::readLine()
  io::print(toString(len(s)) & " " & toString(len(strings::normalizeNfc(s))))
  RETURN 0
END FUNC
"#,
    );
    let out = common::run_with_stdin(&exe, "e\u{301}fg caf\u{e9}\n".as_bytes());
    assert_eq!(out.lines().next(), Some("9 8"));
}

/// The measured-flat siblings stay flat, and a bound `fromScalars` still works: a
/// fix that disabled an allocation or a free elsewhere would show here.
#[test]
fn the_flat_siblings_stay_flat() {
    for (name, expression, expected) in [
        ("b992_graphemes", "len(strings::graphemes(s))", CLUSTERS),
        ("b992_width", "strings::displayWidth(s)", "51"),
        ("b992_mid", "len(strings::mid(s, 1, 10))", "10"),
        ("b992_scalars", "len(strings::toScalars(s))", "50"),
    ] {
        let (_, _, sample) = balanced(name, expression);
        assert_eq!(sample, expected, "{name}");
    }
    let exe = build_debug(
        "b992_bound",
        &program("len(s)").replace(
            "  RETURN len(s)\n",
            "  LET t AS String = strings::fromScalars(strings::toScalars(s))\n  RETURN len(t)\n",
        ),
    );
    for n in [100, 200] {
        let run = run_at("b992_bound", &exe, n);
        assert_eq!((run.allocs, run.live), (run.frees, 0), "b992_bound N={n}");
    }
}

#[test]
fn width_padding_stays_ungated_and_rejects_zero_width_pad_chars() {
    let exe = build_debug(
        "p178_width_padding_off_tui",
        r#"IMPORT errorCode
IMPORT io
IMPORT strings

FUNC rejected(padChar AS String) AS Boolean
  LET ignored AS String = strings::padRightToWidth("x", 3, padChar)
  RETURN FALSE
  TRAP(e)
    RETURN e.code = errorCode::ErrInvalidArgument
  END TRAP
END FUNC

FUNC main() AS Integer
  io::print(strings::padLeftToWidth("界", 4, "."))
  io::print(strings::padRightToWidth("e\u{301}", 3, "."))
  io::print(toString(rejected("\u{301}")) & " " & toString(rejected("\u{200d}")))
  RETURN 0
END FUNC
"#,
    );
    let (status, stdout, stderr) = common::run_capture_with_env(&exe, &[]);
    assert_eq!(status, 0, "{stdout}\n{stderr}");
    assert_eq!(stdout, "..界\ne\u{301}..\nTRUE TRUE\n");
}

/// One value check per member: the three grapheme members agree cluster for
/// cluster, and `graphemeAt` keeps its `ErrIndexOutOfRange` contract.
#[test]
fn the_grapheme_members_still_agree() {
    let source = r#"IMPORT io
IMPORT strings
IMPORT collections
IMPORT errorCode

FUNC at(s AS String, i AS Integer) AS String
  RETURN strings::graphemeAt(s, i)
  TRAP(e)
    IF e.code = errorCode::ErrIndexOutOfRange THEN
      RETURN "<range>"
    END IF
    RETURN "<" & toString(e.code) & ">"
  END TRAP
END FUNC

FUNC check(s AS String) AS String
  LET all AS List OF String = strings::graphemes(s)
  LET n AS Integer = strings::graphemesCount(s)
  IF n <> len(all) THEN
    RETURN "count " & toString(n) & " vs " & toString(len(all))
  END IF
  FOR i = 0 TO n - 1
    IF at(s, i) <> collections::get(all, i) THEN
      RETURN "cluster " & toString(i)
    END IF
  NEXT
  RETURN toString(n) & at(s, 0 - 1) & at(s, n)
END FUNC

FUNC main() AS Integer
  io::print(check(""))
  io::print(check("a"))
  io::print(check("abc"))
  io::print(check("a😀b👍🏽c"))
  io::print(check("e\u{301}fg"))
  io::print(check("👨‍👩‍👧"))
  io::print(check("x\r\ny"))
  io::print(at("a😀b👍🏽c", 3) & "|" & at("abc", 2))
  RETURN 0
END FUNC
"#;
    let exe = build_debug("b992_values", source);
    let (status, stdout, stderr) = common::run_capture_with_env(&exe, &[]);
    assert_eq!(status, 0, "{stdout}\n{stderr}");
    assert_eq!(
        stdout,
        "0<range><range>\n1<range><range>\n3<range><range>\n5<range><range>\n\
         3<range><range>\n1<range><range>\n3<range><range>\n👍🏽|c\n"
    );
}
