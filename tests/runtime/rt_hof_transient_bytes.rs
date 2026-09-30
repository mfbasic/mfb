//! `collections::partition` and `collections::groupBy` hand the allocator about the
//! bytes their result needs, not a multiple of the input.
//!
//! bug-771: `partition` reserved both output lists at the full source size, then
//! copied both into the `Partition` record headroom included (so the result was
//! twice its content); `groupBy` sized its hash scratch by element count (a 32 768-
//! slot table for 7 keys), grew each bucket geometrically from one element, and
//! copied every bucket into a map that also grew from empty. Every freed byte is
//! then scrubbed by the arena's entropy fill.
//!
//! Each case runs its call and a checksum of the result `N` and `2N` times under
//! `--debug`, and a control runs only the checksum (over one result computed
//! before the loop). The added `arena.0.alloc_bytes` per call, less the
//! checksum's own, must stay within the case's budget; and the printed checksum
//! (the result's contents, element by element) must be the one the call has
//! always produced.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const HELPERS: &str = "\
IMPORT io
IMPORT collections

FUNC strIsShort(s AS String) AS Boolean
  RETURN len(s) <= 2
END FUNC
FUNC isEvenN(n AS Integer) AS Boolean
  RETURN n - (n / 2) * 2 = 0
END FUNC
FUNC strLenKey(s AS String) AS Integer
  RETURN len(s)
END FUNC
FUNC strIdentity(s AS String) AS String
  RETURN s
END FUNC
FUNC buildStrRange(n AS Integer) AS List OF String
  MUT xs AS List OF String = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, \"s\" & toString(i))
  NEXT
  RETURN xs
END FUNC
FUNC buildRange(n AS Integer) AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, i)
  NEXT
  RETURN xs
END FUNC
FUNC strSum(xs AS List OF String) AS Integer
  MUT t AS Integer = 0
  FOR i = 0 TO len(xs) - 1
    t = (t * 31 + len(collections::get(xs, i))) MOD 1000000007
  NEXT
  RETURN t
END FUNC
FUNC intSum(xs AS List OF Integer) AS Integer
  MUT t AS Integer = 0
  FOR EACH n IN xs
    t = (t * 31 + n) MOD 1000000007
  NEXT
  RETURN t
END FUNC
";

/// `(name, setup, produce, checksum, budget)`: `setup` binds the input once;
/// `produce` binds one call's result as `r`; `checksum` adds a checksum of `r` to
/// `acc`; `budget` is the most `alloc_bytes` one call may add.
const CASES: &[(&str, &str, &str, &str, u64)] = &[
    (
        "partition_string_1000",
        "LET xs AS List OF String = buildStrRange(1000)",
        "LET r AS Partition OF String = collections::partition(xs, strIsShort)",
        "acc = acc + strSum(r.matched) * 7 + strSum(r.unmatched) + len(r.matched)",
        60_000,
    ),
    (
        "partition_integer_1000",
        "LET xs AS List OF Integer = buildRange(1000)",
        "LET r AS Partition OF Integer = collections::partition(xs, isEvenN)",
        "acc = acc + intSum(r.matched) * 7 + intSum(r.unmatched) + len(r.matched)",
        18_000,
    ),
    (
        "group_by_string_2000_to_4",
        "LET xs AS List OF String = buildStrRange(2000)",
        "LET r AS Map OF Integer TO List OF String = collections::groupBy(xs, strLenKey, strIdentity)",
        "FOR EACH key IN collections::keys(r)
      acc = (acc * 13 + key * 1000 + strSum(collections::get(r, key))) MOD 1000000007
    NEXT",
        330_000,
    ),
    (
        "group_by_integer_10000_to_7",
        "LET xs AS List OF Integer = buildRange(10000)",
        "LET r AS Map OF Integer TO List OF Integer = collections::groupBy(xs, LAMBDA(n AS Integer) -> n MOD 7, LAMBDA(n AS Integer) -> n)",
        "FOR EACH key IN collections::keys(r)
      acc = (acc * 13 + key * 1000 + intSum(collections::get(r, key))) MOD 1000000007
    NEXT",
        400_000,
    ),
];

const N: u32 = 10;

/// One call and its checksum per iteration.
fn program(setup: &str, produce: &str, checksum: &str, reps: u32) -> String {
    format!(
        "{HELPERS}
FUNC main() AS Integer
  {setup}
  MUT acc AS Integer = 0
  FOR k = 1 TO {reps}
    {produce}
    {checksum}
  NEXT
  io::print(toString(acc))
  RETURN 0
END FUNC
"
    )
}

/// The control: the call runs once, before the loop, and only the checksum
/// repeats — what the checksum itself allocates, to subtract.
fn control(setup: &str, produce: &str, checksum: &str, reps: u32) -> String {
    format!(
        "{HELPERS}
FUNC main() AS Integer
  {setup}
  {produce}
  MUT acc AS Integer = 0
  FOR k = 1 TO {reps}
    {checksum}
  NEXT
  io::print(toString(acc))
  RETURN 0
END FUNC
"
    )
}

/// Build with `--debug`, run, and return `(stdout, alloc_bytes)`.
fn run(name: &str, source: &str) -> (String, u64) {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name}: build failed:\n{stdout}\n{}\n--- source ---\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exe = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .find(|p| p.ends_with("-glibc.out"))
        .or_else(|| {
            stdout
                .lines()
                .find_map(|line| line.strip_prefix("Wrote executable to "))
        })
        .expect("an executable")
        .to_string();
    let out = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let report = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "{name}: program failed:\n{text}\n{report}"
    );
    let bytes = report
        .lines()
        .find_map(|line| line.strip_prefix("arena.0.alloc_bytes "))
        .and_then(|n| n.trim().parse::<u64>().ok())
        .unwrap_or_else(|| panic!("{name}: no arena.0.alloc_bytes in\n{report}"));
    (text, bytes)
}

/// The checksum each case prints after `N` and `2N` calls — the result's
/// contents, as every compiler before bug-771's fix produced them.
fn expected(name: &str) -> (&'static str, &'static str) {
    match name {
        "partition_string_1000" => ("63158107240\n", "126316214480\n"),
        "partition_integer_1000" => ("45103258770\n", "90206517540\n"),
        "group_by_string_2000_to_4" => ("163259408\n", "848168075\n"),
        "group_by_integer_10000_to_7" => ("875585095\n", "950876946\n"),
        _ => unreachable!("{name}"),
    }
}

#[test]
fn partition_and_group_by_allocate_about_their_result() {
    let mut failures = Vec::new();
    for (name, setup, produce, checksum, budget) in CASES {
        let (once_text, once) = run(
            &format!("hofbytes_{name}_1"),
            &program(setup, produce, checksum, N),
        );
        let (twice_text, twice) = run(
            &format!("hofbytes_{name}_2"),
            &program(setup, produce, checksum, 2 * N),
        );
        let (want_once, want_twice) = expected(name);
        if once_text != want_once || twice_text != want_twice {
            failures.push(format!(
                "{name}: checksums {once_text:?} / {twice_text:?}, want {want_once:?} / {want_twice:?}"
            ));
            continue;
        }
        let (_, control_once) = run(
            &format!("hofbytes_{name}_c1"),
            &control(setup, produce, checksum, N),
        );
        let (_, control_twice) = run(
            &format!("hofbytes_{name}_c2"),
            &control(setup, produce, checksum, 2 * N),
        );
        let call_and_checksum = twice.saturating_sub(once);
        let checksum_only = control_twice.saturating_sub(control_once);
        let per_call = call_and_checksum.saturating_sub(checksum_only) / u64::from(N);
        if per_call > *budget {
            failures.push(format!(
                "{name}: {per_call} B allocated per call, budget {budget} B"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "partition/groupBy must allocate about their result:\n  {}",
        failures.join("\n  ")
    );
}

/// A failing `valFn` frees the finished `keys` projection. `groupBy` runs `keyFn`
/// over the whole list, then `valFn`; the second transform's failure exit freed
/// only its own output, so under a function-level `TRAP` the `keys` list leaked
/// (8,048 B per failing call over 1,000 Integers), found while fixing bug-771.
#[test]
fn a_failing_value_callback_frees_the_key_projection() {
    let source = "IMPORT io
IMPORT collections

FUNC mod5(n AS Integer) AS Integer
  RETURN n MOD 5
END FUNC
FUNC valOrBoom(n AS Integer) AS Integer
  IF n = 700 THEN FAIL error(77050003, \"val boom\")
  RETURN n * 2
END FUNC
FUNC sValOrBoom(n AS Integer) AS String
  IF n = 700 THEN FAIL error(77050004, \"sval boom\")
  RETURN \"v\" & toString(n)
END FUNC
FUNC fnVal(xs AS List OF Integer) AS Integer
  LET m AS Map OF Integer TO List OF Integer = collections::groupBy(xs, mod5, valOrBoom)
  RETURN len(m)
  TRAP(err)
    RETURN -1
  END TRAP
END FUNC
FUNC fnSVal(xs AS List OF Integer) AS Integer
  LET m AS Map OF Integer TO List OF String = collections::groupBy(xs, mod5, sValOrBoom)
  RETURN len(m)
  TRAP(err)
    RETURN -1
  END TRAP
END FUNC

FUNC main() AS Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO 999
    xs = collections::append(xs, i)
  NEXT
  MUT acc AS Integer = 0
  FOR k = 1 TO 20
    acc = acc + fnVal(xs) + fnSVal(xs)
    LET b AS Integer = len(collections::groupBy(xs, mod5, valOrBoom)) TRAP(e)
      RECOVER -1
    END TRAP
    acc = acc + b
  NEXT
  io::print(toString(acc))
  RETURN 0
END FUNC
";
    let project = common::temp_project("hofbytes_groupby_valfn_fail", source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "build failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exe = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("an executable")
        .to_string();
    let out = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let report = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(text, "-60\n", "{report}");
    let counter = |key: &str| -> u64 {
        report
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|n| n.trim().parse::<u64>().ok())
            .unwrap_or_else(|| panic!("no {key} in\n{report}"))
    };
    assert_eq!(counter("arena.0.double_free_skips "), 0, "{report}");
    assert_eq!(
        counter("arena.0.alloc_calls "),
        counter("arena.0.free_calls "),
        "a failing valFn leaked (live_bytes {})",
        counter("arena.0.live_bytes ")
    );
}
