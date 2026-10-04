//! bug-778: the success path of an inline-`TRAP`'d expression allocates nothing
//! the same expression does not allocate without the `TRAP`.
//!
//! `LET x = f(a) TRAP(e) … END TRAP` desugars (`mfb spec language error-model`
//! §8.8) to a `Result` temporary per fallible node (`$trap_resN = CallResult(..)`
//! / `Checked(..)`) read by `ResultIsOk` / `ResultValue` / `ResultError`, plus a
//! delivery slot `$trap_valN`. Native code used to give every such temporary a
//! heap `{tag, size, payload}` block on every evaluation — a successful one
//! included — copy a block payload into it and back out, build a default for the
//! delivery slot, and copy the delivery slot into the binding at `-O0`: 1–4 extra
//! arena allocations per evaluation, at every `-O` level.
//!
//! The success assertions compare `--debug` `arena.0.alloc_calls` between a
//! trapped program and its plain twin, each run at N and 2N evaluations, so the
//! per-evaluation DELTA is what is compared and start-up allocations cancel. The
//! error assertions are the guard on the other side: every failing shape still
//! delivers the error an untrapped evaluation raises (checked against a
//! function-level `TRAP` oracle), alternating success and failure in one loop
//! still balances the arena, and nothing is freed twice.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use common::debug_report::{arena_lines, counter};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The shared declarations: fallible user functions of every payload class the
/// unboxed form takes (scalar, `Float`, `String`, flat record, flat list,
/// `Nothing`).
const HEAD: &str = r#"IMPORT io
IMPORT os
IMPORT collections

TYPE Duo
  name AS String
  n AS Integer
END TYPE

FUNC userInt(x AS Integer) AS Integer
  IF x < 0 THEN
    FAIL error(77050002, "neg")
  END IF
  RETURN x + 1
END FUNC

FUNC userStr(x AS Integer) AS String
  IF x < 0 THEN
    FAIL error(77050002, "neg")
  END IF
  RETURN "v"
END FUNC

FUNC userFl(x AS Integer) AS Float
  IF x < 0 THEN
    FAIL error(77050002, "neg")
  END IF
  RETURN toFloat(x)
END FUNC

FUNC userDuo(x AS Integer) AS Duo
  IF x < 0 THEN
    FAIL error(77050002, "neg")
  END IF
  RETURN Duo["p", x]
END FUNC

FUNC userList(x AS Integer) AS List OF Integer
  IF x < 0 THEN
    FAIL error(77050002, "neg")
  END IF
  RETURN [x, x]
END FUNC

FUNC userNothing(x AS Integer) AS Nothing
  IF x < 0 THEN
    FAIL error(77050002, "neg")
  END IF
  RETURN
END FUNC
"#;

/// One trapped shape: its name, the bound type, the trapped expression, a use
/// of the bound `v`, the assignment that makes it fail — or, for a shape with
/// no input to break, the failing expression the error program uses instead —
/// and a `RECOVER` value.
struct Shape {
    name: &'static str,
    type_: &'static str,
    expr: &'static str,
    use_: &'static str,
    fail: &'static str,
    failing_expr: Option<&'static str>,
    recover: &'static str,
}

/// The bug's table plus one shape per producer path and payload class: a
/// `Checked` chain under a conversion, a user callee (`Integer`, `String`,
/// record, list), a fallible parse, a constant argument, a raising root
/// operator, a `Float` observation boundary, an inline builtin with a `String`
/// result, and a call through a `FUNC` value.
const SHAPES: &[Shape] = &[
    Shape {
        name: "checked_chain",
        type_: "Scalar",
        expr: "toScalar(65 + (i MOD d))",
        use_: "toInt(v)",
        fail: "d = 0",
        failing_expr: None,
        recover: "toScalar(66)",
    },
    Shape {
        name: "user_int",
        type_: "Integer",
        expr: "userInt(x)",
        use_: "v",
        fail: "x = -1",
        failing_expr: None,
        recover: "0",
    },
    Shape {
        name: "user_str",
        type_: "String",
        expr: "userStr(x)",
        use_: "len(v)",
        fail: "x = -1",
        failing_expr: None,
        recover: "\"r\"",
    },
    Shape {
        name: "parse",
        type_: "Integer",
        expr: "toInt(t)",
        use_: "v",
        fail: "t = \"zz\"",
        failing_expr: None,
        recover: "0",
    },
    Shape {
        name: "user_const",
        type_: "Integer",
        expr: "userInt(5)",
        use_: "v",
        fail: "",
        failing_expr: Some("userInt(-5)"),
        recover: "0",
    },
    Shape {
        name: "root_operator",
        type_: "Integer",
        expr: "userInt(x) * 2 + 3",
        use_: "v",
        fail: "x = 4611686018427387904",
        failing_expr: None,
        recover: "0",
    },
    Shape {
        name: "float_root",
        type_: "Float",
        expr: "userFl(x) * f",
        use_: "toInt(v)",
        fail: "f = 1.0e308",
        failing_expr: None,
        recover: "0.0",
    },
    Shape {
        name: "builtin_str",
        type_: "String",
        expr: "collections::get(names, k)",
        use_: "len(v)",
        fail: "k = 5",
        failing_expr: None,
        recover: "\"r\"",
    },
    Shape {
        name: "user_record",
        type_: "Duo",
        expr: "userDuo(x)",
        use_: "v.n",
        fail: "x = -1",
        failing_expr: None,
        recover: "Duo[\"r\", 0]",
    },
    Shape {
        name: "user_list",
        type_: "List OF Integer",
        expr: "userList(x)",
        use_: "len(v)",
        fail: "x = -1",
        failing_expr: None,
        recover: "[0]",
    },
    Shape {
        name: "func_value",
        type_: "Integer",
        expr: "g(x)",
        use_: "v",
        fail: "x = -1",
        failing_expr: None,
        recover: "0",
    },
];

/// The success inputs every shape reads, as locals; `bad` switches the shape's
/// own `fail` assignment on.
fn inputs(shape: &Shape, indent: &str) -> String {
    let mut lines = vec![
        "MUT x AS Integer = 7".to_string(),
        "MUT d AS Integer = 26".to_string(),
        "MUT t AS String = \"123\"".to_string(),
        "MUT f AS Float = 1.5".to_string(),
        "MUT k AS Integer = 1".to_string(),
        "LET names AS List OF String = [\"aa\", \"bbb\"]".to_string(),
        "LET g AS FUNC(Integer) AS Integer = userInt".to_string(),
    ];
    if !shape.fail.is_empty() {
        lines.push(format!(
            "IF bad THEN\n{indent}  {}\n{indent}END IF",
            shape.fail
        ));
    }
    lines
        .iter()
        .map(|line| format!("{indent}{line}\n"))
        .collect()
}

/// The success program: `which n` runs one shape's loop `n` times, trapped or
/// plain. The handler never runs (every input succeeds).
fn success_source(trapped: bool) -> String {
    let mut out = String::from(HEAD);
    for shape in SHAPES {
        let statement = if trapped {
            format!(
                "    LET v AS {} = {} TRAP(err)\n      RETURN -1\n    END TRAP\n",
                shape.type_, shape.expr
            )
        } else {
            format!("    LET v AS {} = {}\n", shape.type_, shape.expr)
        };
        out.push_str(&format!(
            "FUNC s_{name}(n AS Integer) AS Integer\n  LET bad AS Boolean = FALSE\n{inputs}  MUT s AS Integer = 0\n  FOR i = 1 TO n\n{statement}    s = s + {use_}\n  NEXT\n  RETURN s\nEND FUNC\n\n",
            name = shape.name,
            inputs = inputs(shape, "  "),
            use_ = shape.use_,
        ));
    }
    // The discarded form: no `ResultValue` at all, so a block payload is still
    // the temporary's to free; and a `Nothing` payload.
    let discard = if trapped {
        "    userInt(i) TRAP(err)\n      RETURN -1\n    END TRAP\n    userStr(i) TRAP(err)\n      RETURN -1\n    END TRAP\n    userNothing(i) TRAP(err)\n      RETURN -1\n    END TRAP\n"
    } else {
        "    userInt(i)\n    userStr(i)\n    userNothing(i)\n"
    };
    out.push_str(&format!(
        "FUNC s_discard(n AS Integer) AS Integer\n  FOR i = 1 TO n\n{discard}  NEXT\n  RETURN n\nEND FUNC\n\n"
    ));
    out.push_str(
        "FUNC main AS Integer\n  LET a AS List OF String = os::args()\n  LET which AS String = collections::get(a, 0)\n  LET n AS Integer = toInt(collections::get(a, 1))\n  MUT r AS Integer = 0\n",
    );
    for name in SHAPES.iter().map(|s| s.name).chain(["discard"]) {
        out.push_str(&format!(
            "  IF which = \"{name}\" THEN\n    r = s_{name}(n)\n  END IF\n"
        ));
    }
    out.push_str("  io::print(toString(r))\n  RETURN 0\nEND FUNC\n");
    out
}

/// The error program. Per shape: the inline-`TRAP`'d failure next to the same
/// failure under a function-level `TRAP` (the oracle — it never builds a trapped
/// `Result`), the success path next to it, and a loop alternating success and
/// failure with one handler that reads `e` and one that does not.
fn error_source() -> String {
    let mut out = String::from(HEAD);
    for shape in SHAPES {
        let Shape {
            name,
            type_,
            use_,
            recover,
            ..
        } = shape;
        let expr = shape.failing_expr.unwrap_or(shape.expr);
        let quiet_use = if *use_ == "v" {
            "w".to_string()
        } else {
            use_.replace("(v)", "(w)").replace("v.", "w.")
        };
        out.push_str(&format!(
            r#"FUNC t_{name}(bad AS Boolean, i AS Integer) AS String
{inputs}  LET v AS {type_} = {expr} TRAP(e)
    RETURN "code=" & toString(e.code) & " msg=" & e.message
  END TRAP
  RETURN "ok " & toString({use_})
END FUNC

FUNC o_{name}(bad AS Boolean, i AS Integer) AS String
{inputs}  LET v AS {type_} = {expr}
  RETURN "ok " & toString({use_})
TRAP(e)
  RETURN "code=" & toString(e.code) & " msg=" & e.message
END TRAP
END FUNC

FUNC oc_{name}() AS Integer
  LET bad AS Boolean = TRUE
  LET i AS Integer = 1
{inputs}  LET v AS {type_} = {expr}
  RETURN 0
TRAP(e)
  RETURN e.code
END TRAP
END FUNC

FUNC r_{name}(count AS Integer, oracle AS Integer) AS String
  MUT handled AS Integer = 0
  MUT agreed AS Integer = 0
  MUT quiet AS Integer = 0
  MUT used AS Integer = 0
  FOR i = 1 TO count
    LET bad AS Boolean = i MOD 2 = 1
{loop_inputs}    LET v AS {type_} = {expr} TRAP(e)
      handled = handled + 1
      IF e.code = oracle THEN
        agreed = agreed + 1
      END IF
      RECOVER {recover}
    END TRAP
    used = used + {use_}
    LET w AS {type_} = {expr} TRAP(e2)
      quiet = quiet + 1
      RECOVER {recover}
    END TRAP
    used = used + {quiet_use}
  NEXT
  RETURN "handled=" & toString(handled) & " agreed=" & toString(agreed) & " quiet=" & toString(quiet) & " used=" & toString(used)
END FUNC

"#,
            inputs = inputs(shape, "  "),
            loop_inputs = inputs(shape, "    "),
        ));
    }
    out.push_str("FUNC main AS Integer\n");
    for shape in SHAPES {
        let name = shape.name;
        out.push_str(&format!(
            "  io::print(\"{name} inline \" & t_{name}(TRUE, 1))\n  io::print(\"{name} oracle \" & o_{name}(TRUE, 1))\n  io::print(\"{name} okpath \" & t_{name}(FALSE, 2) & \" / \" & o_{name}(FALSE, 2))\n  io::print(\"{name} loop \" & r_{name}(200, oc_{name}()))\n"
        ));
    }
    out.push_str("  RETURN 0\nEND FUNC\n");
    out
}

/// Build `source` with `--debug` at `-O <level>` and return the executable.
fn build(name: &str, source: &str, level: u8) -> PathBuf {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .args(["build", "--debug", "-O", &level.to_string()])
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name} failed to build:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let chosen = written
        .iter()
        .find(|path| path.ends_with("-glibc.out"))
        .or_else(|| written.first())
        .unwrap_or_else(|| panic!("{name}: no executable in build output:\n{stdout}"));
    PathBuf::from(chosen)
}

/// Run `exe` with `args`; require success; return `(stdout, stderr)`.
fn run(case: &str, exe: &Path, args: &[&str]) -> (String, String) {
    let output = Command::new(exe)
        .args(args)
        .output()
        .expect("run the program");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{case} failed ({:?}):\n{stdout}\n{stderr}",
        output.status
    );
    (stdout, stderr)
}

/// `arena.0.alloc_calls` per 1000 evaluations of `shape`: the count at 2000
/// minus the count at 1000, with the program's printed result.
fn alloc_delta(exe: &Path, case: &str, shape: &str) -> (u64, String) {
    let count = |n: &str| {
        let (stdout, stderr) = run(case, exe, &[shape, n]);
        let lines = arena_lines(case, &stderr);
        (counter(case, &lines, 0, "alloc_calls"), stdout)
    };
    let (small, _) = count("1000");
    let (large, stdout) = count("2000");
    (large - small, stdout)
}

/// The success path of every shape allocates exactly what its plain twin does,
/// at every `-O` level, and computes the same value.
#[test]
fn the_trapped_success_path_allocates_what_the_plain_expression_does() {
    let trapped_source = success_source(true);
    let plain_source = success_source(false);
    for level in 0..=3u8 {
        let trapped = build(&format!("b778_ok_trap_o{level}"), &trapped_source, level);
        let plain = build(&format!("b778_ok_plain_o{level}"), &plain_source, level);
        for shape in SHAPES.iter().map(|s| s.name).chain(["discard"]) {
            let case = format!("{shape} -O{level}");
            let (trapped_delta, trapped_value) = alloc_delta(&trapped, &case, shape);
            let (plain_delta, plain_value) = alloc_delta(&plain, &case, shape);
            assert_eq!(
                trapped_value, plain_value,
                "{case}: the trapped and plain loops compute the same value"
            );
            assert_eq!(
                trapped_delta, plain_delta,
                "{case}: 1000 successful trapped evaluations made {trapped_delta} arena \
                 allocations where the plain expression made {plain_delta}"
            );
        }
    }
}

/// Every failing shape still reaches its handler with the error the untrapped
/// evaluation raises; the success path beside it returns the plain value; and a
/// loop alternating the two balances the arena with no double free.
#[test]
fn every_failing_shape_delivers_the_untrapped_error() {
    let source = error_source();
    for level in [0u8, 1, 3] {
        let exe = build(&format!("b778_err_o{level}"), &source, level);
        let case = format!("errors -O{level}");
        let (stdout, stderr) = run(&case, &exe, &[]);
        let line = |shape: &str, kind: &str| -> String {
            let prefix = format!("{shape} {kind} ");
            stdout
                .lines()
                .find_map(|line| line.strip_prefix(&prefix))
                .unwrap_or_else(|| panic!("{case}: no `{prefix}` line in:\n{stdout}"))
                .to_string()
        };
        for shape in SHAPES {
            let name = shape.name;
            let inline = line(name, "inline");
            assert_eq!(
                inline,
                line(name, "oracle"),
                "{case}: {name}'s inline handler must see the error a function-level TRAP sees"
            );
            assert!(
                inline.starts_with("code=7705"),
                "{case}: {name} must fail with a runtime error code, got `{inline}`"
            );
            let okpath = line(name, "okpath");
            let (trapped, untrapped) = okpath
                .split_once(" / ")
                .unwrap_or_else(|| panic!("{case}: malformed okpath `{okpath}`"));
            assert_eq!(trapped, untrapped, "{case}: {name}'s success value");
            // A shape with no input to break fails every iteration; every other
            // shape fails on the odd half.
            let failing = if shape.failing_expr.is_some() {
                200
            } else {
                100
            };
            assert!(
                line(name, "loop").starts_with(&format!(
                    "handled={failing} agreed={failing} quiet={failing} "
                )),
                "{case}: {name}'s alternating loop: `{}`",
                line(name, "loop")
            );
        }
        // The user-raised error keeps its code and message exactly.
        assert_eq!(line("user_str", "inline"), "code=77050002 msg=neg");
        assert_eq!(line("user_record", "inline"), "code=77050002 msg=neg");
        let lines = arena_lines(&case, &stderr);
        assert_eq!(
            counter(&case, &lines, 0, "live_bytes"),
            0,
            "{case}: every trapped value and Error is freed:\n{stderr}"
        );
        assert_eq!(
            counter(&case, &lines, 0, "double_free_skips"),
            0,
            "{case}: nothing is freed twice:\n{stderr}"
        );
    }
}
