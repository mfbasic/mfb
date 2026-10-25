//! The region allocator (plan-183-C): free memory merges with its neighbours at once,
//! and under a long random mix of small, medium and huge allocations no chunk is ever
//! handed out twice and every chunk handed out reads zero.
//!
//! **What these tests measure.** The `--debug` report's path counters say which
//! region and which source (a free run, or the region's frontier) served each
//! allocation, and `commits` counts every time the arena took more memory from its
//! slice. With `MFB_ARENA_CHECK_ZERO` set, `zero_violations` counts allocations whose
//! chunk held a non-zero byte.
//!
//! Set `MFB_TEST_REMOTE` to run the programs on another box (`tests/common/remote.rs`).

#[path = "../common/mod.rs"]
mod common;

/// `exe`'s stdout and main-arena report lines, run with `MFB_ARENA_CHECK_ZERO`.
fn run(name: &str, exe: &std::path::Path) -> (String, Vec<String>) {
    let output = common::remote::output(exe, &[], &[("MFB_ARENA_CHECK_ZERO", "1")]);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{name}: {}\n{stdout}\n{stderr}",
        common::exit_description(&output.status)
    );
    (stdout, common::debug_report::arena_lines(name, &stderr))
}

/// Four huge lists (36 MB each), with 1,000 small strings made between them, all freed
/// at once; then one list as large as all four. Only a run made of all four freed
/// chunks merged together can hold it, so it must come from a free run, with nothing
/// newly committed for it.
const MERGE: &str = "\
IMPORT collections
IMPORT io
IMPORT strings

FUNC phase() AS Integer
  MUT small AS Integer = 0
  MUT total AS Integer = 0
  LET a AS List OF Integer = collections::repeat([], 1, 4500000)
  FOR i = 1 TO 250
    small = small + len(strings::repeat(\"s\", 1 + (i MOD 40)))
  NEXT
  LET b AS List OF Integer = collections::repeat([], 2, 4500000)
  FOR i = 1 TO 250
    small = small + len(strings::repeat(\"s\", 1 + (i MOD 40)))
  NEXT
  LET c AS List OF Integer = collections::repeat([], 3, 4500000)
  FOR i = 1 TO 250
    small = small + len(strings::repeat(\"s\", 1 + (i MOD 40)))
  NEXT
  LET d AS List OF Integer = collections::repeat([], 4, 4500000)
  FOR i = 1 TO 250
    small = small + len(strings::repeat(\"s\", 1 + (i MOD 40)))
  NEXT
  total = len(a) + len(b) + len(c) + len(d)
  RETURN total + small
END FUNC

FUNC main AS Integer
  LET first AS Integer = phase()
  io::print(toString(first))
  io::print(\"merged\")
  LET all AS List OF Integer = collections::repeat([], 5, 17990000)
  io::print(toString(len(all)))
  RETURN 0
END FUNC
";

#[test]
fn freed_huge_chunks_merge_into_one_run() {
    let name = "arena_regions_merge";
    let exe = common::debug_report::build_debug(name, MERGE);
    let (out, lines) = run(name, &exe);
    assert_eq!(out, "18019940\nmerged\n17990000\n", "the program's output");
    let counter = |key: &str| common::debug_report::counter(name, &lines, 0, key);
    assert_eq!(
        counter("hit_huge_frontier"),
        4,
        "the four lists are carved from the huge frontier: {lines:?}"
    );
    assert_eq!(
        counter("hit_huge_run"),
        1,
        "the list as large as all four is served by the merged free run: {lines:?}"
    );
    assert!(counter("hit_small") >= 1000, "the small strings: {lines:?}");
    assert_eq!(
        counter("zero_violations"),
        0,
        "every chunk handed out reads zero"
    );
}

/// 100,000 random operations over 64 slots: each replaces one slot's string with a
/// new one — usually small (1–200 bytes), 30% medium (2 KiB–62 KiB), one in ten
/// thousand huge (34 MB) — after checking that the string it replaces still has its
/// length and its letter at its first, middle and last byte. A chunk handed out
/// while still live would be overwritten by its second owner and fail the check.
const STRESS: &str = "\
IMPORT collections
IMPORT io
IMPORT strings

FUNC main AS Integer
  LET letters AS String = \"ABCDEFGHIJKLMNOPQRSTUVWXYZ\"
  MUT slots AS List OF String = collections::repeat([], \"\", 64)
  MUT lens AS List OF Integer = collections::repeat([], 0, 64)
  MUT marks AS List OF String = collections::repeat([], \"\", 64)
  MUT seed AS Integer = 12345
  MUT bad AS Integer = 0
  MUT huge AS Integer = 0
  FOR op = 1 TO 100000
    seed = (seed * 1103515245 + 12345) MOD 2147483648
    LET k AS Integer = seed MOD 64
    seed = (seed * 1103515245 + 12345) MOD 2147483648
    LET r AS Integer = seed MOD 10000
    MUT n AS Integer = 1 + (r MOD 200)
    IF r >= 7000 THEN n = 2100 + ((seed / 7) MOD 61000)
    IF r = 9999 THEN n = 34000000
    IF r = 9999 THEN huge = huge + 1
    LET old AS String = collections::get(slots, k)
    LET want AS Integer = collections::get(lens, k)
    LET mark AS String = collections::get(marks, k)
    IF len(old) <> want THEN bad = bad + 1
    IF want > 0 THEN
      IF strings::mid(old, 0, 1) <> mark THEN bad = bad + 1
      IF strings::mid(old, want / 2, 1) <> mark THEN bad = bad + 1
      IF strings::mid(old, want - 1, 1) <> mark THEN bad = bad + 1
    END IF
    LET c AS String = strings::mid(letters, op MOD 26, 1)
    slots = collections::set(slots, k, strings::repeat(c, n))
    lens = collections::set(lens, k, n)
    marks = collections::set(marks, k, c)
  NEXT
  io::print(\"bad \" & toString(bad))
  io::print(\"huge \" & toString(huge))
  RETURN 0
END FUNC
";

#[test]
fn a_random_mix_of_all_three_tiers_never_hands_a_chunk_out_twice() {
    let name = "arena_regions_stress";
    let exe = common::debug_report::build_debug(name, STRESS);
    let (out, lines) = run(name, &exe);
    let counter = |key: &str| common::debug_report::counter(name, &lines, 0, key);
    assert!(
        out.starts_with("bad 0\n"),
        "a live string was overwritten: {out}"
    );
    assert!(
        !out.contains("huge 0\n"),
        "the mix reached the huge tier: {out}"
    );
    for path in [
        "hit_small",
        "hit_medium_run",
        "hit_medium_frontier",
        "hit_huge_frontier",
    ] {
        assert!(counter(path) > 0, "the mix used {path}: {lines:?}");
    }
    assert_eq!(
        counter("zero_violations"),
        0,
        "a chunk was handed out with old bytes"
    );
    assert_eq!(counter("double_free_skips"), 0, "no free was refused");
    assert_eq!(counter("live_bytes"), 0, "everything was freed by the end");
}

/// plan-183-E: 40 lists of 1 MiB live at once, all freed, then 300 medium lists made
/// and dropped one at a time — every 64 medium or huge frees the arena sweeps. With
/// `PIN` the deepest level pins a medium chunk below the 40, so their 40 MiB run is off
/// the frontier; without it the run ends the medium region.
const IDLE: &str = "\
IMPORT io
IMPORT collections

MUT pin AS List OF Integer = []

FUNC hold(n AS Integer) AS Integer
  IF n = 0 THEN
    IF PIN THEN
      pin = collections::repeat([], 1, 3000)
    END IF
    RETURN 0
  END IF
  LET mine AS List OF Integer = collections::repeat([], n, 131072)
  RETURN hold(n - 1) + collections::get(mine, 0)
END FUNC

FUNC churn(n AS Integer) AS Integer
  MUT total AS Integer = 0
  MUT j AS Integer = 0
  WHILE j < n
    LET x AS List OF Integer = collections::repeat([], j, 3000)
    total = total + collections::get(x, 0)
    j = j + 1
  END WHILE
  RETURN total
END FUNC

FUNC main AS Integer
  LET held AS Integer = hold(40)
  io::print(toString(held) & \" \" & toString(churn(300)) & \" \" & toString(len(pin)))
  RETURN 0
END FUNC
";

fn idle(name: &str, pin: bool) -> Vec<String> {
    let source = IDLE.replace("PIN", if pin { "TRUE" } else { "FALSE" });
    let exe = common::debug_report::build_debug(name, &source);
    let (out, lines) = run(name, &exe);
    let pinned = if pin { 3000 } else { 0 };
    assert_eq!(
        out.trim(),
        format!("820 44850 {pinned}"),
        "{name}: the totals"
    );
    lines
}

/// The idle 40 MiB run, kept off the frontier by the pinned chunk, is decommitted by
/// the sweep: found free at one sweep, still free at the next — the medium churn takes
/// its chunks from the run's edge and gives them back, which does not make the rest of
/// it young. The exit's teardown is one decommit, so a sweep's shows as a second.
#[test]
fn an_idle_run_goes_back_to_the_os_after_two_sweeps() {
    let name = "arena_regions_idle_run";
    let lines = idle(name, true);
    let counter = |key: &str| common::debug_report::counter(name, &lines, 0, key);
    assert!(
        counter("decommits") >= 2,
        "the idle 40 MiB run was never decommitted: {lines:?}"
    );
    assert_eq!(counter("zero_violations"), 0, "{lines:?}");
}

/// Without the pin the freed 40 MiB end the medium region: two sweeps later the
/// frontier retreats over them and the commit edge follows, decommitting what it
/// leaves; the churn after it then commits again from the edge, and reads zero.
#[test]
fn a_free_run_at_the_frontier_retreats() {
    let name = "arena_regions_retreat";
    let lines = idle(name, false);
    let counter = |key: &str| common::debug_report::counter(name, &lines, 0, key);
    assert!(
        counter("decommits") >= 2,
        "the frontier never retreated over the freed run: {lines:?}"
    );
    assert_eq!(counter("zero_violations"), 0, "{lines:?}");
}

/// plan-183-E: a cold run shorter than a page, between a freed chunk and a live one.
/// `x` (8 MB) is carved just below `live` and freed: a large free, so it becomes a cold
/// run whose end touches `live`. `y`, 4 KiB smaller, is taken from that run's low end,
/// leaving a 4 KiB cold run against `live` — its whole pages none, its edge page the one
/// `live` starts in. Freeing `y` merges it with that run; the merge must decommit
/// nothing of `live`'s first page (it once decommitted up to the page boundary past the
/// tiny run, and the read of `live` faulted).
const TINY_COLD: &str = "\
IMPORT io
IMPORT strings

MUT live AS String = \"\"

FUNC stepX AS Integer
  LET x AS String = strings::repeat(\"x\", 8000000)
  RETURN len(x)
END FUNC

FUNC stepY AS Integer
  LET y AS String = strings::repeat(\"y\", 7995904)
  RETURN len(y)
END FUNC

FUNC main AS Integer
  live = strings::repeat(\"L\", 3000000)
  LET a AS Integer = stepX()
  LET b AS Integer = stepY()
  io::print(toString(a + b) & \" \" & strings::mid(live, 0, 1) & strings::mid(live, 2999999, 1) & \" \" & toString(len(live)))
  RETURN 0
END FUNC
";

#[test]
fn a_tiny_cold_run_beside_a_live_chunk_leaves_it_committed() {
    let name = "arena_regions_tiny_cold";
    let exe = common::debug_report::build_debug(name, TINY_COLD);
    let (out, lines) = run(name, &exe);
    assert_eq!(out.trim(), "15995904 LL 3000000", "{name}: the live string");
    let counter = |key: &str| common::debug_report::counter(name, &lines, 0, key);
    assert!(
        counter("decommits") >= 2,
        "the 8 MB frees did not decommit: {lines:?}"
    );
    assert_eq!(counter("zero_violations"), 0, "{lines:?}");
}
