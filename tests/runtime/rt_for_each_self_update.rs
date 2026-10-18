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
    let failures = run_cases("for_each_self_update", HELPERS, CASES);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// bug-1036: a graph-typed list (`List OF Node`, a recursive value) is not copied at
/// entry — the loop walks the binding's own block. Every block the body's
/// reassignments replace is freed: the one the loop walks when the loop is done (on
/// the normal exit, `EXIT FOR`, `CONTINUE FOR`, a `RETURN` from the body, and an inner
/// loop over the same binding), every later one at its reassignment. Before, each
/// replaced block leaked.
const GRAPH_HELPERS: &str = "\
IMPORT collections
IMPORT io

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

FUNC mk(t AS Integer) AS Node
  RETURN Node[kids := [Node[kids := [], tag := t]], tag := t]
END FUNC

FUNC tags(xs AS List OF Node) AS String
  MUT s AS String = \"\"
  FOR EACH n IN xs
    s = s & toString(n.tag) & \" \"
  NEXT
  RETURN s
END FUNC

FUNC grownUntil(stop AS Integer) AS List OF Node
  MUT xs AS List OF Node = [mk(1), mk(2), mk(3)]
  FOR EACH v IN xs
    xs = collections::append(xs, mk(v.tag * 10))
    IF v.tag = stop THEN
      RETURN xs
    END IF
  NEXT
  RETURN xs
END FUNC
";

const GRAPH_CASES: &[(&str, &str, &str)] = &[
    (
        "append",
        "MUT xs AS List OF Node = [mk(1), mk(2)]
  MUT seen AS String = \"\"
  FOR EACH v IN xs
    seen = seen & toString(v.tag) & \" \"
    xs = collections::append(xs, mk(v.tag * 10))
    xs = collections::append(xs, mk(v.tag * 100))
  NEXT
  io::print(seen)
  io::print(tags(xs))",
        "1 2 \n1 2 10 100 20 200 ",
    ),
    (
        "replace",
        "MUT xs AS List OF Node = [mk(1), mk(2)]
  MUT seen AS String = \"\"
  FOR EACH v IN xs
    seen = seen & toString(v.tag) & \" \"
    xs = [mk(v.tag + 5)]
  NEXT
  io::print(seen)
  io::print(tags(xs))",
        "1 2 \n7 ",
    ),
    (
        "exit_for",
        "MUT xs AS List OF Node = [mk(1), mk(2), mk(3)]
  FOR EACH v IN xs
    xs = collections::append(xs, mk(v.tag * 10))
    IF v.tag = 2 THEN
      EXIT FOR
    END IF
  NEXT
  io::print(tags(xs))",
        "1 2 3 10 20 ",
    ),
    (
        "continue_for",
        "MUT xs AS List OF Node = [mk(1), mk(2), mk(3)]
  FOR EACH v IN xs
    xs = collections::append(xs, mk(v.tag * 10))
    IF v.tag = 2 THEN
      CONTINUE FOR
    END IF
    xs = collections::append(xs, mk(v.tag))
  NEXT
  io::print(tags(xs))",
        "1 2 3 10 1 20 30 3 ",
    ),
    (
        "return",
        "io::print(tags(grownUntil(2)))
  io::print(tags(grownUntil(9)))",
        "1 2 3 10 20 \n1 2 3 10 20 30 ",
    ),
    (
        "nested",
        "MUT xs AS List OF Node = [mk(1), mk(2)]
  FOR EACH v IN xs
    FOR EACH w IN xs
      xs = collections::append(xs, mk(v.tag * 10 + w.tag))
    NEXT
  NEXT
  io::print(tags(xs))",
        "1 2 11 12 21 22 31 32 ",
    ),
    (
        "nested_after_write",
        "MUT xs AS List OF Node = [mk(1), mk(2)]
  FOR EACH v IN xs
    xs = collections::append(xs, mk(v.tag * 100))
    FOR EACH w IN xs
      xs = collections::append(xs, mk(w.tag))
      EXIT FOR
    NEXT
  NEXT
  io::print(tags(xs))",
        "1 2 100 1 200 1 ",
    ),
];

#[test]
fn a_for_each_over_a_graph_list_frees_every_block_its_body_replaces() {
    let failures = run_cases("for_each_graph_self_update", GRAPH_HELPERS, GRAPH_CASES);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Build each case with `--debug`, run it, and return one line per case that printed
/// the wrong thing, did not free every block it allocated, or freed one twice.
fn run_cases(prefix: &str, helpers: &str, cases: &[(&str, &str, &str)]) -> Vec<String> {
    let mut failures = Vec::new();
    for (name, body, want) in cases {
        let source = format!("{helpers}\nFUNC main() AS Integer\n  {body}\n  RETURN 0\nEND FUNC\n");
        let project = common::temp_project(&format!("{prefix}_{name}"), &source);
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
        let (alloc, free, live, skips) = (
            counter("alloc_calls"),
            counter("free_calls"),
            counter("live_bytes"),
            counter("double_free_skips"),
        );
        let printed = out.trim_end_matches('\n');
        if !run.status.success() || printed != *want || alloc != free || live != 0 || skips != 0 {
            failures.push(format!(
                "{name}: printed {printed:?} (want {want:?}), {alloc} allocated / {free} freed / \
                 {live} B live / {skips} double frees"
            ));
        }
    }
    failures
}
