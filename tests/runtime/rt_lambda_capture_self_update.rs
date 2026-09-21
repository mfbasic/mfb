//! plan-142-G: a self-update through a `MUT` captured by reference in a
//! `collections::forEach` lambda.
//!
//! `forEach(acc, LAMBDA(v AS T) -> acc = collections::append(acc, v))` walks the
//! list its own callback rewrites. `forEach` walks the value `acc` held at the
//! call — its argument 0 is a snapshot, freed with the statement, whenever a
//! sibling closure captures that local by reference — so the walk visits the
//! entry-time elements however the callback rewrites `acc`, in place or not.
//!
//! The self-update runs in place through the reference (`InPlaceDest::Ref`), and
//! any other reassignment through it frees the parent's previous block; before
//! plan-142-G both left that block behind. The balance cases check every block
//! is freed — including a `String` appended to through the reference (which has
//! no in-place form: Correction G1) and a record reassigned while `forEach`
//! walks one of its fields.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const HELPERS: &str = "\
IMPORT collections
IMPORT io

TYPE Box
  items AS List OF Integer
  tag AS String
END TYPE

FUNC isPositive(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC grow(xs AS List OF Integer, n AS Integer) AS List OF Integer
  RETURN collections::append(xs, n * 2)
END FUNC

FUNC showInts(xs AS List OF Integer) AS String
  MUT s AS String = \"\"
  FOR EACH n IN xs
    s = s & toString(n) & \" \"
  NEXT
  RETURN s
END FUNC
";

/// `(name, main body, expected output)`. The body runs inside `FUNC main()`.
const CASES: &[(&str, &str, &str)] = &[
    (
        "self_append",
        "MUT acc AS List OF Integer = [1, 2, 3]
  collections::forEach(acc, LAMBDA(v AS Integer) -> acc = collections::append(acc, v))
  io::print(showInts(acc))",
        "1 2 3 1 2 3 ",
    ),
    (
        "self_prepend",
        "MUT acc AS List OF Integer = [1, 2, 3]
  collections::forEach(acc, LAMBDA(v AS Integer) -> acc = collections::prepend(acc, v))
  io::print(showInts(acc))",
        "3 2 1 1 2 3 ",
    ),
    (
        "self_remove_at",
        "MUT acc AS List OF Integer = [1, 2, 3]
  collections::forEach(acc, LAMBDA(v AS Integer) -> acc = collections::removeAt(acc, 0))
  io::print(toString(len(acc)))",
        "0",
    ),
    (
        "self_strings",
        "MUT acc AS List OF String = [\"a\", \"b\"]
  collections::forEach(acc, LAMBDA(v AS String) -> acc = collections::append(acc, v & v))
  io::print(toString(len(acc)) & \" \" & collections::get(acc, 2) & collections::get(acc, 3))",
        "4 aabb",
    ),
    (
        "other_list",
        "MUT acc AS List OF Integer = [0]
  LET src AS List OF Integer = [1, 2, 3]
  collections::forEach(src, LAMBDA(v AS Integer) -> acc = collections::append(acc, v))
  io::print(showInts(acc))",
        "0 1 2 3 ",
    ),
];

/// `(name, main body, expected output)` whose programs must also free every block.
const BALANCE_CASES: &[(&str, &str, &str)] = &[
    (
        "self_append",
        "MUT acc AS List OF Integer = [1, 2, 3]
  collections::forEach(acc, LAMBDA(v AS Integer) -> acc = collections::append(acc, v))
  io::print(showInts(acc))",
        "1 2 3 1 2 3 ",
    ),
    (
        "other_list",
        "MUT acc AS List OF Integer = [0]
  LET src AS List OF Integer = [1, 2, 3, 4, 5, 6, 7, 8, 9]
  collections::forEach(src, LAMBDA(v AS Integer) -> acc = collections::append(acc, v))
  io::print(showInts(acc))",
        "0 1 2 3 4 5 6 7 8 9 ",
    ),
    (
        "sort",
        "MUT acc AS List OF Integer = [3, 1, 2]
  collections::forEach(acc, LAMBDA(v AS Integer) -> acc = collections::sort(acc))
  io::print(showInts(acc))",
        "1 2 3 ",
    ),
    (
        "filter",
        "MUT acc AS List OF Integer = [1, -2, 3, -4]
  collections::forEach(acc, LAMBDA(v AS Integer) -> acc = collections::filter(acc, isPositive))
  io::print(showInts(acc))",
        "1 3 ",
    ),
    (
        "string_concat",
        "MUT s AS String = \"a\"
  collections::forEach([\"x\", \"y\", \"z\"], LAMBDA(v AS String) -> s = s & v)
  io::print(s)",
        "axyz",
    ),
    (
        "user_function",
        "MUT acc AS List OF Integer = [1, 2]
  collections::forEach(acc, LAMBDA(v AS Integer) -> acc = grow(acc, v))
  io::print(showInts(acc))",
        "1 2 2 4 ",
    ),
    (
        "record_field_walk",
        "MUT b AS Box = Box[[1, 2, 3], \"t\"]
  collections::forEach(b.items, LAMBDA(v AS Integer) -> b = WITH b { tag := b.tag & toString(v) })
  io::print(b.tag & \" \" & toString(len(b.items)))",
        "t123 3",
    ),
];

/// Build and run one case: `(stdout, arena.0 alloc_calls, free_calls, live_bytes)`.
fn run_case(name: &str, body: &str) -> Result<(String, u64, u64, u64), String> {
    let source = format!("{HELPERS}\nFUNC main() AS Integer\n  {body}\n  RETURN 0\nEND FUNC\n");
    let project = common::temp_project(&format!("lambda_capture_self_update_{name}"), &source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        return Err(format!(
            "build failed:\n{stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
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
    let err = String::from_utf8_lossy(&run.stderr);
    if !run.status.success() {
        return Err(format!("program failed:\n{err}"));
    }
    let counter = |key: &str| -> u64 {
        err.lines()
            .find_map(|l| l.strip_prefix(&format!("arena.0.{key} ")))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(u64::MAX)
    };
    Ok((
        String::from_utf8_lossy(&run.stdout)
            .trim_end_matches('\n')
            .to_string(),
        counter("alloc_calls"),
        counter("free_calls"),
        counter("live_bytes"),
    ))
}

#[test]
fn a_for_each_whose_lambda_rewrites_the_walked_list_visits_the_entry_value() {
    let mut failures = Vec::new();
    for (name, body, want) in CASES {
        match run_case(name, body) {
            Ok((printed, ..)) if printed == *want => {}
            Ok((printed, ..)) => {
                failures.push(format!("{name}: printed {printed:?}, want {want:?}"))
            }
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_self_update_through_the_reference_leaves_no_parent_block_behind() {
    let mut failures = Vec::new();
    for (name, body, want) in BALANCE_CASES {
        match run_case(&format!("balance_{name}"), body) {
            Ok((printed, alloc, free, live)) if printed == *want && alloc == free && live == 0 => {}
            Ok((printed, alloc, free, live)) => failures.push(format!(
                "{name}: printed {printed:?} (want {want:?}), {alloc} allocated / {free} freed \
                 / {live} B live"
            )),
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
