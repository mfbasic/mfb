//! bug-755: a resource is closed by the outermost scope that holds it.
//!
//! A `RES` binding of a call used to ADOPT whatever came back, so a callee that
//! handed back a handle its caller passed in got that handle closed when an INNER
//! scope of the caller exited, before its real owner was done with it. The model
//! (the bug doc's "The model"): the resource holds its cleanup scope; every `RES`
//! is a reference; the scope only floats up; `RETURN` from the owning scope
//! releases it to the caller, and `RETURN` of a resource owned further out does not.
//!
//! Each case asserts the exact output the model predicts. A read handle's state
//! (`open`/`CLOSED`) shows whether a passed-in handle was closed early; a FRESH
//! handle writes buffered bytes, so the file's content shows whether (and when) it
//! was closed, because only a close flushes it.

#[path = "../common/mod.rs"]
mod common;

const PRELUDE: &str = r#"IMPORT io
IMPORT thread
IMPORT fs
IMPORT os
IMPORT collections
IMPORT errorCode

FUNC dir() AS String
  RETURN os::getEnv("PROBE_DIR")
END FUNC

FUNC state(h AS fs::File) AS String
  LET line = fs::readLine(h) TRAP(e)
    IF e.code = errorCode::ErrResourceClosed THEN
      RECOVER "CLOSED"
    END IF
    RECOVER "open"
  END TRAP
  IF line = "CLOSED" THEN
    RETURN "CLOSED"
  END IF
  RETURN "open"
END FUNC

FUNC ident(h AS fs::File) AS RES fs::File
  RETURN h
END FUNC

FUNC same(xs AS List OF RES fs::File) AS List OF RES fs::File
  RETURN xs
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
"#;

fn run(name: &str, body: &str) -> Vec<String> {
    let source = format!("{PRELUDE}\n{body}");
    let project = common::temp_project(name, &source);
    let probe = project.join("probe");
    std::fs::create_dir_all(&probe).expect("probe dir");
    std::fs::write(probe.join("data.txt"), "hello\nworld\nagain\n").expect("data");
    let executable = common::build_project(&project);
    let out = common::run_with_env(&executable, &[("PROBE_DIR", probe.to_str().unwrap())]);
    out.lines().map(str::to_string).collect()
}

#[test]
fn a_handed_back_handle_outlives_an_inner_binding() {
    let lines = run(
        "b755_ident",
        r#"FUNC main() AS Nothing
  RES a AS fs::File = fs::open(dir() & "/data.txt", "read")
  IF TRUE THEN
    RES y AS fs::File = ident(a)
  END IF
  io::print("a: " & state(a))
END FUNC
"#,
    );
    assert_eq!(lines, ["a: open"]);
}

#[test]
fn a_handed_back_resource_list_outlives_an_inner_binding() {
    let lines = run(
        "b755_same",
        r#"FUNC main() AS Nothing
  RES b AS fs::File = fs::open(dir() & "/data.txt", "read")
  LET lb AS List OF RES fs::File = [b]
  IF TRUE THEN
    LET p = same(lb)
  END IF
  io::print("b: " & state(b))
END FUNC
"#,
    );
    assert_eq!(lines, ["b: open"]);
}

#[test]
fn a_mixed_list_return_closes_only_the_fresh_handle() {
    let lines = run(
        "b755_mixed_list",
        r#"FUNC addFresh(xs AS List OF RES fs::File) AS List OF RES fs::File
  RETURN collections::append(xs, fresh("mixed.txt"))
END FUNC

FUNC main() AS Nothing
  RES b AS fs::File = fs::open(dir() & "/data.txt", "read")
  LET lb AS List OF RES fs::File = [b]
  IF TRUE THEN
    LET q = addFresh(lb)
  END IF
  io::print("fresh: " & flushed("mixed.txt"))
  io::print("b: " & state(b))
END FUNC
"#,
    );
    assert_eq!(lines, ["fresh: flushed", "b: open"]);
}

#[test]
fn a_mixed_single_return_closes_by_whose_handle_it_is() {
    let lines = run(
        "b755_mixed_single",
        r#"FUNC pick(c AS Boolean, h AS fs::File) AS RES fs::File
  IF c THEN
    RETURN h
  END IF
  RETURN fresh("pick.txt")
END FUNC

FUNC main() AS Nothing
  RES a AS fs::File = fs::open(dir() & "/data.txt", "read")
  IF TRUE THEN
    RES p AS fs::File = pick(TRUE, a)
  END IF
  io::print("passed: " & state(a))
  IF TRUE THEN
    RES q AS fs::File = pick(FALSE, a)
  END IF
  io::print("fresh: " & flushed("pick.txt"))
  io::print("passed again: " & state(a))
END FUNC
"#,
    );
    assert_eq!(
        lines,
        ["passed: open", "fresh: flushed", "passed again: open"]
    );
}

#[test]
fn a_discarded_call_result_closes_only_what_it_produced() {
    let lines = run(
        "b755_discarded",
        r#"FUNC main() AS Nothing
  RES a AS fs::File = fs::open(dir() & "/data.txt", "read")
  ident(a)
  io::print("a: " & state(a))
  fresh("dropped.txt")
  io::print("fresh: " & flushed("dropped.txt"))
END FUNC
"#,
    );
    assert_eq!(lines, ["a: open", "fresh: flushed"]);
}

#[test]
fn a_handle_floated_into_an_outer_collection_closes_once_at_that_scope() {
    let lines = run(
        "b755_float",
        r#"FUNC addTo(xs AS List OF RES fs::File, h AS fs::File) AS List OF RES fs::File
  RETURN collections::append(xs, h)
END FUNC

FUNC main() AS Nothing
  MUT outer AS List OF RES fs::File = []
  IF TRUE THEN
    RES f AS fs::File = fs::open(dir() & "/data.txt", "read")
    outer = addTo(outer, f)
  END IF
  io::print("floated: " & state(collections::get(outer, 0)))
END FUNC
"#,
    );
    assert_eq!(lines, ["floated: open"]);
}

#[test]
fn a_handed_back_record_field_outlives_an_inner_binding() {
    let lines = run(
        "b755_record",
        r#"TYPE Holder
  label  AS String
  handle AS RES fs::File
END TYPE

FUNC wrap(f AS fs::File) AS Holder
  LET h = Holder["made", f]
  RETURN h
END FUNC

FUNC main() AS Nothing
  RES a AS fs::File = fs::open(dir() & "/data.txt", "read")
  IF TRUE THEN
    LET h AS Holder = wrap(a)
  END IF
  io::print("a: " & state(a))
END FUNC
"#,
    );
    assert_eq!(lines, ["a: open"]);
}

/// Case 8: a transferred handle is claimed by the scope that accepts it. The
/// worker accepts inside an inner block, writes through the buffer, and reads the
/// file back after the block: the bytes are there only if that block's exit
/// closed (flushed) the handle. A second close at the worker's or the sender's
/// exit would be a cleanup failure; the clean `main` result rules it out.
#[test]
fn an_accepted_handle_closes_at_the_accepting_scope() {
    let lines = run(
        "b755_accept",
        r#"ISOLATED FUNC worker(t AS ThreadWorker OF RES fs::File TO Integer, path AS String) AS Integer
  IF TRUE THEN
    RES f AS fs::File = thread::accept(t, 20000)
    fs::writeAll(f, "accepted")
  END IF
  RETURN len(fs::readText(path))
END FUNC

FUNC main() AS Nothing
  LET path AS String = dir() & "/accept.txt"
  RES a AS Thread OF RES fs::File TO Integer = thread::start(worker, path)
  RES f AS fs::File = fs::openFile(path, "w")
  fs::setBuffered(f, TRUE)
  thread::transfer(a, f)
  io::print("worker read " & toString(thread::waitFor(a)))
END FUNC
"#,
    );
    assert_eq!(lines, ["worker read 8"]);
}
