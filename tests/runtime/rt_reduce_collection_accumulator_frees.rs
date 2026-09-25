//! `collections::reduce`/`reduceRight` free a superseded collection accumulator.
//!
//! The native fold reclaims the accumulator it replaces each step (plan-86-B) —
//! but only for a `String` accumulator. With a `List`/`Map`/`Set` accumulator
//! every superseded block leaked: `reduce(xs, [0], keepAcc)` over 1000 elements
//! allocated 1012 blocks and freed 13, leaving 47,952 bytes live at exit. Found by
//! plan-142-E's peak-live measurement of `x = reduce(x, seed, keepAcc)`. Each
//! program must end with every block it allocated freed, and print the fold's
//! correct result.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const HELPERS: &str = "\
IMPORT collections
IMPORT io

FUNC keepAcc(acc AS List OF Integer, n AS Integer) AS List OF Integer
  RETURN acc
END FUNC

FUNC addOne(acc AS List OF Integer, n AS Integer) AS List OF Integer
  RETURN collections::append(acc, n)
END FUNC

FUNC freshEach(acc AS List OF Integer, n AS Integer) AS List OF Integer
  RETURN [n]
END FUNC

FUNC countKeys(acc AS Map OF String TO Integer, n AS Integer) AS Map OF String TO Integer
  RETURN collections::set(acc, toString(n MOD 7), n)
END FUNC

FUNC keepSet(acc AS Set OF Integer, n AS Integer) AS Set OF Integer
  RETURN collections::add(acc, n MOD 5)
END FUNC
";

/// `(name, fold statement printing a result, expected output)`.
const CASES: &[(&str, &str, &str)] = &[
    (
        "keep",
        "LET r AS List OF Integer = collections::reduce(xs, seed, keepAcc)\n  io::print(toString(len(r)))",
        "1",
    ),
    (
        "append",
        "LET r AS List OF Integer = collections::reduce(xs, seed, addOne)\n  io::print(toString(len(r)))",
        "301",
    ),
    (
        "fresh",
        "LET r AS List OF Integer = collections::reduce(xs, seed, freshEach)\n  io::print(toString(collections::get(r, 0)))",
        "300",
    ),
    (
        "fresh_right",
        "LET r AS List OF Integer = collections::reduceRight(xs, seed, freshEach)\n  io::print(toString(collections::get(r, 0)))",
        "1",
    ),
    (
        "map",
        "LET r AS Map OF String TO Integer = collections::reduce(xs, Map OF String TO Integer { }, countKeys)\n  io::print(toString(len(r)))",
        "7",
    ),
    (
        "set",
        "LET r AS Set OF Integer = collections::reduce(xs, Set OF Integer { }, keepSet)\n  io::print(toString(len(r)))",
        "5",
    ),
];

#[test]
fn a_collection_accumulator_is_freed_each_step() {
    let mut failures = Vec::new();
    for (name, statement, want) in CASES {
        let source = format!(
            "{HELPERS}\nFUNC main() AS Integer\n  MUT xs AS List OF Integer = []\n  \
             FOR k = 1 TO 300\n    xs = collections::append(xs, k)\n  NEXT\n  \
             LET seed AS List OF Integer = [0]\n  {statement}\n  RETURN 0\nEND FUNC\n"
        );
        let project = common::temp_project(&format!("reduce_acc_{name}"), &source);
        let output = Command::new(common::mfb_exe())
            .arg("build")
            .arg("--debug")
            .arg(&project)
            .output()
            .expect("run mfb build --debug");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(output.status.success(), "{name}: build failed:\n{stdout}");
        let exe = stdout
            .lines()
            .filter_map(|line| line.strip_prefix("Wrote executable to "))
            .find(|path| path.ends_with("-glibc.out"))
            .or_else(|| {
                stdout
                    .lines()
                    .find_map(|line| line.strip_prefix("Wrote executable to "))
            })
            .expect("an executable");
        let run = Command::new(exe).output().expect("run the program");
        let out = String::from_utf8_lossy(&run.stdout);
        let err = String::from_utf8_lossy(&run.stderr);
        let counter = |key: &str| -> u64 {
            err.lines()
                .find_map(|l| l.strip_prefix(&format!("arena.0.{key} ")))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(u64::MAX)
        };
        let (alloc, free, live) = (
            counter("alloc_calls"),
            counter("free_calls"),
            counter("live_bytes"),
        );
        if !run.status.success() || out.lines().next() != Some(*want) || alloc != free || live != 0
        {
            failures.push(format!(
                "{name}: printed {:?} (want {want}), {alloc} allocated / {free} freed / {live} B live",
                out.trim()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
