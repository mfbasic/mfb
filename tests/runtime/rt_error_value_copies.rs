//! Regression test for bug-602: an `Error` value (and the `ErrorLoc` inside it) is
//! copied like any other value — no two bindings ever share one block.
//!
//! ## Why this exists
//!
//! bug-601 found that a `MUT` copy of a `List OF net::Address` shared its source's
//! block, because that record sat on the pointer-`String` layout, which is not
//! `memcpy`-copyable: the compiler made no copy at the bind, and the in-place
//! `removeAt`/`append` arms then emptied or freed the source. A stale doc comment
//! listed `Error`/`ErrorLoc` in the same class, so bug-602 asked whether they alias
//! too.
//!
//! They do not. `TypeModel` registers `Error` and `ErrorLoc` as ordinary records
//! (`validation.rs`), so their `String` and `ErrorLoc` fields are inlined and the
//! whole value is one flat block. Every shape below printed the correct values on
//! the compiler before plan-132 as well as after it. This test pins that, so a
//! future change that moves `Error` off the flat layout fails here rather than as a
//! silent wrong value or a use-after-free.
//!
//! ## What it runs
//!
//! One program, one line per way a second name can reach an `Error`: a copy of a
//! `TRAP` binding, bug-601's three in-place list shapes on a `List OF Error`, a
//! record field, a function parameter, a closure that outlives its scope, and a
//! `FAIL` that re-raises a binding still in scope. Each line reads the source after
//! the copy changed, so a shared block shows up as a wrong value or a crash.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const ERROR_COPY_SOURCE: &str = r#"IMPORT io
IMPORT collections

TYPE Holder
  err AS Error
  n AS Integer
END TYPE

FUNC boom(code AS Integer, msg AS String) AS Integer
  FAIL error(code, msg)
END FUNC

FUNC pass(x AS Error) AS Error
  RETURN x
END FUNC

FUNC makeF() AS FUNC(Integer) AS String
  LET a = error(12, "escaped")
  RETURN LAMBDA(n AS Integer) -> a.message & toString(n)
END FUNC

FUNC rethrow(x AS Error) AS Integer
  FAIL x
END FUNC

FUNC main AS Integer
  LET v = boom(7, "first") TRAP(e)
    LET e2 = e
    MUT e3 = e
    e3 = error(8, "second")
    io::print("trap e=" & e.message & " e2=" & e2.message & " e3=" & e3.message)
    RECOVER 0
  END TRAP

  MUT build AS List OF Error = []
  build = collections::append(build, error(1, "one"))
  LET xs = build

  MUT removed = xs
  removed = collections::removeAt(removed, 0)
  io::print("removed ys=" & toString(len(removed)) & " xs=" & toString(len(xs)))

  MUT grown = xs
  grown = collections::append(grown, error(2, "two"))
  io::print("grown ys=" & toString(len(grown)) & " xs=" & toString(len(xs)))

  MUT fetched = xs
  fetched = collections::append(fetched, collections::get(xs, 0))
  LET f1 = collections::get(fetched, 1)
  io::print("fetched ys=" & toString(len(fetched)) & " xs=" & toString(len(xs)) & " ys1=" & f1.message)

  LET first = collections::get(xs, 0)
  io::print("source " & toString(first.code) & "/" & first.message)

  LET h = Holder[error(5, "held"), 1]
  MUT h2 = h
  h2 = WITH h2 { err := error(6, "replaced") }
  io::print("holder h=" & h.err.message & " h2=" & h2.err.message)

  LET a = error(9, "nine")
  MUT c = pass(a)
  c = error(10, "ten")
  io::print("param a=" & a.message & " c=" & c.message)

  LET g = makeF()
  io::print("closure " & g(1))

  LET r = rethrow(a) TRAP(t)
    io::print("rethrown t=" & t.message & " a=" & a.message)
    RECOVER 0
  END TRAP
  io::print("after a=" & a.message & " v=" & toString(v + r))
  RETURN 0
END FUNC
"#;

#[test]
fn an_error_value_copy_is_independent_of_its_source() {
    let project = common::temp_project("bug602_error_copies_independent", ERROR_COPY_SOURCE);
    let exe = common::build_project(&project);

    let output = Command::new(&exe)
        .output()
        .expect("run the Error copy probe");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        output.status.success(),
        "bug-602: copying or mutating a copy of an Error crashed (status {:?}): two \
         bindings share one Error block.\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
    assert_eq!(
        stdout.trim(),
        [
            "trap e=first e2=first e3=second",
            "removed ys=0 xs=1",
            "grown ys=2 xs=1",
            "fetched ys=2 xs=1 ys1=one",
            "source 1/one",
            "holder h=held h2=replaced",
            "param a=nine c=ten",
            "closure escaped1",
            "rethrown t=nine a=nine",
            "after a=nine v=0",
        ]
        .join("\n"),
        "bug-602: every copy of an Error must change only itself.\nstderr:\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&project);
}
