//! bug-806: a resource stored into a module-level aggregate.
//!
//! `put()` produced `RES f` and appended `Holder[f]` to a module-level
//! `MUT saved AS List OF Holder`. The escape analysis placed the global at
//! `put`'s own depth, so `f` closed at `put`'s exit and its record was reclaimed
//! while `saved` still pointed at it: the next read through the global read freed
//! memory (it reported a `thread::transfer` move that never happened).
//!
//! The contract: a module-level binding is the outermost scope there is (depth
//! -1). A resource stored into one — produced here, or arriving through a
//! parameter from a scope that exits first — is owned by the module instance and
//! closed exactly once at instance teardown: program exit (normal, error, or
//! `os::exit`) for the main instance, thread exit for a worker's own
//! instance. Removing the last global view does not close it; an explicit close
//! still does, and the teardown close is then a no-op.
//!
//! Each case writes through a FRESH buffered handle, whose bytes reach the file
//! only when the handle is closed: an empty file while the program runs proves it
//! is still open, and `flushed` read back by this test AFTER the process exited
//! proves the teardown closed it.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const PRELUDE: &str = r#"IMPORT io
IMPORT fs
IMPORT os
IMPORT thread
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
    probe: std::path::PathBuf,
}

impl Run {
    /// A probe file's content after the process exited.
    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.probe.join(name)).unwrap_or_else(|_| "<missing>".into())
    }
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
        probe,
    }
}

/// Build and run, requiring a clean exit with no cleanup failure (a second close
/// of a live handle is reported there).
fn run(name: &str, body: &str) -> Run {
    let run = build_and_run(name, body, false);
    assert_eq!(
        run.status, 0,
        "{name}: exit status; stderr:\n{}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("Cleanup failure"),
        "{name}: a cleanup failed:\n{}",
        run.stderr
    );
    run
}

fn double_free_skips(name: &str, run: &Run) -> u64 {
    let line = run
        .stderr
        .lines()
        .find(|line| line.starts_with("arena.0.double_free_skips "))
        .unwrap_or_else(|| panic!("{name}: no double_free_skips line:\n{}", run.stderr));
    line.rsplit(' ').next().unwrap().parse().expect("counter")
}

/// The bug document's reproduction: the global's handle reads the first line.
#[test]
fn a_resource_appended_to_a_global_stays_readable() {
    let run = run(
        "b806_primary",
        r#"FUNC open() AS RES fs::File
  RETURN fs::openFile(dir() & "/data.txt")
END FUNC

FUNC read(f AS fs::File) AS String
  LET line = fs::readLine(f) TRAP(e)
    RECOVER e.message
  END TRAP
  RETURN line
END FUNC

MUT saved AS List OF Holder = []

FUNC put() AS Nothing
  RES f = open()
  saved = collections::append(saved, Holder[f])
END FUNC

FUNC main() AS Nothing
  put()
  LET h = collections::get(saved, 0)
  io::print(read(h.handle))
END FUNC
"#,
    );
    assert_eq!(run.lines, ["first"]);
}

/// Open while the program runs, closed once at program exit, every block freed
/// at most once.
#[test]
fn a_global_resource_closes_once_at_program_exit() {
    let body = r#"MUT saved AS List OF Holder = []

FUNC put() AS Nothing
  RES f = fresh("global.txt")
  saved = collections::append(saved, Holder[f])
END FUNC

FUNC main() AS Nothing
  put()
  io::print("poke: " & poke(collections::get(saved, 0).handle))
  io::print("running: [" & flushed("global.txt") & "]")
END FUNC
"#;
    let run = build_and_run("b806_exit_debug", body, true);
    assert_eq!(run.status, 0, "stderr:\n{}", run.stderr);
    assert_eq!(run.lines, ["poke: open", "running: []"], "{}", run.stderr);
    assert!(!run.stderr.contains("Cleanup failure"), "{}", run.stderr);
    assert_eq!(run.file("global.txt"), "flushed+");
    assert_eq!(double_free_skips("b806_exit_debug", &run), 0);
}

/// A handle that arrives through a parameter from a caller scope that exits
/// before the global is read: the store raises it to the module instance.
#[test]
fn a_parameter_stored_into_a_global_outlives_its_callers_scope() {
    let run = run(
        "b806_param",
        r#"MUT saved AS List OF Holder = []

FUNC keep(f AS fs::File) AS Nothing
  saved = collections::append(saved, Holder[f])
END FUNC

FUNC main() AS Nothing
  IF TRUE THEN
    RES f AS fs::File = fresh("param.txt")
    keep(f)
  END IF
  io::print("poke: " & poke(collections::get(saved, 0).handle))
  io::print("running: [" & flushed("param.txt") & "]")
END FUNC
"#,
    );
    assert_eq!(run.lines, ["poke: open", "running: []"]);
    assert_eq!(run.file("param.txt"), "flushed+");
}

/// A record parameter carrying a handle, stored whole into a global.
#[test]
fn a_record_parameter_stored_into_a_global_outlives_its_callers_scope() {
    let run = run(
        "b806_record_param",
        r#"MUT saved AS List OF Holder = []

FUNC keep(h AS Holder) AS Nothing
  saved = collections::append(saved, h)
END FUNC

FUNC main() AS Nothing
  IF TRUE THEN
    RES f AS fs::File = fresh("recparam.txt")
    keep(Holder[f])
  END IF
  io::print("poke: " & poke(collections::get(saved, 0).handle))
  io::print("running: [" & flushed("recparam.txt") & "]")
END FUNC
"#,
    );
    assert_eq!(run.lines, ["poke: open", "running: []"]);
    assert_eq!(run.file("recparam.txt"), "flushed+");
}

#[test]
fn a_flat_global_resource_list_keeps_its_handles() {
    let run = run(
        "b806_flat",
        r#"MUT files AS List OF RES fs::File = []

FUNC add() AS Nothing
  RES f AS fs::File = fresh("flat.txt")
  files = collections::append(files, f)
END FUNC

FUNC main() AS Nothing
  add()
  io::print("poke: " & poke(collections::get(files, 0)))
  io::print("running: [" & flushed("flat.txt") & "]")
END FUNC
"#,
    );
    assert_eq!(run.lines, ["poke: open", "running: []"]);
    assert_eq!(run.file("flat.txt"), "flushed+");
}

#[test]
fn a_nested_global_aggregate_keeps_its_handles() {
    let run = run(
        "b806_nested",
        r#"MUT grid AS List OF List OF Holder = []

FUNC add() AS Nothing
  RES f AS fs::File = fresh("nested.txt")
  grid = collections::append(grid, [Holder[f]])
END FUNC

FUNC main() AS Nothing
  add()
  io::print("poke: " & poke(collections::get(collections::get(grid, 0), 0).handle))
  io::print("running: [" & flushed("nested.txt") & "]")
END FUNC
"#,
    );
    assert_eq!(run.lines, ["poke: open", "running: []"]);
    assert_eq!(run.file("nested.txt"), "flushed+");
}

/// Removing the last global view does not close the resource or move its
/// cleanup inward: it still closes at program exit.
#[test]
fn removing_the_last_global_view_does_not_close() {
    let run = run(
        "b806_removed",
        r#"MUT saved AS List OF Holder = []

FUNC put() AS Nothing
  RES f = fresh("removed.txt")
  saved = collections::append(saved, Holder[f])
END FUNC

FUNC main() AS Nothing
  put()
  saved = []
  io::print("running: [" & flushed("removed.txt") & "] " & toString(len(saved)))
END FUNC
"#,
    );
    assert_eq!(run.lines, ["running: [] 0"]);
    assert_eq!(run.file("removed.txt"), "flushed");
}

/// An explicit close through the global view still closes; the teardown close is
/// then the defined no-op, not a cleanup failure.
#[test]
fn an_explicit_close_through_the_global_is_honoured() {
    let run = run(
        "b806_closed",
        r#"MUT saved AS List OF Holder = []

FUNC put() AS Nothing
  RES f = fresh("closed.txt")
  saved = collections::append(saved, Holder[f])
END FUNC

FUNC main() AS Nothing
  put()
  RES h AS fs::File = collections::get(saved, 0).handle
  fs::close(h)
  io::print("closed: " & flushed("closed.txt"))
  io::print("poke: " & poke(collections::get(saved, 0).handle))
END FUNC
"#,
    );
    assert_eq!(run.lines, ["closed: flushed", "poke: CLOSED"]);
    assert_eq!(run.file("closed.txt"), "flushed");
}

/// `os::exit` from a nested call is an instance teardown too.
#[test]
fn exit_program_closes_global_resources() {
    let run = build_and_run(
        "b806_exit_program",
        r#"MUT saved AS List OF Holder = []

FUNC put() AS Nothing
  RES f = fresh("exitprog.txt")
  saved = collections::append(saved, Holder[f])
END FUNC

FUNC quit() AS Nothing
  os::exit(3)
END FUNC

FUNC main() AS Nothing
  put()
  io::print("running: [" & flushed("exitprog.txt") & "]")
  quit()
END FUNC
"#,
        false,
    );
    assert_eq!(run.status, 3, "stderr:\n{}", run.stderr);
    assert_eq!(run.lines, ["running: []"]);
    assert_eq!(run.file("exitprog.txt"), "flushed");
}

/// An untrapped error ending the program is an instance teardown too.
#[test]
fn an_untrapped_error_closes_global_resources() {
    let run = build_and_run(
        "b806_error_exit",
        r#"MUT saved AS List OF Holder = []

FUNC put() AS Nothing
  RES f = fresh("errexit.txt")
  saved = collections::append(saved, Holder[f])
END FUNC

FUNC main() AS Nothing
  put()
  io::print("running: [" & flushed("errexit.txt") & "]")
  FAIL error(77050002, "boom")
END FUNC
"#,
        false,
    );
    assert_ne!(run.status, 0, "stderr:\n{}", run.stderr);
    assert!(run.stderr.contains("boom"), "{}", run.stderr);
    assert_eq!(run.lines, ["running: []"]);
    assert_eq!(run.file("errexit.txt"), "flushed");
}

/// A worker runs its own instance of the project: a resource it stores into its
/// copy of the global stays open after the storing function returns, closes when
/// the worker exits (not at the parent's exit), and the parent's copy of the
/// global never sees it.
#[test]
fn a_workers_global_resource_closes_at_worker_exit() {
    let run = run(
        "b806_worker",
        r#"MUT saved AS List OF Holder = []

FUNC put(name AS String) AS Nothing
  RES f AS fs::File = fresh(name)
  saved = collections::append(saved, Holder[f])
END FUNC

ISOLATED FUNC worker(t AS ThreadWorker OF Integer TO Integer, name AS String) AS Integer
  put(name)
  IF poke(collections::get(saved, 0).handle) = "open" THEN
    RETURN len(saved)
  END IF
  RETURN -1
END FUNC

FUNC main() AS Nothing
  RES t AS Thread OF Integer TO Integer = thread::start(worker, "worker.txt")
  LET n = thread::waitFor(t)
  io::print("worker kept " & toString(n) & "; parent has " & toString(len(saved)))
  io::print("after worker: " & flushed("worker.txt"))
END FUNC
"#,
    );
    assert_eq!(
        run.lines,
        ["worker kept 1; parent has 0", "after worker: flushed+"]
    );
}

/// A global initializer that stores a resource: owned by the instance from the
/// start, closed at exit.
#[test]
fn a_global_initializer_resource_closes_at_program_exit() {
    let run = run(
        "b806_initializer",
        r#"MUT boot AS List OF Holder = [Holder[fresh("boot.txt")]]

FUNC main() AS Nothing
  io::print("poke: " & poke(collections::get(boot, 0).handle))
  io::print("running: [" & flushed("boot.txt") & "]")
END FUNC
"#,
    );
    assert_eq!(run.lines, ["poke: open", "running: []"]);
    assert_eq!(run.file("boot.txt"), "flushed+");
}

/// Many stores: each record is raised and registered once, every handle closes
/// once at exit, and nothing is freed twice.
#[test]
fn repeated_global_stores_close_each_handle_once() {
    let body = r#"MUT saved AS List OF Holder = []

FUNC put(i AS Integer) AS Nothing
  RES f = fresh("many" & toString(i) & ".txt")
  saved = collections::append(saved, Holder[f])
END FUNC

FUNC main() AS Nothing
  FOR i = 1 TO 40
    put(i)
  NEXT
  MUT open = 0
  FOR EACH h IN saved
    IF poke(h.handle) = "open" THEN
      open = open + 1
    END IF
  NEXT
  io::print("open: " & toString(open))
END FUNC
"#;
    let run = build_and_run("b806_many_debug", body, true);
    assert_eq!(run.status, 0, "stderr:\n{}", run.stderr);
    assert_eq!(run.lines, ["open: 40"], "{}", run.stderr);
    assert!(!run.stderr.contains("Cleanup failure"), "{}", run.stderr);
    for i in 1..=40 {
        assert_eq!(run.file(&format!("many{i}.txt")), "flushed+", "file {i}");
    }
    assert_eq!(double_free_skips("b806_many_debug", &run), 0);
}

/// The local twin of the module case: a handle floated from an inner scope into an
/// outer `List OF Holder` closes at the container's own exit. (This one aggregate
/// layer failed native lowering before bug-805, like the nested case.)
#[test]
fn a_local_outer_container_float_is_unchanged() {
    let run = run(
        "b806_local_control",
        r#"FUNC fill() AS Nothing
  MUT keep AS List OF Holder = []
  IF TRUE THEN
    RES f = fresh("local.txt")
    keep = collections::append(keep, Holder[f])
  END IF
  io::print("poke: " & poke(collections::get(keep, 0).handle))
  io::print("inside: [" & flushed("local.txt") & "]")
END FUNC

FUNC main() AS Nothing
  fill()
  io::print("after: " & flushed("local.txt"))
END FUNC
"#,
    );
    assert_eq!(run.lines, ["poke: open", "inside: []", "after: flushed+"]);
}

/// A resource-union handle arriving through a parameter from a scope that exits
/// first: the union's own box is that scope's, so the module lists the active
/// variant's record and tag, not the box.
#[test]
fn a_union_parameter_stored_into_a_global_outlives_its_callers_scope() {
    let run = run(
        "b806_union_param",
        r#"UNION Stream
  fs::File
  tcp::Socket
END UNION

MUT streams AS List OF RES Stream = []

FUNC keep(s AS Stream) AS Nothing
  streams = collections::append(streams, s)
END FUNC

FUNC main() AS Nothing
  IF TRUE THEN
    RES g AS Stream = fresh("unionparam.txt")
    keep(g)
  END IF
  MATCH collections::get(streams, 0)
    CASE fs::File(f)
      io::print("poke: " & poke(f))
    CASE tcp::Socket(t)
      io::print("socket")
  END MATCH
  io::print("running: [" & flushed("unionparam.txt") & "]")
END FUNC
"#,
    );
    assert_eq!(run.lines, ["poke: open", "running: []"]);
    assert_eq!(run.file("unionparam.txt"), "flushed+");
}

#[test]
fn a_global_map_of_records_keeps_its_handles() {
    let run = run(
        "b806_map",
        r#"MUT byName AS Map OF String TO Holder = Map OF String TO Holder {}

FUNC add(name AS String) AS Nothing
  RES f = fresh(name)
  byName = collections::set(byName, name, Holder[f])
END FUNC

FUNC main() AS Nothing
  add("map.txt")
  io::print("poke: " & poke(collections::get(byName, "map.txt").handle))
  io::print("running: [" & flushed("map.txt") & "]")
END FUNC
"#,
    );
    assert_eq!(run.lines, ["poke: open", "running: []"]);
    assert_eq!(run.file("map.txt"), "flushed+");
}

/// A record produced by a call and stored straight into a global: its handle has
/// no name before the store, so the store walks the stored global afterwards.
#[test]
fn a_call_result_stored_into_a_global_is_raised_after_the_store() {
    let run = run(
        "b806_call_result",
        r#"MUT saved AS List OF Holder = []

FUNC make(name AS String) AS List OF Holder
  MUT out AS List OF Holder = []
  RES f = fresh(name)
  out = collections::append(out, Holder[f])
  RETURN out
END FUNC

FUNC put() AS Nothing
  saved = collections::append(saved, make("call.txt"))
END FUNC

FUNC main() AS Nothing
  put()
  io::print("poke: " & poke(collections::get(saved, 0).handle))
  io::print("running: [" & flushed("call.txt") & "]")
END FUNC
"#,
    );
    assert_eq!(run.lines, ["poke: open", "running: []"]);
    assert_eq!(run.file("call.txt"), "flushed+");
}
