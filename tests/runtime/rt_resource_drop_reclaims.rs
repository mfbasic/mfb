//! A resource drop reclaims every block the resource owns.
//!
//! Each case runs one `--debug` build at `N` and `2N` iterations and requires the
//! main arena to end with as many frees as allocations, no live bytes, and no
//! double free the arena had to skip. A leak of one block per iteration shows as a
//! `live_bytes` that doubles; a drop that frees a block twice shows as
//! `double_free_skips`. Every case also prints what the program read or wrote
//! through the handle, so a "fix" that stopped the I/O would fail on the value.
#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use common::debug_report::{arena_lines, build_debug, counter};

struct Counts {
    allocs: u64,
    frees: u64,
    live: u64,
    double_free_skips: u64,
    stdout: String,
}

fn run_at(name: &str, exe: &std::path::Path, dir: &std::path::Path, n: u32) -> Counts {
    let (status, stdout, stderr) = common::run_capture_with_env(
        exe,
        &[
            ("N", n.to_string()),
            ("PROBE_DIR", dir.to_string_lossy().into_owned()),
        ],
    );
    assert_eq!(status, 0, "{name} N={n} failed:\n{stdout}\n{stderr}");
    let lines = arena_lines(name, &stderr);
    Counts {
        allocs: counter(name, &lines, 0, "alloc_calls"),
        frees: counter(name, &lines, 0, "free_calls"),
        live: counter(name, &lines, 0, "live_bytes"),
        double_free_skips: counter(name, &lines, 0, "double_free_skips"),
        stdout,
    }
}

/// Build `source`, run it at 10 and 20 iterations, and require a balanced arena at
/// both. Returns the 10-iteration stdout for the caller's value check.
fn assert_balanced(name: &str, source: &str) -> String {
    let exe = build_debug(name, source);
    let dir = exe.parent().expect("build dir").join("probe");
    std::fs::create_dir_all(&dir).expect("probe dir");
    std::fs::write(dir.join("data.txt"), "first\nsecond\n").expect("data");
    let mut first = None;
    for n in [10, 20] {
        let run = run_at(name, &exe, &dir, n);
        assert_eq!(
            run.double_free_skips, 0,
            "{name} N={n}: the arena skipped a double free"
        );
        assert_eq!(
            (run.allocs, run.live),
            (run.frees, 0),
            "{name} N={n}: alloc_calls {} free_calls {} live_bytes {}",
            run.allocs,
            run.frees,
            run.live
        );
        first.get_or_insert(run.stdout);
    }
    first.expect("ran")
}

/// [`assert_balanced`] without the balance: a shape the fix deliberately leaves
/// to the close-only drain must still never free a block twice.
fn assert_no_double_free(name: &str, source: &str) -> String {
    let exe = build_debug(name, source);
    let dir = exe.parent().expect("build dir").join("probe");
    std::fs::create_dir_all(&dir).expect("probe dir");
    std::fs::write(dir.join("data.txt"), "first\nsecond\n").expect("data");
    let run = run_at(name, &exe, &dir, 10);
    assert_eq!(
        run.double_free_skips, 0,
        "{name}: the arena skipped a double free"
    );
    assert!(
        run.frees <= run.allocs,
        "{name}: free_calls {} exceeds alloc_calls {}",
        run.frees,
        run.allocs
    );
    run.stdout
}

const PRELUDE: &str = r#"IMPORT io
IMPORT os
IMPORT fs
IMPORT tcp
IMPORT collections
IMPORT errorCode

UNION Stream
  fs::File
  tcp::Socket
END UNION

FUNC dir() AS String
  RETURN os::getEnv("PROBE_DIR")
END FUNC

FUNC count() AS Integer
  RETURN toInt(os::getEnv("N"))
END FUNC

' A write handle with a live 4 KiB output buffer.
FUNC openBuffered() AS RES fs::File
  RES f AS fs::File = fs::openFile(dir() & "/out.txt", "w")
  fs::setBuffered(f, TRUE)
  RETURN f
END FUNC

' A read handle; its first `readLine` allocates the 16 KiB read buffer.
FUNC openData() AS RES fs::File
  RETURN fs::openFile(dir() & "/data.txt")
END FUNC
"#;

fn program(body: &str) -> String {
    format!("{PRELUDE}\n{body}")
}

// ---- bug-1001: a resource union bound straight from a producer ----

#[test]
fn a_union_bound_from_a_producer_frees_the_write_buffer() {
    let out = assert_balanced(
        "b1001_write",
        &program(
            r#"FUNC body() AS Nothing
  RES u AS Stream = openBuffered()
  MATCH u
    CASE fs::File(f)
      fs::writeAll(f, "U")
    CASE tcp::Socket(k)
      io::print("socket")
  END MATCH
END FUNC

FUNC main() AS Integer
  FOR i = 1 TO count()
    body()
  NEXT
  io::print(fs::readText(dir() & "/out.txt"))
  RETURN 0
END FUNC
"#,
        ),
    );
    assert_eq!(out.trim(), "U", "the buffered byte is flushed by the drop");
}

#[test]
fn a_union_bound_from_a_producer_frees_the_read_buffer() {
    let out = assert_balanced(
        "b1001_read",
        &program(
            r#"FUNC once() AS Integer
  RES s AS Stream = openData()
  MATCH s
    CASE fs::File(h)
      RETURN len(fs::readLine(h))
    CASE tcp::Socket(k)
      RETURN 0
  END MATCH
  RETURN 0
END FUNC

FUNC main() AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO count()
    t = t + once()
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC
"#,
        ),
    );
    assert_eq!(out.trim(), "50", "ten reads of the 5-byte first line");
}

/// The control the bug report names: the concrete binding owns the record and its
/// buffers, the union only wraps it. Flat before and after the fix.
#[test]
fn a_union_wrapping_a_live_binding_stays_balanced() {
    let out = assert_balanced(
        "b1001_alias",
        &program(
            r#"FUNC body() AS Nothing
  RES f AS fs::File = openBuffered()
  RES u AS Stream = f
  MATCH u
    CASE fs::File(g)
      fs::writeAll(g, "A")
    CASE tcp::Socket(k)
      io::print("socket")
  END MATCH
  ' Still open: the union's drop must not have run early.
  fs::writeAll(f, "B")
END FUNC

FUNC main() AS Integer
  FOR i = 1 TO count()
    body()
  NEXT
  io::print(fs::readText(dir() & "/out.txt"))
  RETURN 0
END FUNC
"#,
        ),
    );
    assert_eq!(out.trim(), "AB");
}

/// A stateful union: the buffers, the STATE block, the record and the box all go.
#[test]
fn a_stateful_union_bound_from_a_producer_frees_its_buffers() {
    let out = assert_balanced(
        "b1001_state",
        &program(
            r#"TYPE Cursor
  line AS Integer
END TYPE

FUNC once() AS Integer
  RES s AS Stream STATE Cursor = openData()
  s.state = Cursor[line := 7]
  MUT n AS Integer = 0
  MATCH s
    CASE fs::File(h)
      n = len(fs::readLine(h))
    CASE tcp::Socket(k)
      n = 0
  END MATCH
  RETURN n + s.state.line
END FUNC

FUNC main() AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO count()
    t = t + once()
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC
"#,
        ),
    );
    assert_eq!(out.trim(), "120");
}

// ---- bug-1000: an owned `List OF RES` that is read, iterated or adopted ----

const LIST_PRELUDE: &str = r#"FUNC mkList() AS List OF RES fs::File
  RES f AS fs::File = openBuffered()
  RETURN [f]
END FUNC

FUNC mkData() AS List OF RES fs::File
  RES f AS fs::File = openData()
  RETURN [f]
END FUNC

' Hands back an element of the caller's own list.
FUNC first(xs AS List OF RES fs::File) AS RES fs::File
  RETURN collections::get(xs, 0)
END FUNC

FUNC wrap(f AS fs::File) AS List OF RES fs::File
  RETURN [f]
END FUNC

' `open` when the handle still takes a write; `closed` only for the defined
' closed-handle error, so a read of a freed record cannot pass as either.
FUNC poke(h AS fs::File, text AS String) AS String
  fs::writeAll(h, text)
  RETURN "open"
  TRAP(e)
    IF e.code = errorCode::ErrResourceClosed THEN
      RETURN "closed"
    END IF
    RETURN "error" & toString(e.code)
  END TRAP
END FUNC
"#;

fn list_program(body: &str) -> String {
    format!("{PRELUDE}\n{LIST_PRELUDE}\n{body}")
}

const LOOP_MAIN: &str = r#"
FUNC main() AS Integer
  FOR i = 1 TO count()
    body()
  NEXT
  io::print(fs::readText(dir() & "/out.txt"))
  RETURN 0
END FUNC
"#;

/// The report's leakA: a list adopted from a call, then read through `get`.
#[test]
fn an_adopted_list_read_through_get_frees_its_element() {
    let out = assert_balanced(
        "b1000_adopted_get",
        &list_program(&format!(
            r#"FUNC body() AS Nothing
  LET xs = mkList()
  fs::writeAll(collections::get(xs, 0), "L")
END FUNC
{LOOP_MAIN}"#
        )),
    );
    assert_eq!(out.trim(), "L");
}

/// The report's leakA3: the element floats in from an inner block.
#[test]
fn a_floated_element_read_through_get_is_freed() {
    let out = assert_balanced(
        "b1000_floated_get",
        &list_program(&format!(
            r#"FUNC body() AS Nothing
  MUT xs AS List OF RES fs::File = []
  IF len("x") = 1 THEN
    RES f AS fs::File = openBuffered()
    xs = collections::append(xs, f)
  END IF
  fs::writeAll(collections::get(xs, 0), "F")
END FUNC
{LOOP_MAIN}"#
        )),
    );
    assert_eq!(out.trim(), "F");
}

/// The report's second trigger: a same-scope list that is only iterated never
/// freed its own block.
#[test]
fn an_iterated_list_frees_its_block() {
    let out = assert_balanced(
        "b1000_for_each",
        &list_program(
            r#"FUNC once() AS Integer
  MUT xs AS List OF RES fs::File = []
  RES f AS fs::File = openData()
  xs = collections::append(xs, f)
  MUT n AS Integer = 0
  FOR EACH h IN xs
    n = n + len(fs::readLine(h))
  NEXT
  RETURN n
END FUNC

FUNC main() AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO count()
    t = t + once()
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC
"#,
        ),
    );
    assert_eq!(out.trim(), "50");
}

/// Floated AND iterated: the drain reclaims the record and its 16 KiB read buffer.
#[test]
fn a_floated_element_of_an_iterated_list_is_freed() {
    let out = assert_balanced(
        "b1000_floated_for_each",
        &list_program(
            r#"FUNC once() AS Integer
  MUT xs AS List OF RES fs::File = []
  IF len("x") = 1 THEN
    RES f AS fs::File = openData()
    xs = collections::append(xs, f)
  END IF
  MUT n AS Integer = 0
  FOR EACH h IN xs
    n = n + len(fs::readLine(h))
  NEXT
  RETURN n + len(fs::readLine(collections::get(xs, 0)))
END FUNC

FUNC main() AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO count()
    t = t + once()
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC
"#,
        ),
    );
    assert_eq!(out.trim(), "110", "`first` then `second`, ten times");
}

/// An adopted list that is iterated and passed to a callee that only iterates it.
#[test]
fn an_adopted_list_iterated_here_and_in_a_callee_is_freed() {
    let out = assert_balanced(
        "b1000_adopted_iterated",
        &list_program(
            r#"FUNC total(xs AS List OF RES fs::File) AS Integer
  MUT n AS Integer = 0
  FOR EACH h IN xs
    n = n + len(fs::readLine(h))
  NEXT
  RETURN n
END FUNC

FUNC once() AS Integer
  LET xs = mkData()
  MUT n AS Integer = total(xs)
  FOR EACH h IN xs
    n = n + len(fs::readLine(h))
  NEXT
  RETURN n
END FUNC

FUNC main() AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO count()
    t = t + once()
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC
"#,
        ),
    );
    assert_eq!(out.trim(), "110");
}

/// A floated element handed to a named function that binds its parameter (here,
/// a `MATCH` view of it) still belongs to the list: what a named callee keeps dies
/// with its frame. Only code the proof cannot read disqualifies an element.
#[test]
fn a_floated_element_given_to_a_named_callee_is_freed() {
    let out = assert_balanced(
        "b1000_named_callee",
        &list_program(
            r#"FUNC firstLine(h AS fs::File) AS Integer
  RES view AS fs::File = h
  RETURN len(fs::readLine(view))
END FUNC

FUNC once() AS Integer
  MUT xs AS List OF RES fs::File = []
  MUT n AS Integer = 0
  IF len("x") = 1 THEN
    RES f AS fs::File = openData()
    n = firstLine(f)
    xs = collections::append(xs, f)
  END IF
  RETURN n + len(xs)
END FUNC

FUNC main() AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO count()
    t = t + once()
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC
"#,
        ),
    );
    assert_eq!(out.trim(), "60");
}

// The shapes below must NOT be freed by the list: something else still holds the
// record. Each uses the handle after the point a wrong free would have run, and
// each must stay free of double frees.

/// A list built around the caller's own binding is adopted in the same scope as
/// that binding. The binding owns the record; the list must not list it. Before
/// this fix the drain freed the record and the binding's drop then read the freed
/// block's owner word.
#[test]
fn a_list_around_a_same_scope_binding_leaves_the_record_to_the_binding() {
    let out = assert_balanced(
        "b1000_same_scope_owner",
        &list_program(&format!(
            r#"FUNC body() AS Nothing
  RES a AS fs::File = openBuffered()
  IF len("x") = 1 THEN
    LET xs = wrap(a)
    fs::writeAll(collections::get(xs, 0), "1")
  END IF
  ' The inner list is gone; `a` is still this scope's and still open.
  io::write(poke(a, "2") & " ")
  LET ys = wrap(a)
  FOR EACH h IN ys
    fs::writeAll(h, "3")
  NEXT
END FUNC
{LOOP_MAIN}"#
        )),
    );
    assert_eq!(
        out.trim(),
        "open open open open open open open open open open 123"
    );
}

/// An element handed back out of the list by `RETURN` is the caller's: the list's
/// drain must neither close nor free it. (Not a balance case: a list an element
/// is returned out of keeps the close-only drain, as before.)
#[test]
fn an_element_returned_out_of_the_list_stays_open_for_the_caller() {
    let out = assert_no_double_free(
        "b1000_returned_element",
        &list_program(&format!(
            r#"FUNC pick() AS RES fs::File
  LET xs = mkList()
  fs::writeAll(collections::get(xs, 0), "a")
  RETURN collections::get(xs, 0)
END FUNC

FUNC body() AS Nothing
  RES f AS fs::File = pick()
  io::write(poke(f, "b") & " ")
END FUNC
{LOOP_MAIN}"#
        )),
    );
    assert_eq!(
        out.trim(),
        "open open open open open open open open open open ab"
    );
}

/// A callee hands an element back and the caller binds it: the binding is one more
/// reference in the list's scope, and the record is freed once.
#[test]
fn an_element_a_callee_hands_back_is_freed_once() {
    let out = assert_balanced(
        "b1000_callee_hands_back",
        &list_program(&format!(
            r#"FUNC body() AS Nothing
  LET xs = mkList()
  RES h AS fs::File = first(xs)
  fs::writeAll(h, "h")
  fs::writeAll(collections::get(xs, 0), "x")
END FUNC
{LOOP_MAIN}"#
        )),
    );
    assert_eq!(out.trim(), "hx");
}

/// An element stored into a longer-lived list takes that list's scope (§15.6; the
/// owner ruling of 2026-10-11 recorded in bug-1045): `keep` outlives `xs`, so the
/// handles the loop appends stay open until `body` returns and the inner list's
/// drain leaves them alone. Until bug-1045 nothing raised them, they closed with
/// the inner list, and this test pinned `closedclosed`: the outer list read a
/// closed handle. What it guards is unchanged: nothing is freed under the outer
/// list, and nothing is freed twice.
#[test]
fn an_element_copied_into_an_outer_list_takes_its_scope() {
    let exe = build_debug(
        "b1000_copied_out",
        &list_program(
            r#"FUNC body() AS String
  MUT keep AS List OF RES fs::File = []
  IF len("x") = 1 THEN
    LET xs = mkList()
    keep = collections::append(keep, collections::get(xs, 0))
    FOR EACH h IN xs
      keep = collections::append(keep, h)
    NEXT
  END IF
  RETURN poke(collections::get(keep, 0), "z") & poke(collections::get(keep, 1), "z")
END FUNC

FUNC main() AS Integer
  FOR i = 1 TO count()
    io::write(body() & " ")
  NEXT
  RETURN 0
END FUNC
"#,
        ),
    );
    let dir = exe.parent().expect("build dir").join("probe");
    std::fs::create_dir_all(&dir).expect("probe dir");
    let run = run_at("b1000_copied_out", &exe, &dir, 5);
    assert_eq!(run.double_free_skips, 0);
    assert_eq!(
        run.stdout.trim(),
        "openopen openopen openopen openopen openopen",
        "the outer list reads handles its own scope keeps open, not freed memory"
    );
}

/// The same rule for code the ownership proof cannot read. A callback handed each
/// element may store it in a longer-lived list. Before this fix the list passed to
/// `collections::forEach` still counted as the one owner: its drain freed the
/// record and the outer list read freed memory (it reported `ErrResourceMoved`, a
/// flag bit of the scrubbed block). A call through a function value is the other
/// such call; here its result is assigned to the outer list, which adopts what it
/// carries (§15.6), so those elements float outward and stay open — the inner
/// list's drain must leave them alone either way.
#[test]
fn an_element_a_callback_keeps_is_closed_not_freed() {
    let exe = build_debug(
        "b1000_callback_keeps",
        &list_program(
            r#"FUNC stash(keep AS List OF RES fs::File, h AS fs::File) AS List OF RES fs::File
  RETURN collections::append(keep, h)
END FUNC

FUNC viaCallback() AS String
  MUT keep AS List OF RES fs::File = []
  IF len("x") = 1 THEN
    MUT xs AS List OF RES fs::File = []
    IF len("x") = 1 THEN
      RES f AS fs::File = openBuffered()
      xs = collections::append(xs, f)
    END IF
    collections::forEach(xs, LAMBDA(h AS fs::File) -> keep = collections::append(keep, h))
  END IF
  RETURN poke(collections::get(keep, 0), "z")
END FUNC

FUNC viaFunctionValue(put AS FUNC(List OF RES fs::File, fs::File) AS List OF RES fs::File) AS String
  MUT keep AS List OF RES fs::File = []
  IF len("x") = 1 THEN
    LET xs = mkList()
    keep = put(keep, collections::get(xs, 0))
    FOR EACH h IN xs
      keep = put(keep, h)
    NEXT
  END IF
  RETURN poke(collections::get(keep, 0), "z") & poke(collections::get(keep, 1), "z")
END FUNC

FUNC main() AS Integer
  FOR i = 1 TO count()
    io::write(viaCallback() & " " & viaFunctionValue(stash) & " ")
  NEXT
  RETURN 0
END FUNC
"#,
        ),
    );
    let dir = exe.parent().expect("build dir").join("probe");
    std::fs::create_dir_all(&dir).expect("probe dir");
    let run = run_at("b1000_callback_keeps", &exe, &dir, 3);
    assert_eq!(run.double_free_skips, 0);
    // bug-1075: the store inside the `LAMBDA` raises the element to `keep`'s
    // scope (§15.6), so it is still open after the inner block, as the element a
    // function value handed back is.
    assert_eq!(
        run.stdout.trim(),
        "open openopen open openopen open openopen"
    );
    assert!(run.frees <= run.allocs);
}

/// A callback that only reads through its element keeps nothing, so the list it
/// is applied to is still freed.
#[test]
fn a_list_given_to_a_reading_callback_is_still_freed() {
    let out = assert_balanced(
        "b1000_reading_callback",
        &list_program(
            r#"FUNC once() AS Integer
  MUT xs AS List OF RES fs::File = []
  IF len("x") = 1 THEN
    RES f AS fs::File = openData()
    xs = collections::append(xs, f)
  END IF
  MUT n AS Integer = 0
  collections::forEach(xs, LAMBDA(h AS fs::File) -> n = n + len(fs::readLine(h)))
  RETURN n
END FUNC

FUNC main() AS Integer
  MUT t AS Integer = 0
  FOR i = 1 TO count()
    t = t + once()
  NEXT
  io::print(toString(t))
  RETURN 0
END FUNC
"#,
        ),
    );
    assert_eq!(out.trim(), "50");
}

// ---- bug-1044, bug-1056, bug-1043: what a call hands back and a binding or a
// ---- statement takes is freed with it ----

/// Run `exe` at `n` iterations with at most 64 open descriptors: a shape that
/// leaves one file open per iteration runs out within the first hundred.
fn run_limited(exe: &std::path::Path, dir: &std::path::Path, n: u32) -> (bool, String, String) {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg("ulimit -n 64 && exec \"$0\"")
        .arg(exe)
        .env("N", n.to_string())
        .env("PROBE_DIR", dir)
        .output()
        .expect("run the program under sh");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// [`assert_balanced`], then 300 iterations inside 64 descriptors printing
/// `at_300`.
fn assert_balanced_and_closed(name: &str, source: &str, at_10: &str, at_300: &str) {
    assert_eq!(assert_balanced(name, source), at_10, "{name}");
    let exe = build_debug(name, source);
    let dir = exe.parent().expect("build dir").join("probe");
    std::fs::create_dir_all(&dir).expect("probe dir");
    std::fs::write(dir.join("data.txt"), "first\nsecond\n").expect("data");
    let (ok, stdout, stderr) = run_limited(&exe, &dir, 300);
    assert!(
        ok && stdout == at_300,
        "{name}: 300 iterations must fit in 64 descriptors; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// bug-1044: a record built around a call that returns a `List OF RES`, returned
/// by a call and bound. The binding owns the record, the list block and the
/// file records, and frees them all with its scope.
#[test]
fn a_record_built_from_a_list_of_resources_frees_what_it_holds() {
    let source = program(
        "TYPE Box\n  items AS List OF RES fs::File\nEND TYPE\n\n\
         FUNC mkList() AS List OF RES fs::File\n\
         \x20 MUT xs AS List OF RES fs::File = []\n\
         \x20 RES f AS fs::File = fs::createTempFile()\n\
         \x20 xs = collections::append(xs, f)\n\
         \x20 RETURN xs\n\
         END FUNC\n\n\
         FUNC mkBox() AS Box\n\
         \x20 RETURN Box[items := mkList()]\n\
         END FUNC\n\n\
         FUNC once() AS Integer\n\
         \x20 LET b AS Box = mkBox()\n\
         \x20 RETURN len(b.items)\n\
         END FUNC\n\n\
         FUNC main AS Integer\n\
         \x20 MUT n AS Integer = 0\n\
         \x20 FOR i = 1 TO count()\n\
         \x20   n = n + once()\n\
         \x20 NEXT\n\
         \x20 io::print(\"n=\" & toString(n))\n\
         \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_balanced_and_closed("bug1044_box_from_call", &source, "n=10\n", "n=300\n");
}

/// bug-1056: a `List OF List OF RES fs::File` returned by a call and bound. The
/// binding frees the inner list blocks before the outer one.
#[test]
fn a_nested_list_of_resources_bound_from_a_call_frees_its_inner_blocks() {
    let source = program(
        "FUNC mkList() AS List OF RES fs::File\n\
         \x20 MUT xs AS List OF RES fs::File = []\n\
         \x20 RES f AS fs::File = fs::createTempFile()\n\
         \x20 xs = collections::append(xs, f)\n\
         \x20 RES g AS fs::File = fs::createTempFile()\n\
         \x20 xs = collections::append(xs, g)\n\
         \x20 RETURN xs\n\
         END FUNC\n\n\
         FUNC mkNested() AS List OF List OF RES fs::File\n\
         \x20 MUT outer AS List OF List OF RES fs::File = []\n\
         \x20 outer = collections::append(outer, mkList())\n\
         \x20 outer = collections::append(outer, mkList())\n\
         \x20 RETURN outer\n\
         END FUNC\n\n\
         FUNC main AS Integer\n\
         \x20 MUT n AS Integer = 0\n\
         \x20 FOR i = 1 TO count()\n\
         \x20   LET t AS List OF List OF RES fs::File = mkNested()\n\
         \x20   n = n + len(t)\n\
         \x20 NEXT\n\
         \x20 io::print(\"n=\" & toString(n))\n\
         \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_balanced_and_closed("bug1056_nested_from_call", &source, "n=20\n", "n=600\n");
}

/// bug-1043: a single resource a call returns, used unbound. Its statement closes
/// it and frees its record.
#[test]
fn a_resource_returned_by_a_call_and_used_unbound_frees_its_record() {
    let source = program(
        "FUNC openOne() AS RES fs::File\n\
         \x20 RETURN fs::openFile(dir() & \"/data.txt\")\n\
         END FUNC\n\n\
         FUNC once() AS Integer\n\
         \x20 RETURN len(fs::readAll(openOne()))\n\
         END FUNC\n\n\
         FUNC main AS Integer\n\
         \x20 MUT n AS Integer = 0\n\
         \x20 FOR i = 1 TO count()\n\
         \x20   n = n + once()\n\
         \x20 NEXT\n\
         \x20 io::print(\"n=\" & toString(n))\n\
         \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_balanced_and_closed("bug1043_unbound_resource", &source, "n=130\n", "n=3900\n");
}

/// bug-1043: the unbound resource is handed to a function that puts it in the
/// list it returns, and nothing takes that list either.
#[test]
fn a_resource_used_unbound_through_a_list_building_call_frees_its_record() {
    let source = program(
        "FUNC openOne() AS RES fs::File\n\
         \x20 RETURN fs::openFile(dir() & \"/data.txt\")\n\
         END FUNC\n\n\
         FUNC mkFrom(f AS fs::File) AS List OF RES fs::File\n\
         \x20 MUT xs AS List OF RES fs::File = []\n\
         \x20 xs = collections::append(xs, f)\n\
         \x20 RETURN xs\n\
         END FUNC\n\n\
         FUNC main AS Integer\n\
         \x20 MUT n AS Integer = 0\n\
         \x20 FOR i = 1 TO count()\n\
         \x20   n = n + len(mkFrom(openOne()))\n\
         \x20 NEXT\n\
         \x20 io::print(\"n=\" & toString(n))\n\
         \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_balanced_and_closed("bug1043_through_list", &source, "n=10\n", "n=300\n");
}
