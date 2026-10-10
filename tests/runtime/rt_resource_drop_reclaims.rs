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

const PRELUDE: &str = r#"IMPORT io
IMPORT os
IMPORT fs
IMPORT tcp
IMPORT collections

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
