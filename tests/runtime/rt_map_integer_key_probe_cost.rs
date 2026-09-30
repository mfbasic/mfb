//! bug-770: an `Integer`-key `Map` lookup and insert cost about what a `List`
//! access costs, and a `Map` list-value read-modify-write allocates twice per
//! iteration.
//!
//! Every Map/Set probe treated an `Integer` key as 8 bytes: an 8-step byte-at-a-
//! time FNV-1a, a 64-bit `udiv` for `mod 2*capacity`, and a byte-compare loop. A
//! new-key `set` hashed the key about five times (inline probe, fallback
//! re-probe, `_mfb_rt_map_bucket_put`, and the rebuild after every grow), and
//! FNV mod a non-power-of-two clustered to 3.7 probes per hit at n = 5,000. And
//! `getOr`'s owned copy was tight, so the `append` right after it always
//! reallocated.
//!
//! The cost tests time, in-process, a `Map` against a same-binary `List`
//! control (best of five), so the bound holds on a loaded host. Each program
//! also folds its results into a checksum the test compares.
//!
//! `every_hash_site_agrees_across_grows_and_removals` is green before the fix
//! and must stay green: the index is only correct if every probe, bucket write,
//! rebuild and backward-shift delete computes the same home slot, and a site that
//! disagrees makes a present key silently unreachable.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const SIZES: [u64; 4] = [2_000, 3_000, 5_000, 20_000];

fn run_program(name: &str, source: &str, debug: bool) -> (String, String) {
    let project = common::temp_project(name, source);
    let mut build = Command::new(common::mfb_exe());
    build.arg("build");
    if debug {
        build.arg("--debug");
    }
    let output = build.arg(&project).output().expect("run mfb build");
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
    (text, report)
}

/// Per size: `n mapSet mapGet listAppend listGet mapChecksum listChecksum`, the
/// four timings the best of five, in ns for the whole loop.
fn timing_program() -> String {
    let calls: String = SIZES.iter().map(|n| format!("  runOne({n})\n")).collect();
    format!(
        "IMPORT io
IMPORT collections
IMPORT datetime

FUNC runOne(n AS Integer) AS Nothing
  MUT bb AS Integer = 999999999999
  MUT bg AS Integer = 999999999999
  MUT la AS Integer = 999999999999
  MUT lg AS Integer = 999999999999
  MUT mapChk AS Integer = 0
  MUT listChk AS Integer = 0
  FOR r = 0 TO 4
    LET t0 AS Integer = datetime::monotonicNanos()
    MUT m AS Map OF Integer TO Integer = Map OF Integer TO Integer {{}}
    FOR i = 0 TO n - 1
      m = collections::set(m, i, i)
    NEXT
    LET t1 AS Integer = datetime::monotonicNanos()
    MUT s AS Integer = 0
    FOR i = 0 TO n - 1
      s = s + collections::get(m, i)
    NEXT
    LET t2 AS Integer = datetime::monotonicNanos()
    MUT xs AS List OF Integer = []
    FOR i = 0 TO n - 1
      xs = collections::append(xs, i)
    NEXT
    LET t3 AS Integer = datetime::monotonicNanos()
    MUT u AS Integer = 0
    FOR i = 0 TO n - 1
      u = u + collections::get(xs, i)
    NEXT
    LET t4 AS Integer = datetime::monotonicNanos()
    mapChk = s + len(m)
    listChk = u + len(xs)
    IF t1 - t0 < bb THEN
      bb = t1 - t0
    END IF
    IF t2 - t1 < bg THEN
      bg = t2 - t1
    END IF
    IF t3 - t2 < la THEN
      la = t3 - t2
    END IF
    IF t4 - t3 < lg THEN
      lg = t4 - t3
    END IF
  NEXT
  io::print(toString(n) & \" \" & toString(bb) & \" \" & toString(bg) & \" \" & toString(la) & \" \" & toString(lg) & \" \" & toString(mapChk) & \" \" & toString(listChk))
END FUNC

FUNC main() AS Integer
{calls}  RETURN 0
END FUNC
"
    )
}

/// `(n, map set, map get, list append, list get)` in ns for the whole loop.
fn timings() -> Vec<(u64, f64, f64, f64, f64)> {
    let (text, _) = run_program("b770_probe_cost", &timing_program(), false);
    text.lines()
        .map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(f.len(), 7, "bad line {line:?} in\n{text}");
            assert_eq!(
                f[5], f[6],
                "the map and list checksums disagree at n={}: a probe lost a key\n{text}",
                f[0]
            );
            let p = |i: usize| f[i].parse::<f64>().expect("a number");
            (f[0].parse().expect("n"), p(1), p(2), p(3), p(4))
        })
        .collect()
}

#[test]
fn an_integer_key_lookup_costs_within_2_5x_a_list_get() {
    let rows = timings();
    let mut failures = Vec::new();
    let mut per_get = Vec::new();
    for (n, _, map_get, _, list_get) in &rows {
        let ratio = map_get / list_get;
        eprintln!(
            "n={n}: map get {:.1} ns, list get {:.1} ns, ×{ratio:.2}",
            map_get / *n as f64,
            list_get / *n as f64
        );
        per_get.push(map_get / *n as f64);
        if ratio > 2.5 {
            failures.push(format!("n={n}: map get ×{ratio:.2} a list get (bound ×2.5)"));
        }
    }
    let max = per_get.iter().cloned().fold(f64::MIN, f64::max);
    let min = per_get.iter().cloned().fold(f64::MAX, f64::min);
    if max / min > 1.5 {
        failures.push(format!(
            "map get is not flat in n: {min:.1}–{max:.1} ns, ×{:.2} (bound ×1.5)",
            max / min
        ));
    }
    assert!(failures.is_empty(), "bug-770:\n  {}", failures.join("\n  "));
}

#[test]
fn building_an_integer_key_map_costs_within_3_5x_list_appends() {
    let rows = timings();
    let (n, map_set, _, list_append, _) = *rows.last().expect("a row");
    let ratio = map_set / list_append;
    eprintln!(
        "n={n}: map set {:.1} ns, list append {:.1} ns, ×{ratio:.2}",
        map_set / n as f64,
        list_append / n as f64
    );
    assert!(
        ratio <= 3.5,
        "bug-770: building a {n}-key `Map OF Integer TO Integer` by `set` took ×{ratio:.2} the \
         same number of `List` appends (bound ×3.5)"
    );
}

#[test]
fn a_list_value_read_modify_write_allocates_twice_per_iteration() {
    let program = |n: u64| {
        format!(
            "IMPORT io
IMPORT collections

FUNC agg(n AS Integer) AS Integer
  MUT m AS Map OF Integer TO List OF Integer = Map OF Integer TO List OF Integer {{}}
  FOR i = 0 TO n - 1
    LET key AS Integer = i MOD 64
    LET empty AS List OF Integer = []
    MUT bucket AS List OF Integer = collections::getOr(m, key, empty)
    bucket = collections::append(bucket, i)
    m = collections::set(m, key, bucket)
  NEXT
  MUT acc AS Integer = 0
  LET none AS List OF Integer = []
  FOR k = 0 TO 63
    LET b AS List OF Integer = collections::getOr(m, k, none)
    acc = acc + (k + 1) * len(b) + collections::sum(b)
  NEXT
  RETURN acc
END FUNC

FUNC main() AS Integer
  io::print(toString(agg({n})))
  RETURN 0
END FUNC
"
        )
    };
    let count = |report: &str| -> u64 {
        report
            .lines()
            .find_map(|line| line.strip_prefix("arena.0.alloc_calls "))
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or_else(|| panic!("no arena.0.alloc_calls in\n{report}"))
    };
    let (text_a, report_a) = run_program("b770_agg_2000", &program(2_000), true);
    let (text_b, report_b) = run_program("b770_agg_4000", &program(4_000), true);
    assert_eq!(text_a, "2063616\n", "agg(2000) checksum");
    assert_eq!(text_b, "8127488\n", "agg(4000) checksum");
    let per_iteration = (count(&report_b) - count(&report_a)) as f64 / 2_000.0;
    assert!(
        per_iteration <= 2.0,
        "bug-770 C: the `getOr` -> `append` -> `set` bucket update made {per_iteration:.2} \
         arena allocations per iteration (bound 2): the owned copy `append` grows is tight"
    );
}

#[test]
fn every_hash_site_agrees_across_grows_and_removals() {
    // For each key pattern and size: insert every key, remove every third (the
    // backward-shift delete), re-insert every ninth, then check membership and
    // value of every key and of absent neighbours. Expected answers are computed
    // from the index alone, so no oracle container is needed.
    let source = "IMPORT io
IMPORT collections

FUNC keyOf(pattern AS Integer, i AS Integer) AS Integer
  IF pattern = 0 THEN
    RETURN i
  END IF
  IF pattern = 1 THEN
    RETURN i * 2
  END IF
  IF pattern = 2 THEN
    RETURN i * 1024
  END IF
  IF pattern = 3 THEN
    RETURN 0 - i - 1
  END IF
  IF pattern = 4 THEN
    RETURN 9223372036854775807 - i
  END IF
  RETURN -9223372036854775807 - 1 + i
END FUNC

FUNC present(i AS Integer) AS Boolean
  RETURN i MOD 3 <> 0 OR i MOD 9 = 0
END FUNC

FUNC checkInt(pattern AS Integer, n AS Integer) AS Integer
  MUT m AS Map OF Integer TO Integer = Map OF Integer TO Integer {}
  MUT s AS Set OF Integer = Set OF Integer {}
  FOR i = 0 TO n - 1
    m = collections::set(m, keyOf(pattern, i), i)
    s = collections::add(s, keyOf(pattern, i))
  NEXT
  FOR i = 0 TO n - 1
    IF i MOD 3 = 0 THEN
      m = collections::removeKey(m, keyOf(pattern, i))
      s = collections::remove(s, keyOf(pattern, i))
    END IF
  NEXT
  FOR i = 0 TO n - 1
    IF i MOD 9 = 0 THEN
      m = collections::set(m, keyOf(pattern, i), i)
      s = collections::add(s, keyOf(pattern, i))
    END IF
  NEXT
  MUT bad AS Integer = 0
  FOR i = 0 TO n - 1
    LET k AS Integer = keyOf(pattern, i)
    IF collections::hasKey(m, k) <> present(i) THEN
      bad = bad + 1
    END IF
    IF collections::contains(s, k) <> present(i) THEN
      bad = bad + 1
    END IF
    IF present(i) AND collections::getOr(m, k, -1) <> i THEN
      bad = bad + 1
    END IF
  NEXT
  IF len(m) <> len(s) THEN
    bad = bad + 1
  END IF
  RETURN bad
END FUNC

FUNC checkFloat(n AS Integer) AS Integer
  MUT m AS Map OF Float TO Integer = Map OF Float TO Integer {}
  LET negZero AS Float = 0.0 * (0.0 - 1.0)
  m = collections::set(m, 0.0, -10)
  m = collections::set(m, negZero, -20)
  FOR i = 1 TO n
    m = collections::set(m, toFloat(i) * 0.5, i)
  NEXT
  FOR i = 1 TO n
    IF i MOD 3 = 0 THEN
      m = collections::removeKey(m, toFloat(i) * 0.5)
    END IF
  NEXT
  MUT bad AS Integer = 0
  IF collections::getOr(m, 0.0, 0) <> -10 THEN
    bad = bad + 1
  END IF
  IF collections::getOr(m, negZero, 0) <> -20 THEN
    bad = bad + 1
  END IF
  FOR i = 1 TO n
    LET want AS Boolean = i MOD 3 <> 0
    IF collections::hasKey(m, toFloat(i) * 0.5) <> want THEN
      bad = bad + 1
    END IF
    IF want AND collections::getOr(m, toFloat(i) * 0.5, 0) <> i THEN
      bad = bad + 1
    END IF
    IF collections::hasKey(m, toFloat(i) * 0.5 + 0.25) THEN
      bad = bad + 1
    END IF
  NEXT
  RETURN bad
END FUNC

FUNC checkFixed(n AS Integer) AS Integer
  MUT m AS Map OF Fixed TO Integer = Map OF Fixed TO Integer {}
  FOR i = 0 TO n - 1
    m = collections::set(m, toFixed(i) * toFixed(0.25), i)
  NEXT
  FOR i = 0 TO n - 1
    IF i MOD 3 = 0 THEN
      m = collections::removeKey(m, toFixed(i) * toFixed(0.25))
    END IF
  NEXT
  MUT bad AS Integer = 0
  FOR i = 0 TO n - 1
    LET want AS Boolean = i MOD 3 <> 0
    IF collections::hasKey(m, toFixed(i) * toFixed(0.25)) <> want THEN
      bad = bad + 1
    END IF
    IF want AND collections::getOr(m, toFixed(i) * toFixed(0.25), -1) <> i THEN
      bad = bad + 1
    END IF
  NEXT
  RETURN bad
END FUNC

FUNC main() AS Integer
  MUT bad AS Integer = 0
  MUT checked AS Integer = 0
  FOR EACH n IN [1, 5, 17, 100, 1025, 1537, 3000, 6000]
    FOR pattern = 0 TO 5
      bad = bad + checkInt(pattern, n)
      checked = checked + 1
    NEXT
    bad = bad + checkFloat(n) + checkFixed(n)
    checked = checked + 2
  NEXT
  io::print(toString(checked) & \" checks, \" & toString(bad) & \" wrong\")
  RETURN 0
END FUNC
";
    let (text, _) = run_program("b770_hash_sites_agree", source, false);
    assert_eq!(text, "64 checks, 0 wrong\n");
}
