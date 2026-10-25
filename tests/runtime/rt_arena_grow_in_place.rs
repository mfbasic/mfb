//! A growing collection extends in place instead of being copied (plan-183-D).
//!
//! The huge region (chunks of 32 MiB or more) grows up, so the newest huge chunk ends
//! at the region's frontier and `arena_extend` grows it there. A collection below
//! 32 MiB is a medium chunk; the medium region grows down, so its growth copies (as it
//! always did), and the copy leaves less than 32 MiB extra live at its peak. When the
//! collection reaches 32 MiB it moves into the huge region once, and from then on no
//! growth step holds two copies: the peak live bytes are the final ones plus whatever
//! the program itself holds at once.
//!
//! The collection is a top-level variable, so it is still live when the `--debug`
//! report is written, and the report's `live_bytes` is its final size.
//!
//! Set `MFB_TEST_REMOTE` to run the programs on another box (`tests/common/remote.rs`).

#[path = "../common/mod.rs"]
mod common;

/// One `--debug` run of `source`: the arena-0 counters named in `names`.
fn counters(name: &str, source: &str, names: &[&str]) -> Vec<u64> {
    let exe = common::debug_report::build_debug(name, source);
    let output = common::remote::output(&exe, &[], &[]);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{name}: {stdout}\n{stderr}");
    let lines = common::debug_report::arena_lines(name, &stderr);
    names
        .iter()
        .map(|counter| common::debug_report::counter(name, &lines, 0, counter))
        .collect()
}

const MIB: u64 = 1 << 20;
/// A data commit step (`ARENA_COMMIT_STEP`).
const COMMIT_STEP: u64 = 4 * MIB;

/// 41 records appended one at a time, each holding a 1.6 MB `List OF Float` (200,000
/// floats) inline — the shape of weather's forecast list. The list of records crosses
/// 32 MiB part way and is moved into the huge region exactly once (`hit_huge_frontier`
/// 1); every later step extends it (`extend_hits`), so the peak is the final size plus
/// two records: `day` holds its list and the record built around it at once (measured:
/// final 82,076,848 B, peak 85,276,960 B — before plan-183-D the peak was
/// 138,394,832 B).
const RECORDS: &str = "\
IMPORT io
IMPORT collections

TYPE Day
  hour AS Integer
  samples AS List OF Float
END TYPE

FUNC day(n AS Integer) AS Day
  MUT xs AS List OF Float = []
  xs = collections::repeat(xs, toFloat(n), 200000)
  RETURN Day[hour := n, samples := xs]
END FUNC

MUT days AS List OF Day = []

FUNC main AS Integer
  MUT i AS Integer = 0
  WHILE i < 41
    days = collections::append(days, day(i))
    i = i + 1
  END WHILE
  io::print(toString(len(days)) & \" \" & toString(collections::get(collections::get(days, 40).samples, 199999)))
  RETURN 0
END FUNC
";

#[test]
fn a_list_of_records_moves_into_the_huge_region_once_and_then_grows_in_place() {
    let record = 200_000 * 8;
    let [final_live, peak, crossings, hits] = counters(
        "arena_grow_records",
        RECORDS,
        &[
            "live_bytes",
            "peak_live_bytes",
            "hit_huge_frontier",
            "extend_hits",
        ],
    )[..] else {
        unreachable!()
    };
    assert!(
        final_live > 41 * record,
        "the list's final size {final_live} cannot hold its 41 records"
    );
    assert_eq!(
        crossings, 1,
        "the list moved into the huge region {crossings} times; once, then it extends"
    );
    assert!(
        hits >= 1,
        "no growth step extended in place ({hits} extend hits)"
    );
    assert!(
        peak <= final_live + 2 * record + MIB,
        "peak live {peak} exceeds the final {final_live} by more than the two records \
         `day` holds at once and 1 MiB — a growth step held two copies of the list"
    );
}

/// One `List OF Float` appended to 2^25 elements (256 MiB of data), one element at a
/// time: below 32 MiB it copies at each geometric step, crosses into the huge region
/// once, and then extends at every step. The peak is the final capacity (measured:
/// 310,282,496 B; before plan-183-D 517,137,296 B).
const FLOATS: &str = "\
IMPORT io
IMPORT collections

MUT xs AS List OF Float = []

FUNC main AS Integer
  MUT i AS Integer = 0
  WHILE i < 33554432
    xs = collections::append(xs, toFloat(i))
    i = i + 1
  END WHILE
  io::print(toString(len(xs)) & \" \" & toString(collections::get(xs, 33554431)))
  RETURN 0
END FUNC
";

#[test]
fn a_huge_list_grows_in_place_at_every_step_after_the_tier_crossing() {
    let [final_live, peak, crossings, hits, calls] = counters(
        "arena_grow_floats",
        FLOATS,
        &[
            "live_bytes",
            "peak_live_bytes",
            "hit_huge_frontier",
            "extend_hits",
            "extend_calls",
        ],
    )[..] else {
        unreachable!()
    };
    assert!(
        final_live >= 256 * MIB,
        "the list's final size {final_live} cannot hold 2^25 floats"
    );
    assert_eq!(
        crossings, 1,
        "the list moved into the huge region {crossings} times; once, then it extends"
    );
    // 32 MiB to 256 MiB in steps of x1.5 is at least five steps, every one a hit.
    assert!(
        hits >= 5,
        "{hits} of {calls} growth steps extended in place; every huge step should"
    );
    assert!(
        peak <= final_live + COMMIT_STEP,
        "peak live {peak} exceeds the final {final_live} by more than one commit step — \
         a huge growth step held two copies of the list"
    );
}
