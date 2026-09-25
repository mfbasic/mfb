//! plan-142-F: a `FOR EACH` whose body self-updates the binding it walks.
//!
//! The loop walks the value the binding held at entry (`05_collections.md`): its
//! own writes are never visited. It walks a private copy made once at entry, so
//! every `x = op(x, …)` in the body runs in place; before, each one copied the
//! whole binding and leaked the block it replaced. Each program prints the
//! elements the loop visited and the binding it ended with, and must end with
//! every block it allocated freed — on the normal exit, `EXIT FOR`, and a
//! `RETURN` from inside the body.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const HELPERS: &str = "\
IMPORT collections
IMPORT io

FUNC showInts(xs AS List OF Integer) AS String
  MUT s AS String = \"\"
  FOR EACH n IN xs
    s = s & toString(n) & \" \"
  NEXT
  RETURN s
END FUNC

FUNC isSmall(n AS Integer) AS Boolean
  RETURN n < 3
END FUNC

FUNC firstGrown(start AS List OF Integer) AS List OF Integer
  MUT xs AS List OF Integer = start
  FOR EACH v IN xs
    xs = collections::append(xs, v * 10)
    IF v = 2 THEN
      RETURN xs
    END IF
  NEXT
  RETURN xs
END FUNC
";

/// `(name, main body, expected output)`. The body runs inside `FUNC main()`.
const CASES: &[(&str, &str, &str)] = &[
    (
        "append",
        "MUT xs AS List OF Integer = [1, 2, 3]
  MUT seen AS String = \"\"
  FOR EACH v IN xs
    seen = seen & toString(v) & \" \"
    xs = collections::append(xs, v * 10)
  NEXT
  io::print(seen)
  io::print(showInts(xs))",
        "1 2 3 \n1 2 3 10 20 30 ",
    ),
    (
        "remove_at",
        "MUT xs AS List OF Integer = [1, 2, 3]
  MUT seen AS String = \"\"
  FOR EACH v IN xs
    seen = seen & toString(v) & \" \"
    xs = collections::removeAt(xs, 0)
  NEXT
  io::print(seen)
  io::print(toString(len(xs)))",
        "1 2 3 \n0",
    ),
    (
        "sort",
        "MUT xs AS List OF Integer = [3, 1, 2]
  MUT seen AS String = \"\"
  FOR EACH v IN xs
    seen = seen & toString(v) & \" \"
    xs = collections::sort(xs)
    xs = collections::prepend(xs, v)
  NEXT
  io::print(seen)
  io::print(showInts(xs))",
        "3 1 2 \n2 1 1 2 3 3 ",
    ),
    (
        "filter",
        "MUT xs AS List OF Integer = [1, 2, 3, 4]
  MUT seen AS String = \"\"
  FOR EACH v IN xs
    seen = seen & toString(v) & \" \"
    xs = collections::filter(xs, isSmall)
  NEXT
  io::print(seen)
  io::print(showInts(xs))",
        "1 2 3 4 \n1 2 ",
    ),
    (
        "strings",
        "MUT xs AS List OF String = [\"a\", \"b\"]
  MUT seen AS String = \"\"
  FOR EACH v IN xs
    seen = seen & v & \" \"
    xs = collections::append(xs, v & v)
  NEXT
  io::print(seen)
  io::print(toString(len(xs)) & \" \" & collections::get(xs, 2) & collections::get(xs, 3))",
        "a b \n4 aabb",
    ),
    (
        "map",
        "MUT m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1, \"b\" := 2 }
  MUT seen AS Integer = 0
  FOR EACH e IN m
    seen = seen + 1
    m = collections::set(m, e.key & \"x\", e.value)
  NEXT
  io::print(toString(seen))
  io::print(toString(len(m)))",
        "2\n4",
    ),
    (
        "set",
        "MUT s AS Set OF Integer = Set OF Integer { 1, 2 }
  MUT seen AS Integer = 0
  FOR EACH v IN s
    seen = seen + v
    s = collections::add(s, v + 10)
  NEXT
  io::print(toString(seen))
  io::print(toString(len(s)))",
        "3\n4",
    ),
    (
        "exit_for",
        "MUT xs AS List OF Integer = [1, 2, 3]
  FOR EACH v IN xs
    xs = collections::append(xs, v * 10)
    IF v = 2 THEN
      EXIT FOR
    END IF
  NEXT
  io::print(showInts(xs))",
        "1 2 3 10 20 ",
    ),
    (
        "return",
        "io::print(showInts(firstGrown([1, 2, 3])))",
        "1 2 3 10 20 ",
    ),
    (
        "nested",
        "MUT xs AS List OF Integer = [1, 2]
  FOR EACH v IN xs
    FOR EACH w IN xs
      xs = collections::append(xs, v * 10 + w)
    NEXT
  NEXT
  io::print(showInts(xs))",
        "1 2 11 12 21 22 31 32 ",
    ),
];

#[test]
fn a_for_each_that_self_updates_its_binding_visits_the_entry_value_and_frees_everything() {
    let mut failures = Vec::new();
    for (name, body, want) in CASES {
        let source = format!("{HELPERS}\nFUNC main() AS Integer\n  {body}\n  RETURN 0\nEND FUNC\n");
        let project = common::temp_project(&format!("for_each_self_update_{name}"), &source);
        let output = Command::new(common::mfb_exe())
            .arg("build")
            .arg("--debug")
            .arg(&project)
            .output()
            .expect("run mfb build --debug");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.status.success() {
            failures.push(format!(
                "{name}: build failed:\n{stdout}{}",
                String::from_utf8_lossy(&output.stderr)
            ));
            continue;
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
        let printed = out.trim_end_matches('\n');
        if !run.status.success() || printed != *want || alloc != free || live != 0 {
            failures.push(format!(
                "{name}: printed {printed:?} (want {want:?}), {alloc} allocated / {free} freed / \
                 {live} B live"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
