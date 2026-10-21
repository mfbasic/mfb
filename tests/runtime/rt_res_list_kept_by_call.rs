//! bug-801: a `List OF RES` block stored INSIDE another collection by a call was
//! still freed at its own binding's scope exit. `record_ownership` counted every
//! bare-local call argument as a use, but `collections::append(out, piece)` keeps
//! `piece`'s block pointer inside `out`, and so does a callee whose body appends
//! its parameter. The outer list then held a freed block: `len` of the inner list
//! read garbage and the element read faulted (SIGSEGV).
//!
//! Each program builds a `List OF List OF RES fs::File` in a callee, returns it, and
//! reads the inner list and its handle back out in the caller.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const PRELUDE: &str = r#"IMPORT io
IMPORT fs
IMPORT os
IMPORT collections
"#;

const MAIN: &str = r#"
FUNC main() AS Nothing
  LET path AS String = os::getEnv("PROBE")
  fs::writeText(path, "abc\n")
  RES d AS fs::File = fs::open(path, "read")
  LET w = wrap(d)
  LET inner = collections::get(w, 0)
  io::print("len=" & toString(len(inner)))
  RES pulled AS fs::File = collections::get(inner, 0)
  io::print(fs::readLine(pulled))
END FUNC
"#;

/// `piece` is appended into `out` directly.
const DIRECT: &str = r#"
FUNC wrap(f AS fs::File) AS List OF List OF RES fs::File
  MUT out AS List OF List OF RES fs::File = []
  LET piece AS List OF RES fs::File = [f]
  out = collections::append(out, piece)
  RETURN out
END FUNC
"#;

/// `piece` is handed to a user FUNC that appends it.
const THROUGH_CALLEE: &str = r#"
FUNC box(xs AS List OF RES fs::File) AS List OF List OF RES fs::File
  MUT out AS List OF List OF RES fs::File = []
  out = collections::append(out, xs)
  RETURN out
END FUNC

FUNC wrap(f AS fs::File) AS List OF List OF RES fs::File
  LET piece AS List OF RES fs::File = [f]
  RETURN box(piece)
END FUNC
"#;

/// `collections::window`'s body binds each piece from `__collections_slice` and
/// appends it to the result (the bug-643 fixture's `dwins` shape).
const THROUGH_WINDOW: &str = r#"
FUNC wrap(f AS fs::File) AS List OF List OF RES fs::File
  LET l AS List OF RES fs::File = [f]
  RETURN collections::window(l, 1)
END FUNC
"#;

fn run(name: &str, functions: &str) {
    let source = format!("{PRELUDE}{functions}{MAIN}");
    let project = common::temp_project(name, &source);
    let executable = common::build_project(&project);
    let output = Command::new(&executable)
        .env("PROBE", project.join("probe.txt"))
        .output()
        .expect("run the probe");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "probe failed ({:?}):\n{stdout}{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        vec!["len=1", "abc"],
        "the inner list and its handle survive the callee's scope exit"
    );
}

#[test]
fn a_list_appended_into_a_returned_list_keeps_its_block() {
    run("bug801_direct", DIRECT);
}

#[test]
fn a_list_a_callee_appends_keeps_its_block() {
    run("bug801_callee", THROUGH_CALLEE);
}

#[test]
fn a_window_piece_keeps_its_block() {
    run("bug801_window", THROUGH_WINDOW);
}
