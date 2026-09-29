//! bug-777: a failing collection callback frees everything in flight.
//!
//! A native collection lowering that calls a user function per element must,
//! when that function fails, leave like any failed call (`mfb spec memory
//! fallible-call-abi`, *Leaving a function on an error*): every block it owns is
//! freed — its working lists and the `String` element copy it made to pass to
//! the callback — and so is every owned local of the failing function.
//!
//! Before the fix, `partition` returned with a bare `ret` (its working lists and
//! the enclosing function's locals leaked, 87,888 B per failing call over 1000
//! strings), and `filter`, `transform`, `sortBy`, `forEach`, `reduce`,
//! `reduceRight`, `findLastIndex` and in-place `filter` never freed the failing
//! element's copy (its whole block, 64 B for the 45-byte element used here).
//! Output was right, so only the arena counters show it.
//!
//! Each case runs the failing statement `REPS` times, once trapped by a
//! function-level `TRAP` one frame up and once by an inline `TRAP` in the
//! failing frame, with an owned local alive across the statement. A `--debug`
//! run must end with `alloc_calls = free_calls` and `live_bytes 0`. The sites
//! the bug did not affect (in-place `transform`/`sortBy`, `mapValues`, `any`,
//! `all`, `findIndex`) stay here as guards.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const REPS: u32 = 50;

const HELPERS: &str = "\
IMPORT collections
IMPORT io

' The failing element: 45 bytes, so its copy is a 64-byte block.
FUNC longKey() AS String
  RETURN \"fail-fail-fail-fail-fail-fail-fail-fail-fail!\"
END FUNC

FUNC strs() AS List OF String
  MUT xs AS List OF String = []
  FOR i = 0 TO 19
    IF i = 10 THEN
      xs = collections::append(xs, longKey())
    ELSE
      xs = collections::append(xs, \"s\" & toString(i))
    END IF
  NEXT
  RETURN xs
END FUNC

FUNC ints() AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO 19
    xs = collections::append(xs, i)
  NEXT
  RETURN xs
END FUNC

FUNC strMap() AS Map OF Integer TO String
  MUT m AS Map OF Integer TO String = Map OF Integer TO String {}
  FOR i = 0 TO 19
    IF i = 10 THEN
      m = collections::set(m, i, longKey())
    ELSE
      m = collections::set(m, i, \"s\" & toString(i))
    END IF
  NEXT
  RETURN m
END FUNC

FUNC predFail(s AS String) AS Boolean
  IF s = longKey() THEN
    FAIL error(77050002, \"boom\")
  END IF
  RETURN FALSE
END FUNC

FUNC predFailTrue(s AS String) AS Boolean
  IF s = longKey() THEN
    FAIL error(77050002, \"boom\")
  END IF
  RETURN TRUE
END FUNC

FUNC intPredFail(n AS Integer) AS Boolean
  IF n = 10 THEN
    FAIL error(77050002, \"boom\")
  END IF
  RETURN n > 5
END FUNC

FUNC mapFail(s AS String) AS String
  IF s = longKey() THEN
    FAIL error(77050002, \"boom\")
  END IF
  RETURN s & \"!\"
END FUNC

FUNC keyFail(s AS String) AS Integer
  IF s = longKey() THEN
    FAIL error(77050002, \"boom\")
  END IF
  RETURN len(s)
END FUNC

FUNC accFail(total AS Integer, s AS String) AS Integer
  IF s = longKey() THEN
    FAIL error(77050002, \"boom\")
  END IF
  RETURN total + len(s)
END FUNC

FUNC concatFail(total AS String, s AS String) AS String
  IF s = longKey() THEN
    FAIL error(77050002, \"boom\")
  END IF
  RETURN total & s
END FUNC

FUNC eachFail(s AS String) AS Nothing
  IF s = longKey() THEN
    FAIL error(77050002, \"boom\")
  END IF
END FUNC
";

/// `(name, statement)`: the failing statement. `xs` is the String list, `ixs`
/// the Integer list, `m` the String-valued map, `ys`/`mm` mutable copies for the
/// in-place forms. A statement is a `LET` initializer or a bare call.
const CASES: &[(&str, &str)] = &[
    (
        "partition_string",
        "LET r AS Partition OF String = collections::partition(xs, predFail)",
    ),
    (
        "partition_integer",
        "LET r AS Partition OF Integer = collections::partition(ixs, intPredFail)",
    ),
    (
        "filter",
        "LET r AS List OF String = collections::filter(xs, predFail)",
    ),
    (
        "transform",
        "LET r AS List OF String = collections::transform(xs, mapFail)",
    ),
    (
        "groupBy",
        "LET r AS Map OF Integer TO List OF String = collections::groupBy(xs, keyFail, mapFail)",
    ),
    (
        "sortBy",
        "LET r AS List OF String = collections::sortBy(xs, keyFail)",
    ),
    (
        "reduce",
        "LET r AS Integer = collections::reduce(xs, 0, accFail)",
    ),
    (
        // The accumulator built over the first ten elements is the loop's own
        // block when the reducer fails; it leaked with the item.
        "reduce_string_accumulator",
        "LET r AS String = collections::reduce(xs, \"\", concatFail)",
    ),
    (
        "reduceRight",
        "LET r AS Integer = collections::reduceRight(xs, 0, accFail)",
    ),
    ("forEach", "collections::forEach(xs, eachFail)"),
    (
        "findLastIndex",
        "LET r AS Integer = collections::findLastIndex(xs, predFail)",
    ),
    ("inplace_filter", "ys = collections::filter(ys, predFail)"),
    // Guards: measured 0 before the fix.
    ("inplace_transform", "ys = collections::transform(ys, mapFail)"),
    ("inplace_sortBy", "ys = collections::sortBy(ys, keyFail)"),
    (
        "mapValues",
        "LET r AS Map OF Integer TO String = collections::mapValues(m, mapFail)",
    ),
    ("inplace_mapValues", "mm = collections::mapValues(mm, mapFail)"),
    (
        "any",
        "LET r AS Boolean = collections::any(xs, predFail)",
    ),
    (
        "all",
        "LET r AS Boolean = collections::all(xs, predFailTrue)",
    ),
    (
        "findIndex",
        "LET r AS Integer = collections::findIndex(xs, predFail, 0)",
    ),
];

/// The failing statement, trapped by an inline `TRAP` when `inline`.
fn trapped(statement: &str, inline: bool) -> String {
    if inline {
        format!("{statement} TRAP(e)\n    RETURN -1\n  END TRAP")
    } else {
        statement.to_string()
    }
}

fn program(statement: &str, inline: bool) -> String {
    format!(
        "{HELPERS}
FUNC doOp(xs AS List OF String, ixs AS List OF Integer, m AS Map OF Integer TO String) AS Integer
  LET extra AS List OF String = strs()
  MUT ys AS List OF String = xs
  MUT mm AS Map OF Integer TO String = m
  {}
  RETURN len(extra) + len(ys) + len(mm)
END FUNC

FUNC tryOnce(xs AS List OF String, ixs AS List OF Integer, m AS Map OF Integer TO String) AS Integer
  RETURN doOp(xs, ixs, m)
  TRAP(err)
    RETURN -1
  END TRAP
END FUNC

FUNC main() AS Integer
  LET xs AS List OF String = strs()
  LET ixs AS List OF Integer = ints()
  LET m AS Map OF Integer TO String = strMap()
  MUT acc AS Integer = 0
  FOR k = 1 TO {REPS}
    acc = acc + tryOnce(xs, ixs, m)
  NEXT
  io::print(toString(acc))
  RETURN 0
END FUNC
",
        trapped(statement, inline)
    )
}

/// Build with `--debug`, run, and return `(stdout, alloc_calls, free_calls,
/// live_bytes)` summed over the arenas.
fn run(name: &str, source: &str) -> (String, u64, u64, u64) {
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
        "{name}: build failed:\n{stdout}\n{}",
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
    let sum = |key: &str| -> u64 {
        report
            .lines()
            .filter_map(|line| line.strip_prefix("arena."))
            .filter_map(|rest| rest.split_once(key))
            .filter(|(arena, _)| !arena.contains('.'))
            .map(|(_, n)| n.trim().parse::<u64>().expect("a count"))
            .sum()
    };
    (
        text,
        sum(".alloc_calls "),
        sum(".free_calls "),
        sum(".live_bytes "),
    )
}

#[test]
fn a_failing_callback_frees_everything_in_flight() {
    let mut failures = Vec::new();
    for (member, statement) in CASES {
        for inline in [false, true] {
            let mode = if inline { "inline" } else { "function" };
            let name = format!("b777_{member}_{mode}");
            let (text, alloc, free, live) = run(&name, &program(statement, inline));
            let want = format!("-{REPS}\n");
            if text != want {
                failures.push(format!(
                    "{member} ({mode} TRAP): the callback failure was not trapped every time: \
                     printed {text:?}, want {want:?}"
                ));
            } else if alloc != free || live != 0 {
                failures.push(format!(
                    "{member} ({mode} TRAP): leaked {} block(s), {live} B live over {REPS} \
                     failing calls",
                    alloc as i64 - free as i64
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "bug-777: a failing collection callback must free every block in flight:\n  {}",
        failures.join("\n  ")
    );
}
