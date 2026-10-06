//! bug-805: a resource nested inside an ordinary returned aggregate.
//!
//! `FUNC make() AS List OF List OF Holder` (with `TYPE Holder { handle AS RES
//! fs::File }`) used to fail native lowering — the owned-list drop descriptor took
//! one aggregate layer and found `List OF Holder`, which has no close op. Making it
//! compile was not enough: the `RETURN` released only a resource, a resource union
//! or a flat `List OF RES`, and the caller adopted only a flat `List OF RES`, so the
//! callee's own scope closed the handle it was handing over.
//!
//! The contract (§15.6): a returned aggregate transfers every resource it reaches,
//! at any depth, to the caller's binding scope, which closes each exactly once.
//!
//! Each case writes through a FRESH buffered handle, whose bytes reach the file
//! only when the handle is closed. Reading the file inside the caller's scope
//! (empty: still open) and after it (`flushed`: closed there) pins the close to
//! the caller's scope; a write through the handle after the callee returned proves
//! it was not closed early (a closed handle raises `ErrResourceClosed`, recovered
//! as `CLOSED`). A `--debug` build's arena counters pin a single close/free.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const PRELUDE: &str = r#"IMPORT io
IMPORT fs
IMPORT os
IMPORT tcp
IMPORT collections
IMPORT errorCode

TYPE Holder
  handle AS RES fs::File
END TYPE

FUNC dir() AS String
  RETURN os::getEnv("PROBE_DIR")
END FUNC

' A fresh write handle whose only bytes sit in its buffer until it is closed.
FUNC fresh(name AS String) AS RES fs::File
  RES f AS fs::File = fs::openFile(dir() & "/" & name, "w")
  fs::setBuffered(f, TRUE)
  fs::writeAll(f, "flushed")
  RETURN f
END FUNC

FUNC flushed(name AS String) AS String
  RETURN fs::readText(dir() & "/" & name)
END FUNC

' Write one more byte through `h`: `open` when the handle was still live.
FUNC poke(h AS fs::File) AS String
  fs::writeAll(h, "+")
  RETURN "open"
  TRAP(e)
    IF e.code = errorCode::ErrResourceClosed THEN
      RETURN "CLOSED"
    END IF
    RETURN "error " & e.message
  END TRAP
END FUNC
"#;

struct Run {
    lines: Vec<String>,
    stderr: String,
    status: i32,
}

fn build_and_run(name: &str, body: &str, debug: bool) -> Run {
    let source = format!("{PRELUDE}\n{body}");
    let project = common::temp_project(name, &source);
    let probe = project.join("probe");
    std::fs::create_dir_all(&probe).expect("probe dir");
    std::fs::write(probe.join("data.txt"), "first\nsecond\n").expect("data");
    let mut command = Command::new(common::mfb_exe());
    command.arg("build");
    if debug {
        command.arg("--debug");
    }
    let output = command.arg(&project).output().expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        output.status.success(),
        "{name}: build failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let flavored = format!("-{}.out", common::host_libc_flavor());
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let executable = written
        .iter()
        .find(|path| path.ends_with(&flavored))
        .or_else(|| written.first())
        .expect("executable path");
    let (status, out, stderr) = common::run_capture_with_env(
        std::path::Path::new(executable),
        &[("PROBE_DIR", probe.to_string_lossy().to_string())],
    );
    Run {
        lines: out.lines().map(str::to_string).collect(),
        stderr,
        status,
    }
}

fn run(name: &str, body: &str) -> Vec<String> {
    let run = build_and_run(name, body, false);
    assert_eq!(
        run.status, 0,
        "{name}: exit status; stderr:\n{}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("Cleanup failure"),
        "{name}: a cleanup failed (a second close is reported there):\n{}",
        run.stderr
    );
    run.lines
}

/// The main arena's `double_free_skips` from a `--debug` run: a free of a block
/// already free is skipped and counted, so 0 means every record and node was
/// freed at most once.
fn double_free_skips(name: &str, run: &Run) -> u64 {
    let line = run
        .stderr
        .lines()
        .find(|line| line.starts_with("arena.0.double_free_skips "))
        .unwrap_or_else(|| panic!("{name}: no double_free_skips line:\n{}", run.stderr));
    line.rsplit(' ').next().unwrap().parse().expect("counter")
}

/// The bug document's reproduction, verbatim in shape: the caller reads the
/// first line through the nested record.
#[test]
fn a_nested_list_of_records_returns_a_live_handle() {
    let lines = run(
        "b805_primary",
        r#"FUNC open() AS RES fs::File
  RETURN fs::openFile(dir() & "/data.txt")
END FUNC

FUNC read(f AS fs::File) AS String
  LET line = fs::readLine(f) TRAP(e)
    RECOVER e.message
  END TRAP
  RETURN line
END FUNC

FUNC make() AS List OF List OF Holder
  MUT out AS List OF List OF Holder = []
  RES f = open()
  out = collections::append(out, [Holder[f]])
  RETURN out
END FUNC

FUNC main() AS Nothing
  LET out = make()
  LET row = collections::get(out, 0)
  LET h = collections::get(row, 0)
  io::print(read(h.handle))
END FUNC
"#,
    );
    assert_eq!(lines, ["first"]);
}

/// The same shape with a buffered handle: open through the returned value, closed
/// once at the caller's inner scope exit, and freed once.
#[test]
fn a_nested_list_of_records_closes_once_at_the_callers_scope() {
    let body = r#"FUNC make() AS List OF List OF Holder
  MUT out AS List OF List OF Holder = []
  RES f = fresh("nested.txt")
  out = collections::append(out, [Holder[f]])
  RETURN out
END FUNC

FUNC main() AS Nothing
  IF TRUE THEN
    LET out = make()
    LET h = collections::get(collections::get(out, 0), 0)
    io::print("poke: " & poke(h.handle))
    io::print("inside: [" & flushed("nested.txt") & "]")
  END IF
  io::print("after: " & flushed("nested.txt"))
END FUNC
"#;
    let run = build_and_run("b805_nested_debug", body, true);
    assert_eq!(run.status, 0, "stderr:\n{}", run.stderr);
    assert_eq!(
        run.lines,
        ["poke: open", "inside: []", "after: flushed+"],
        "stderr:\n{}",
        run.stderr
    );
    assert!(!run.stderr.contains("Cleanup failure"), "{}", run.stderr);
    assert_eq!(double_free_skips("b805_nested_debug", &run), 0);
}

#[test]
fn a_list_of_records_transfers_its_handles() {
    let lines = run(
        "b805_list_holder",
        r#"FUNC make() AS List OF Holder
  MUT out AS List OF Holder = []
  RES f = fresh("list.txt")
  out = collections::append(out, Holder[f])
  RETURN out
END FUNC

FUNC main() AS Nothing
  IF TRUE THEN
    LET out = make()
    io::print("poke: " & poke(collections::get(out, 0).handle))
    io::print("inside: [" & flushed("list.txt") & "]")
  END IF
  io::print("after: " & flushed("list.txt"))
END FUNC
"#,
    );
    assert_eq!(lines, ["poke: open", "inside: []", "after: flushed+"]);
}

#[test]
fn a_map_of_records_transfers_its_handles() {
    let lines = run(
        "b805_map_holder",
        r#"FUNC make() AS Map OF String TO Holder
  MUT out AS Map OF String TO Holder = Map OF String TO Holder {}
  RES f = fresh("map.txt")
  out = collections::set(out, "k", Holder[f])
  RETURN out
END FUNC

FUNC main() AS Nothing
  IF TRUE THEN
    LET out = make()
    io::print("poke: " & poke(collections::get(out, "k").handle))
    io::print("inside: [" & flushed("map.txt") & "]")
  END IF
  io::print("after: " & flushed("map.txt"))
END FUNC
"#,
    );
    assert_eq!(lines, ["poke: open", "inside: []", "after: flushed+"]);
}

/// A record holding a list of records: the resource is two aggregate layers
/// below the returned value, behind an inlined collection field.
#[test]
fn a_record_holding_a_list_of_records_transfers_its_handles() {
    let lines = run(
        "b805_bag",
        r#"TYPE Bag
  label AS String
  items AS List OF Holder
END TYPE

FUNC make() AS Bag
  MUT b AS Bag = Bag["bag", []]
  RES f = fresh("bag.txt")
  b = WITH b { items := collections::append(b.items, Holder[f]) }
  RETURN b
END FUNC

FUNC main() AS Nothing
  IF TRUE THEN
    LET b = make()
    io::print(b.label & " poke: " & poke(collections::get(b.items, 0).handle))
    io::print("inside: [" & flushed("bag.txt") & "]")
  END IF
  io::print("after: " & flushed("bag.txt"))
END FUNC
"#,
    );
    assert_eq!(lines, ["bag poke: open", "inside: []", "after: flushed+"]);
}

/// Two kinds of resource in one record — a concrete `File` field and a resource
/// union field — used to be refused outright ("an owned-list carries one close
/// op"). Each kind now has its own list and both close once at the caller.
#[test]
fn a_record_with_two_resource_kinds_transfers_both() {
    let lines = run(
        "b805_pair",
        r#"UNION Stream
  fs::File
  tcp::Socket
END UNION

TYPE Duo
  a AS RES fs::File
  s AS RES Stream
END TYPE

FUNC make() AS List OF Duo
  MUT out AS List OF Duo = []
  RES f = fresh("pair_a.txt")
  RES g AS Stream = fresh("pair_s.txt")
  out = collections::append(out, Duo[f, g])
  RETURN out
END FUNC

FUNC main() AS Nothing
  IF TRUE THEN
    LET out = make()
    io::print("poke: " & poke(collections::get(out, 0).a))
    io::print("inside: [" & flushed("pair_a.txt") & "][" & flushed("pair_s.txt") & "]")
  END IF
  io::print("after: " & flushed("pair_a.txt") & " " & flushed("pair_s.txt"))
END FUNC
"#,
    );
    assert_eq!(
        lines,
        ["poke: open", "inside: [][]", "after: flushed+ flushed"]
    );
}

/// A data-union variant carrying a resource field.
#[test]
fn a_data_union_variant_transfers_its_handle() {
    let lines = run(
        "b805_union",
        r#"TYPE Opened
  h AS RES fs::File
END TYPE

TYPE Missing
  why AS String
END TYPE

UNION Slot
  Opened
  Missing
END UNION

FUNC make() AS List OF Slot
  MUT out AS List OF Slot = [Missing["none"]]
  RES f = fresh("union.txt")
  LET o AS Slot = Opened[f]
  out = collections::append(out, o)
  RETURN out
END FUNC

FUNC main() AS Nothing
  IF TRUE THEN
    LET out = make()
    MATCH collections::get(out, 1)
      CASE Opened(o)
        io::print("poke: " & poke(o.h))
      CASE Missing(m)
        io::print("missing " & m.why)
    END MATCH
    io::print("inside: [" & flushed("union.txt") & "]")
  END IF
  io::print("after: " & flushed("union.txt"))
END FUNC
"#,
    );
    assert_eq!(lines, ["poke: open", "inside: []", "after: flushed+"]);
}

/// A recursive record: the walk defers the type that reaches itself to a run-time
/// worklist instead of expanding it forever (or recursing on the native stack).
#[test]
fn a_recursive_record_transfers_every_handle_in_the_tree() {
    let body = r#"TYPE Node
  h AS RES fs::File
  kids AS List OF Node
END TYPE

FUNC make() AS List OF Node
  MUT out AS List OF Node = []
  RES f = fresh("root.txt")
  RES g = fresh("leaf.txt")
  out = collections::append(out, Node[f, [Node[g, []]]])
  RETURN out
END FUNC

FUNC main() AS Nothing
  IF TRUE THEN
    LET out = make()
    LET root = collections::get(out, 0)
    io::print("poke: " & poke(root.h) & " " & poke(collections::get(root.kids, 0).h))
    io::print("inside: [" & flushed("root.txt") & "][" & flushed("leaf.txt") & "]")
  END IF
  io::print("after: " & flushed("root.txt") & " " & flushed("leaf.txt"))
END FUNC
"#;
    let run = build_and_run("b805_recursive", body, true);
    assert_eq!(run.status, 0, "stderr:\n{}", run.stderr);
    assert_eq!(
        run.lines,
        [
            "poke: open open",
            "inside: [][]",
            "after: flushed+ flushed+"
        ],
        "stderr:\n{}",
        run.stderr
    );
    assert!(!run.stderr.contains("Cleanup failure"), "{}", run.stderr);
    assert_eq!(double_free_skips("b805_recursive", &run), 0);
}

/// A discarded nested result is closed at the call statement: nobody else will.
#[test]
fn a_discarded_nested_result_closes_what_it_carried() {
    let lines = run(
        "b805_discarded",
        r#"FUNC make() AS List OF List OF Holder
  MUT out AS List OF List OF Holder = []
  RES f = fresh("discarded.txt")
  out = collections::append(out, [Holder[f]])
  RETURN out
END FUNC

FUNC main() AS Nothing
  make()
  io::print("after call: " & flushed("discarded.txt"))
END FUNC
"#,
    );
    assert_eq!(lines, ["after call: flushed"]);
}

/// An error before the `RETURN`: the resources still ride the callee's scope,
/// which closes them on its error exit.
#[test]
fn an_error_before_the_return_closes_in_the_callee() {
    let lines = run(
        "b805_error",
        r#"FUNC make(shouldFail AS Boolean) AS List OF List OF Holder
  MUT out AS List OF List OF Holder = []
  RES f = fresh("error.txt")
  out = collections::append(out, [Holder[f]])
  IF shouldFail THEN
    FAIL error(77050002, "boom")
  END IF
  RETURN out
END FUNC

FUNC main() AS Nothing
  LET out = make(TRUE) TRAP(e)
    io::print("trapped: " & flushed("error.txt"))
    RECOVER []
  END TRAP
  io::print("rows: " & toString(len(out)))
END FUNC
"#,
    );
    assert_eq!(lines, ["trapped: flushed", "rows: 0"]);
}

/// A handle the caller passed in comes back inside a nested aggregate: it is
/// owned further out, so neither the callee nor the caller's inner binding closes
/// it (bug-755's rule, one aggregate layer deeper).
#[test]
fn a_passed_in_handle_returned_inside_a_nested_aggregate_keeps_its_owner() {
    let lines = run(
        "b805_passed",
        r#"FUNC wrapAll(h AS fs::File) AS List OF List OF Holder
  RETURN [[Holder[h]]]
END FUNC

FUNC main() AS Nothing
  RES a AS fs::File = fresh("passed.txt")
  IF TRUE THEN
    LET rows = wrapAll(a)
  END IF
  io::print("poke: " & poke(a))
  io::print("before exit: [" & flushed("passed.txt") & "]")
END FUNC
"#,
    );
    assert_eq!(lines, ["poke: open", "before exit: []"]);
}

/// A `collections` helper over the caller's own nested value re-packages its
/// pointers; binding the result in an inner scope must not adopt (and close)
/// them (bug-643).
#[test]
fn a_repackaged_nested_value_does_not_adopt_the_callers_handles() {
    let lines = run(
        "b805_borrowed",
        r#"FUNC make() AS List OF List OF Holder
  MUT out AS List OF List OF Holder = []
  RES f = fresh("borrowed.txt")
  out = collections::append(out, [Holder[f]])
  RETURN out
END FUNC

FUNC main() AS Nothing
  LET out = make()
  IF TRUE THEN
    LET some = collections::take(out, 1)
    LET row = collections::get(out, 0)
  END IF
  io::print("poke: " & poke(collections::get(collections::get(out, 0), 0).handle))
  io::print("before exit: [" & flushed("borrowed.txt") & "]")
END FUNC
"#,
    );
    assert_eq!(lines, ["poke: open", "before exit: []"]);
}

/// The flat contrast the bug document names: a direct `List OF RES fs::File`
/// return keeps working exactly as before.
#[test]
fn the_flat_resource_list_return_is_unchanged() {
    let lines = run(
        "b805_flat",
        r#"FUNC make() AS List OF RES fs::File
  MUT out AS List OF RES fs::File = []
  RES f = fresh("flat.txt")
  out = collections::append(out, f)
  RETURN out
END FUNC

FUNC main() AS Nothing
  IF TRUE THEN
    LET out = make()
    io::print("poke: " & poke(collections::get(out, 0)))
    io::print("inside: [" & flushed("flat.txt") & "]")
  END IF
  io::print("after: " & flushed("flat.txt"))
END FUNC
"#,
    );
    assert_eq!(lines, ["poke: open", "inside: []", "after: flushed+"]);
}

/// Found by this bug's audit: merely DECLARING a resource union that lists
/// `fs::File` made `RETURN f` of a plain `File` hand the caller a 16-byte copy of
/// the record's head (the union's inline size answered for its variant), so the
/// caller's first write read a truncated, poisoned record.
#[test]
fn a_declared_resource_union_does_not_change_a_plain_file_return() {
    let lines = run(
        "b805_union_declared",
        r#"UNION Stream
  fs::File
  tcp::Socket
END UNION

FUNC main() AS Nothing
  IF TRUE THEN
    RES f = fresh("declared.txt")
    io::print("poke: " & poke(f))
    io::print("inside: [" & flushed("declared.txt") & "]")
  END IF
  io::print("after: " & flushed("declared.txt"))
END FUNC
"#,
    );
    assert_eq!(lines, ["poke: open", "inside: []", "after: flushed+"]);
}

/// The main arena's live bytes at exit from a `--debug` run.
fn live_bytes(name: &str, run: &Run) -> u64 {
    let line = run
        .stderr
        .lines()
        .find(|line| line.starts_with("arena.0.live_bytes "))
        .unwrap_or_else(|| panic!("{name}: no live_bytes line:\n{}", run.stderr));
    line.rsplit(' ').next().unwrap().parse().expect("counter")
}

/// Live bytes at exit after `n` calls of `make()` bound in a loop body.
fn live_after_calls(name: &str, returns: &str, build: &str, n: u32) -> u64 {
    let body = format!(
        "FUNC make() AS {returns}\n{build}END FUNC\n\nFUNC main() AS Nothing\n  FOR i = 1 TO {n}\n    LET out = make()\n  NEXT\n  io::print(\"ok\")\nEND FUNC\n"
    );
    let run = build_and_run(name, &body, true);
    assert_eq!(run.status, 0, "{name}: stderr:\n{}", run.stderr);
    assert_eq!(run.lines, ["ok"]);
    assert_eq!(double_free_skips(name, &run), 0);
    live_bytes(name, &run)
}

/// A returned nested aggregate costs nothing per call: the caller's adopting
/// binding closes and reclaims each record, and the callee frees the owned-list
/// node its float allocated.
#[test]
fn a_returned_nested_aggregate_leaks_nothing_per_call() {
    let build = "  MUT out AS List OF List OF Holder = []\n  RES f = fresh(\"loop.txt\")\n  out = collections::append(out, [Holder[f]])\n  RETURN out\n";
    let few = live_after_calls("b805_leak_nested_50", "List OF List OF Holder", build, 50);
    let many = live_after_calls("b805_leak_nested_100", "List OF List OF Holder", build, 100);
    assert_eq!(few, many, "bytes leaked per call: {}", (many - few) / 50);
}

/// The flat return's owned-list node used to leak (16 B per call): the `RETURN`
/// dropped the drain that frees it.
#[test]
fn a_returned_flat_resource_list_frees_its_float_nodes() {
    let build = "  MUT out AS List OF RES fs::File = []\n  RES f = fresh(\"loop.txt\")\n  out = collections::append(out, f)\n  RETURN out\n";
    let few = live_after_calls("b805_leak_flat_50", "List OF RES fs::File", build, 50);
    let many = live_after_calls("b805_leak_flat_100", "List OF RES fs::File", build, 100);
    assert_eq!(few, many, "bytes leaked per call: {}", (many - few) / 50);
}

/// A resource removed from a container the function then returns is still that
/// function's (removal does not undo the float): it closes at the function's exit,
/// where before it was owned by a dead frame and never closed.
#[test]
fn a_resource_removed_before_the_return_closes_in_the_callee() {
    let lines = run(
        "b805_removed",
        r#"FUNC make() AS List OF RES fs::File
  MUT out AS List OF RES fs::File = []
  RES f = fresh("removed.txt")
  out = collections::append(out, f)
  out = collections::removeAt(out, 0)
  RETURN out
END FUNC

FUNC main() AS Nothing
  LET out = make()
  io::print("after call: " & flushed("removed.txt") & " " & toString(len(out)))
END FUNC
"#,
    );
    assert_eq!(lines, ["after call: flushed 0"]);
}

/// Found by this bug's audit: a container ASSIGNED a call result (not bound from
/// one) adopted nothing, so what the callee handed out was owned by nobody and
/// never closed — the buffered bytes never reached the file. The assignment now
/// adopts into the binding's own scope: open after an inner block assigned it,
/// closed when the binding's scope exits.
#[test]
fn a_container_assigned_a_call_result_adopts_its_resources() {
    let lines = run(
        "b805_assigned",
        r#"FUNC make(name AS String) AS List OF List OF Holder
  MUT out AS List OF List OF Holder = []
  RES f = fresh(name)
  out = collections::append(out, [Holder[f]])
  RETURN out
END FUNC

FUNC fill() AS Nothing
  MUT rows AS List OF List OF Holder = []
  IF TRUE THEN
    rows = make("assigned.txt")
  END IF
  io::print("poke: " & poke(collections::get(collections::get(rows, 0), 0).handle))
  io::print("inside: [" & flushed("assigned.txt") & "]")
END FUNC

FUNC main() AS Nothing
  fill()
  io::print("after: " & flushed("assigned.txt"))
END FUNC
"#,
    );
    assert_eq!(lines, ["poke: open", "inside: []", "after: flushed+"]);
}
