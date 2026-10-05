//! plan-165-A: handler failure routing, lexical `PROPAGATE` binding, and an
//! `Error` returned as successful data (`mfb spec language error-model` §8.5,
//! §8.6 rules 3–4).
//!
//! An inline `TRAP` protects its expression, not its handler body. A failure in
//! the handler — a logging call, the `RECOVER` operand, the `RETURN` operand —
//! is a new error that routes outward: to the function-level handler when there
//! is one, otherwise to the caller. A handler may contain a nested inline `TRAP`
//! over a new expression; `PROPAGATE` re-raises the error of the innermost
//! lexical handler, and after a nested recovery it re-raises the outer one. A
//! function-level handler never catches its own failures.
//!
//! Each row of that matrix is built twice — once with no function-level handler
//! (the caller's inline `TRAP` sees the error) and once with one (`fh …`) — and
//! the error's code, message and origin line are compared. The memory test runs
//! the nested-recovery and `PROPAGATE` rows over `String`, `List` and record
//! payloads at `-O0`, `-O1` and `-O3` (bug-778 moves a block payload on read)
//! and requires a balanced arena with nothing freed twice. `RETURN` with an
//! `Error` value is success data in an `AS Error` function and a
//! `TYPE_RETURN_MISMATCH` anywhere the declared type is not `Error`.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use common::debug_report::{arena_lines, counter};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Fallible helpers whose errors carry distinct messages, so a replaced error is
/// told apart from the original by more than its code, and `describe`, which
/// renders `code:message@line` for an error.
const HELPERS: &str = r#"IMPORT io
IMPORT collections

FUNC broken(code AS Integer) AS Integer
  FAIL error(code, "broken")
END FUNC

FUNC again(code AS Integer) AS Integer
  FAIL error(code, "again")
END FUNC

FUNC logFails(code AS Integer) AS Nothing
  FAIL error(code, "log")
END FUNC

FUNC describe(e AS Error) AS String
  RETURN toString(e.code) & ":" & e.message & "@" & toString(e.source.line)
END FUNC

"#;

/// The 1-based line of `HELPERS`/a source whose text contains `needle`: an
/// error's origin is the line of the `error(...)` that built it.
fn line_of(source: &str, needle: &str) -> usize {
    source
        .lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no `{needle}` in source"))
        + 1
}

/// One routing row: its name, the inline-`TRAP` statement binding `x` (an
/// `Integer`), and what the error that leaves the function is — `None` when the
/// row recovers and the function returns normally.
struct Row {
    name: &'static str,
    body: &'static str,
    /// `(code, helper message)` of the escaping error; the helper's `FAIL` line is
    /// its origin.
    escapes: Option<(u32, &'static str)>,
    /// The value printed when the row recovers.
    recovered: &'static str,
}

const ROWS: &[Row] = &[
    // A logging call that fails inside the inline handler.
    Row {
        name: "log",
        body: r#"  LET x = broken(90000001) TRAP(e)
    io::print("log handler ran")
    logFails(90000011)
    io::print("log after the failing call")
    RECOVER 0
  END TRAP
"#,
        escapes: Some((90000011, "log")),
        recovered: "",
    },
    // The RECOVER operand fails: no recovery.
    Row {
        name: "recover",
        body: r#"  LET x = broken(90000001) TRAP(e)
    RECOVER again(90000012)
  END TRAP
"#,
        escapes: Some((90000012, "again")),
        recovered: "",
    },
    // The RETURN operand fails: no return.
    Row {
        name: "return",
        body: r#"  LET x = broken(90000001) TRAP(e)
    RETURN "returned " & toString(again(90000013))
  END TRAP
"#,
        escapes: Some((90000013, "again")),
        recovered: "",
    },
    // A nested inline expression fails: the nested handler runs.
    Row {
        name: "nested",
        body: r#"  LET x = broken(90000001) TRAP(e)
    LET y = again(90000014) TRAP(inner)
      RECOVER inner.code
    END TRAP
    RECOVER y + 1
  END TRAP
"#,
        escapes: None,
        recovered: "90000015",
    },
    // PROPAGATE in a nested named handler re-raises the nested error.
    Row {
        name: "nested_propagate",
        body: r#"  LET x = broken(90000001) TRAP(e)
    LET y = again(90000015) TRAP(inner)
      PROPAGATE
    END TRAP
    RECOVER y
  END TRAP
"#,
        escapes: Some((90000015, "again")),
        recovered: "",
    },
    // The same through a bare nested handler.
    Row {
        name: "nested_propagate_bare",
        body: r#"  LET x = broken(90000001) TRAP(e)
    LET y = again(90000016) TRAP
      PROPAGATE
    END TRAP
    RECOVER y
  END TRAP
"#,
        escapes: Some((90000016, "again")),
        recovered: "",
    },
    // PROPAGATE after the nested handler recovered re-raises the outer error.
    Row {
        name: "after_recovery",
        body: r#"  LET x = broken(90000002) TRAP(e)
    LET y = again(90000017) TRAP(inner)
      RECOVER 0
    END TRAP
    io::print("after_recovery nested value " & toString(y))
    PROPAGATE
  END TRAP
"#,
        escapes: Some((90000002, "broken")),
        recovered: "",
    },
    // The same with a bare outer handler.
    Row {
        name: "after_recovery_bare",
        body: r#"  LET x = broken(90000003) TRAP
    LET y = again(90000018) TRAP(inner)
      RECOVER inner.code
    END TRAP
    io::print("after_recovery_bare nested value " & toString(y))
    PROPAGATE
  END TRAP
"#,
        escapes: Some((90000003, "broken")),
        recovered: "",
    },
];

/// The routing program: each row without (`_nf`) and with (`_wf`) a
/// function-level handler, each called through the caller's inline `TRAP`.
/// Plus the function-handler rows: its body failing, `FAIL`ing and
/// `PROPAGATE`ing all reach the caller.
fn routing_source() -> String {
    let mut out = String::from(HELPERS);
    for row in ROWS {
        let name = row.name;
        let body = row.body;
        out.push_str(&format!(
            "FUNC {name}_nf() AS String\n{body}  io::print(\"{name} continued\")\n  RETURN \"ok \" & toString(x)\nEND FUNC\n\n\
             FUNC {name}_wf() AS String\n{body}  io::print(\"{name} continued\")\n  RETURN \"ok \" & toString(x)\n  TRAP(f)\n    RETURN \"fh \" & describe(f)\n  END TRAP\nEND FUNC\n\n"
        ));
    }
    out.push_str(
        r#"FUNC handler_fails() AS String
  LET x = broken(90000004)
  RETURN "ok " & toString(x)
  TRAP(f)
    io::print("handler_fails handler ran")
    logFails(90000019)
    io::print("handler_fails after the failing call")
    RETURN "fh"
  END TRAP
END FUNC

FUNC handler_fail_stmt() AS String
  LET x = broken(90000005)
  RETURN "ok " & toString(x)
  TRAP(f)
    FAIL error(90000020, "from the handler")
  END TRAP
END FUNC

FUNC handler_propagates() AS String
  LET x = broken(90000006)
  RETURN "ok " & toString(x)
  TRAP
    PROPAGATE
  END TRAP
END FUNC

FUNC main() AS Integer
"#,
    );
    let calls = ROWS
        .iter()
        .flat_map(|row| [format!("{}_nf", row.name), format!("{}_wf", row.name)])
        .chain(
            ["handler_fails", "handler_fail_stmt", "handler_propagates"]
                .iter()
                .map(|s| s.to_string()),
        );
    for call in calls {
        out.push_str(&format!(
            "  LET r_{call} = {call}() TRAP(e)\n    RECOVER \"caller \" & describe(e)\n  END TRAP\n  io::print(\"{call} \" & r_{call})\n"
        ));
    }
    out.push_str("  RETURN 0\nEND FUNC\n");
    out
}

/// What `routing_source` must print, derived from the matrix.
fn routing_expected(source: &str) -> String {
    let origin = |message: &str| match message {
        "broken" => line_of(source, "FAIL error(code, \"broken\")"),
        "again" => line_of(source, "FAIL error(code, \"again\")"),
        "log" => line_of(source, "FAIL error(code, \"log\")"),
        other => panic!("unknown helper message {other}"),
    };
    let mut out = String::new();
    for row in ROWS {
        let name = row.name;
        for form in ["nf", "wf"] {
            match name {
                "log" => out.push_str("log handler ran\n"),
                "after_recovery" => out.push_str("after_recovery nested value 0\n"),
                "after_recovery_bare" => {
                    out.push_str("after_recovery_bare nested value 90000018\n")
                }
                _ => {}
            }
            let result = match row.escapes {
                None => {
                    out.push_str(&format!("{name} continued\n"));
                    format!("ok {}", row.recovered)
                }
                Some((code, message)) => {
                    let described = format!("{code}:{message}@{}", origin(message));
                    if form == "nf" {
                        format!("caller {described}")
                    } else {
                        format!("fh {described}")
                    }
                }
            };
            out.push_str(&format!("{name}_{form} {result}\n"));
        }
    }
    out.push_str("handler_fails handler ran\n");
    out.push_str(&format!(
        "handler_fails caller 90000019:log@{}\n",
        origin("log")
    ));
    out.push_str(&format!(
        "handler_fail_stmt caller 90000020:from the handler@{}\n",
        line_of(source, "FAIL error(90000020, \"from the handler\")")
    ));
    out.push_str(&format!(
        "handler_propagates caller 90000006:broken@{}\n",
        origin("broken")
    ));
    out
}

fn project_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("mfb_p165a_{name}_{}", common::unique_nonce()));
    std::fs::create_dir_all(root.join("src")).expect("create project dir");
    std::fs::write(
        root.join("project.json"),
        "{\"name\":\"p165a\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write manifest");
    root
}

/// Run `mfb build <args> <project>` over `source`.
fn build_output(name: &str, source: &str, args: &[&str]) -> Output {
    let root = project_root(name);
    std::fs::write(root.join("src/main.mfb"), source).expect("write source");
    Command::new(common::mfb_exe())
        .arg("build")
        .args(args)
        .arg(&root)
        .output()
        .expect("run mfb build")
}

/// Build `source` with `args`, requiring success, and return the executable.
fn build(name: &str, source: &str, args: &[&str]) -> PathBuf {
    let output = build_output(name, source, args);
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
    PathBuf::from(chosen.trim())
}

/// Run `exe`, require a clean exit, and return `(stdout, stderr)`.
fn run(case: &str, exe: &Path) -> (String, String) {
    let output = Command::new(exe).output().expect("run the program");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{case} exited {:?}:\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );
    (stdout, stderr)
}

/// Every route of the matrix, with and without a function-level handler: a
/// failure in an inline handler is a new error that replaces the caught one and
/// leaves the function, `PROPAGATE` re-raises the innermost lexical handler's
/// error with its code, message and origin intact, and a function-level
/// handler's own failure reaches the caller.
#[test]
fn every_handler_route_reaches_the_documented_handler() {
    let source = routing_source();
    let expected = routing_expected(&source);
    for level in [0u8, 3] {
        let exe = build(
            &format!("routing_o{level}"),
            &source,
            &["-O", &level.to_string()],
        );
        let (stdout, _) = run(&format!("routing -O{level}"), &exe);
        assert_eq!(stdout, expected, "routing -O{level}");
    }
}

/// `RETURN` of an `Error` value is success: the caller receives it as data and no
/// handler runs, whether the value comes from `error(...)`, an inline handler's
/// caught error or a function-level handler's. `FAIL` with the same kind of value
/// is the failure.
#[test]
fn an_error_returned_from_an_error_function_is_successful_data() {
    let source = format!(
        "{HELPERS}{}",
        r#"FUNC makeError() AS Error
  RETURN error(90000030, "data")
END FUNC

FUNC failError() AS Error
  FAIL error(90000031, "failed")
END FUNC

FUNC fromInline() AS Error
  LET x = broken(90000032) TRAP(e)
    RETURN e
  END TRAP
  RETURN error(0, "unreached")
END FUNC

FUNC fromFunctionHandler() AS Error
  LET x = broken(90000033)
  RETURN error(0, "unreached")
  TRAP(e)
    RETURN e
  END TRAP
END FUNC

FUNC recoveredError() AS Error
  LET d = failError() TRAP(e)
    RECOVER e
  END TRAP
  RETURN d
END FUNC

FUNC main() AS Integer
  LET a = makeError() TRAP(e)
    io::print("makeError failed " & describe(e))
    RECOVER error(0, "x")
  END TRAP
  io::print("makeError data " & describe(a))
  LET b = failError() TRAP(e)
    io::print("failError failed " & describe(e))
    RECOVER error(0, "x")
  END TRAP
  io::print("failError data " & describe(b))
  LET c = fromInline() TRAP(e)
    io::print("fromInline failed " & describe(e))
    RECOVER error(0, "x")
  END TRAP
  io::print("fromInline data " & describe(c))
  LET d = fromFunctionHandler() TRAP(e)
    io::print("fromFunctionHandler failed " & describe(e))
    RECOVER error(0, "x")
  END TRAP
  io::print("fromFunctionHandler data " & describe(d))
  LET g = recoveredError() TRAP(e)
    io::print("recoveredError failed " & describe(e))
    RECOVER error(0, "x")
  END TRAP
  io::print("recoveredError data " & describe(g))
  RETURN 0
END FUNC
"#
    );
    let data_line = line_of(&source, "RETURN error(90000030, \"data\")");
    let fail_line = line_of(&source, "FAIL error(90000031, \"failed\")");
    let broken_line = line_of(&source, "FAIL error(code, \"broken\")");
    // `failError`'s caller recovers through the second `RECOVER error(0, "x")`.
    let x_line = source
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("RECOVER error(0, \"x\")"))
        .map(|(index, _)| index + 1)
        .nth(1)
        .expect("main's second RECOVER");
    let expected = format!(
        "makeError data 90000030:data@{data_line}\n\
         failError failed 90000031:failed@{fail_line}\n\
         failError data 0:x@{x_line}\n\
         fromInline data 90000032:broken@{broken_line}\n\
         fromFunctionHandler data 90000033:broken@{broken_line}\n\
         recoveredError data 90000031:failed@{fail_line}\n"
    );
    let exe = build("error_data", &source, &[]);
    let (stdout, _) = run("error data", &exe);
    assert_eq!(stdout, expected);
}

/// `RETURN` requires a value assignable to the declared success type: an `Error`
/// is not an `Integer` or `Nothing`, so returning one is a type error, not an
/// implicit failure — from the body, an inline handler or a function handler.
#[test]
fn returning_an_error_from_a_non_error_function_is_a_type_mismatch() {
    let cases = [
        (
            "int_body",
            "FUNC f() AS Integer\n  RETURN error(90000040, \"x\")\nEND FUNC\n",
        ),
        (
            "nothing_body",
            "FUNC f() AS Nothing\n  RETURN error(90000041, \"x\")\nEND FUNC\n",
        ),
        (
            "int_inline_handler",
            "FUNC f() AS Integer\n  LET x = broken(1) TRAP(e)\n    RETURN e\n  END TRAP\n  RETURN x\nEND FUNC\n",
        ),
        (
            "nothing_inline_handler",
            "FUNC f() AS Nothing\n  LET x = broken(1) TRAP(e)\n    RETURN e\n  END TRAP\n  io::print(toString(x))\nEND FUNC\n",
        ),
        (
            "int_function_handler",
            "FUNC f() AS Integer\n  RETURN broken(1)\n  TRAP(e)\n    RETURN e\n  END TRAP\nEND FUNC\n",
        ),
        (
            "nothing_function_handler",
            "FUNC f() AS Nothing\n  io::print(toString(broken(1)))\n  RETURN\n  TRAP(e)\n    RETURN e\n  END TRAP\nEND FUNC\n",
        ),
    ];
    for (name, func) in cases {
        let source =
            format!("{HELPERS}{func}\nFUNC main() AS Integer\n  f()\n  RETURN 0\nEND FUNC\n");
        let output = build_output(&format!("reject_{name}"), &source, &[]);
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !output.status.success(),
            "{name}: returning an Error must not compile:\n{combined}"
        );
        assert!(
            combined.contains("TYPE_RETURN_MISMATCH"),
            "{name}: expected TYPE_RETURN_MISMATCH, got:\n{combined}"
        );
    }
}

/// One payload class for the memory test: its type, a fallible producer that
/// fails on odd `k` with a message naming `k`, the value a handler recovers
/// from a message `MSG`, how a recovered value `y` absorbs a second message
/// `MSG`, and how the caller renders a value `v`.
struct Payload {
    name: &'static str,
    type_: &'static str,
    ok: &'static str,
    err_prefix: &'static str,
    from_message: &'static str,
    combine: &'static str,
    digest: &'static str,
}

const PAYLOADS: &[Payload] = &[
    Payload {
        name: "str",
        type_: "String",
        ok: "\"v\" & toString(k)",
        err_prefix: "s",
        from_message: "\"r\" & MSG",
        combine: "y & MSG",
        digest: "v",
    },
    Payload {
        name: "list",
        type_: "List OF String",
        ok: "[\"v\" & toString(k), \"w\"]",
        err_prefix: "l",
        from_message: "[\"r\" & MSG]",
        combine: "collections::append(y, MSG)",
        digest: "joinList(v)",
    },
    Payload {
        name: "duo",
        type_: "Duo",
        ok: "Duo[\"v\" & toString(k), k]",
        err_prefix: "d",
        from_message: "Duo[\"r\" & MSG, 0]",
        combine: "Duo[y.name & MSG, y.n + 1]",
        digest: "v.name & \"#\" & toString(v.n)",
    },
];

/// The three nested shapes per payload: recovery inside recovery, `PROPAGATE`
/// from the nested handler, and `PROPAGATE` of the outer error after the nested
/// handler recovered. The nested producer reads `i / 2`, so with `i` odd the
/// nested call alternates between success and failure.
fn memory_source() -> String {
    let mut out = String::from(
        r#"IMPORT io
IMPORT collections

TYPE Duo
  name AS String
  n AS Integer
END TYPE

FUNC joinList(l AS List OF String) AS String
  MUT s AS String = ""
  FOR EACH item IN l
    s = s & item & "+"
  NEXT
  RETURN s
END FUNC

"#,
    );
    for p in PAYLOADS {
        let Payload {
            name,
            type_,
            ok,
            err_prefix,
            from_message,
            combine,
            digest,
        } = p;
        let recover_inner = from_message.replace("MSG", "inner.message");
        let recover_outer = combine.replace("MSG", "e.message");
        out.push_str(&format!(
            r#"FUNC maybe_{name}(k AS Integer) AS {type_}
  IF k MOD 2 = 1 THEN
    FAIL error(90000100, "{err_prefix}" & toString(k))
  END IF
  RETURN {ok}
END FUNC

FUNC digest_{name}(v AS {type_}) AS String
  RETURN {digest}
END FUNC

FUNC rec_{name}(i AS Integer) AS {type_}
  LET x = maybe_{name}(i) TRAP(e)
    LET y = maybe_{name}(i / 2) TRAP(inner)
      RECOVER {recover_inner}
    END TRAP
    RECOVER {recover_outer}
  END TRAP
  RETURN x
END FUNC

FUNC prop_{name}(i AS Integer) AS {type_}
  LET x = maybe_{name}(i) TRAP(e)
    LET y = maybe_{name}(i / 2) TRAP(inner)
      PROPAGATE
    END TRAP
    RECOVER {recover_outer}
  END TRAP
  RETURN x
END FUNC

FUNC after_{name}(i AS Integer) AS {type_}
  LET x = maybe_{name}(i) TRAP(e)
    LET y = maybe_{name}(i / 2) TRAP(inner)
      RECOVER {recover_inner}
    END TRAP
    IF digest_{name}(y) = "never" THEN
      RECOVER y
    END IF
    PROPAGATE
  END TRAP
  RETURN x
END FUNC

"#
        ));
    }
    out.push_str("FUNC main() AS Integer\n  FOR i = 1 TO 40\n");
    for p in PAYLOADS {
        for shape in ["rec", "prop", "after"] {
            let name = p.name;
            out.push_str(&format!(
                "    LET v_{shape}_{name} = digest_{name}({shape}_{name}(i)) TRAP(e)\n      RECOVER \"c\" & e.message\n    END TRAP\n    io::print(\"{shape}_{name} \" & toString(i) & \" \" & v_{shape}_{name})\n"
            ));
        }
    }
    out.push_str("  NEXT\n  RETURN 0\nEND FUNC\n");
    out
}

/// A payload value in the Rust mirror: what `digest_<payload>` prints.
#[derive(Clone)]
enum Value {
    Str(String),
    List(Vec<String>),
    Duo(String, i64),
}

impl Value {
    fn digest(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            Value::List(items) => items.iter().map(|item| format!("{item}+")).collect(),
            Value::Duo(name, n) => format!("{name}#{n}"),
        }
    }
}

/// What `memory_source` must print, by mirroring each shape.
fn memory_expected() -> String {
    let maybe = |p: &Payload, k: i64| -> Result<Value, String> {
        if k % 2 == 1 {
            return Err(format!("{}{k}", p.err_prefix));
        }
        Ok(match p.name {
            "str" => Value::Str(format!("v{k}")),
            "list" => Value::List(vec![format!("v{k}"), "w".to_string()]),
            _ => Value::Duo(format!("v{k}"), k),
        })
    };
    let from_message = |p: &Payload, m: &str| match p.name {
        "str" => Value::Str(format!("r{m}")),
        "list" => Value::List(vec![format!("r{m}")]),
        _ => Value::Duo(format!("r{m}"), 0),
    };
    let combine = |y: Value, m: &str| match y {
        Value::Str(s) => Value::Str(format!("{s}{m}")),
        Value::List(mut items) => {
            items.push(m.to_string());
            Value::List(items)
        }
        Value::Duo(name, n) => Value::Duo(format!("{name}{m}"), n + 1),
    };
    let mut out = String::new();
    for i in 1..=40i64 {
        for p in PAYLOADS {
            let outcomes: [(&str, Result<Value, String>); 3] = [
                (
                    "rec",
                    maybe(p, i).or_else(|e| {
                        let y = maybe(p, i / 2).unwrap_or_else(|inner| from_message(p, &inner));
                        Ok(combine(y, &e))
                    }),
                ),
                (
                    "prop",
                    maybe(p, i).or_else(|e| maybe(p, i / 2).map(|y| combine(y, &e))),
                ),
                ("after", maybe(p, i).or_else(Err)),
            ];
            for (shape, outcome) in outcomes {
                let shown = match outcome {
                    Ok(value) => value.digest(),
                    Err(message) => format!("c{message}"),
                };
                out.push_str(&format!("{shape}_{} {i} {shown}\n", p.name));
            }
        }
    }
    out
}

/// Nested recovery and `PROPAGATE` over block payloads move each value exactly
/// once: the results match the mirror, every block and `Error` is freed, and
/// nothing is freed twice, at every optimization level the bug-778 slots differ
/// across.
#[test]
fn nested_recovery_and_propagate_balance_the_arena() {
    let source = memory_source();
    let expected = memory_expected();
    for level in [0u8, 1, 3] {
        let case = format!("memory -O{level}");
        let exe = build(
            &format!("memory_o{level}"),
            &source,
            &["--debug", "-O", &level.to_string()],
        );
        let (stdout, stderr) = run(&case, &exe);
        assert_eq!(stdout, expected, "{case}");
        let lines = arena_lines(&case, &stderr);
        assert_eq!(
            counter(&case, &lines, 0, "live_bytes"),
            0,
            "{case}: every payload and Error is freed:\n{stderr}"
        );
        assert_eq!(
            counter(&case, &lines, 0, "double_free_skips"),
            0,
            "{case}: nothing is freed twice:\n{stderr}"
        );
    }
}
