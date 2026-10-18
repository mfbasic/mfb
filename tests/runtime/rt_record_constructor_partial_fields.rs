//! bug-1021: a by-field record constructor may name SOME of the fields; each field it
//! leaves out takes its default — the value an uninitialized `MUT x AS T` holds.
//! A positional constructor keeps requiring every field, an unknown field name keeps
//! its diagnostics, and an omitted field with no default is refused, naming it.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::{build_project, build_project_opt, mfb_exe, run_with_stdin, temp_project};

const RUN: &str = "IMPORT io
IMPORT color
IMPORT collections

ENUM Mode
  Slow, Fast
END ENUM

TYPE Inner
  n AS Integer
  s AS String
END TYPE

TYPE Ck
  name AS String
  value AS String
  port AS Integer
  secure AS Boolean
  ratio AS Float
  fx AS Fixed
  mo AS Money
  by AS Byte
  mode AS Mode
  inner AS Inner
  xs AS List OF Integer
  m AS Map OF String TO Integer
  st AS Set OF String
  col AS color::Color
  at AS AttributedString
END TYPE

TYPE Duo OF A, B
  first AS A
  second AS B
END TYPE

TYPE Sq
  side AS Integer
  tag AS String
END TYPE

TYPE Ci
  r AS Integer
END TYPE

UNION Shape
  Sq
  Ci
END UNION

FUNC show(c AS Ck) AS String
  RETURN c.name & \"|\" & c.value & \"|\" & toString(c.port) & \"|\" & toString(c.secure) & \"|\" & toString(c.ratio) & \"|\" & toString(c.fx) & \"|\" & toString(c.mo) & \"|\" & toString(c.by) & \"|\" & toString(c.mode) & \"|\" & toString(c.inner.n) & c.inner.s & \"|\" & toString(len(c.xs)) & toString(len(c.m)) & toString(len(c.st))
END FUNC

FUNC main AS Integer
  io::print(show(Ck[name := \"a\", value := \"b\"]))
  io::print(show(Ck[value := \"b\", name := \"a\", port := 5, secure := TRUE, ratio := 2.5]))
  MUT d AS Ck
  io::print(show(d))
  io::print(show(Ck[]))
  LET w = Ck[name := \"w\"]
  io::print(toString(w.col.red) & toString(w.col.alpha))
  LET p AS Duo OF Integer, String = Duo[first := 7]
  io::print(toString(p.first) & \"[\" & p.second & \"]\")
  LET c = color::Color[red := toByte(9)]
  io::print(toString(c.red) & toString(c.green) & toString(c.alpha))
  LET s AS Shape = Sq[side := 3]
  MATCH s
    CASE Sq(q)
      io::print(\"sq \" & toString(q.side) & \"[\" & q.tag & \"]\")
    CASE Ci(z)
      io::print(\"ci\")
  END MATCH
  LET l = [Sq[side := 1], Sq[tag := \"t\"]]
  io::print(toString(collections::get(l, 1).side) & collections::get(l, 1).tag)
  MUT i AS Integer = 0
  WHILE i < 3
    LET q = Sq[tag := \"x\"]
    io::print(q.tag & toString(q.side))
    i = i + 1
  END WHILE
  RETURN 0
END FUNC
";

const EXPECTED: [&str; 12] = [
    "a|b|0|FALSE|0.00|0.00|0.00|0|Slow|0|000",
    "a|b|5|TRUE|2.50|0.00|0.00|0|Slow|0|000",
    "||0|FALSE|0.00|0.00|0.00|0|Slow|0|000",
    "||0|FALSE|0.00|0.00|0.00|0|Slow|0|000",
    "00",
    "7[]",
    "900",
    "sq 3[]",
    "0t",
    "x0",
    "x0",
    "x0",
];

#[test]
fn omitted_fields_take_their_defaults_at_o0_and_o3() {
    let project = temp_project("ctor_partial_fields", RUN);
    for level in ["0", "3"] {
        let exe = if level == "0" {
            build_project(&project)
        } else {
            build_project_opt(&project, level)
        };
        let out = run_with_stdin(&exe, b"");
        assert_eq!(out.lines().collect::<Vec<_>>(), EXPECTED, "-O{level}");
    }
}

fn diagnostics(name: &str, source: &str) -> String {
    let project = temp_project(name, source);
    let output = Command::new(mfb_exe())
        .arg("build")
        .arg(&project)
        .output()
        .expect("run mfb build");
    assert!(!output.status.success(), "{name} must be rejected");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

const REJECT: &str = "IMPORT io
IMPORT fs

TYPE Held
  label AS String
  f AS RES fs::File
END TYPE

TYPE Ck
  name AS String
  port AS Integer
END TYPE

FUNC main AS Integer
  LET a = Held[label := \"x\"]
  LET b = Ck[name := \"a\", bogus := 2]
  LET c = Ck[\"a\"]
  RETURN 0
END FUNC
";

#[test]
fn undefaultable_unknown_and_positional_short_lists_are_still_refused() {
    let out = diagnostics("ctor_partial_reject", REJECT);
    assert!(
        out.contains(
            "Constructor `Held` has 1 argument(s), expected 2. Field(s) `f` have no default value"
        ),
        "omitted field with no default must be named:\n{out}"
    );
    assert!(
        out.contains("Constructor `Ck` has 1 argument(s), expected 2.\n")
            || out
                .matches("Constructor `Ck` has 1 argument(s), expected 2.")
                .count()
                >= 2,
        "unknown field name and positional short list keep the arity error:\n{out}"
    );
}
