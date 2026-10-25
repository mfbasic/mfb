//! plan-165-D (review 3.1, Option 1): an inline `TRAP` may protect any
//! well-typed expression at a binding, assignment or bare-expression position.
//!
//! The protected expression is the complete right-hand side with the language's
//! precedence; `RECOVER` supplies the whole value, never just the last
//! operator's. Trapping changes where a failure goes, never whether or when it
//! happens: an Integer zero divisor fails at once and skips the rest, a `Float`
//! is observed at its existing boundaries only (a transient non-finite that
//! becomes finite does not trap; `a * b - c` still contracts), a comparison that
//! promotes into `Fixed` raises at the comparison, and an expression that cannot
//! fail carries a provably dead handler flagged `TYPE_INLINE_TRAP_DEAD_HANDLER` —
//! `a & b TRAP(e)` included, because `&` is never fallible. Before plan-165-D an
//! expression with no runtime call was rejected with
//! `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` (bug-471's control, bug-779's fixture).

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use common::debug_report::{arena_lines, counter};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const HEAD: &str = r#"IMPORT io
IMPORT os
IMPORT collections

FUNC note(tag AS String, n AS Integer) AS Integer
  io::print("eval " & tag)
  RETURN n
END FUNC

FUNC cat(a AS String, b AS String) AS String
  LET s = a & b
  LET marker = "m"
  RETURN s & marker
  TRAP(e)
    RETURN marker
  END TRAP
END FUNC

"#;

fn build_output(name: &str, source: &str, args: &[&str]) -> Output {
    Command::new(common::mfb_exe())
        .arg("build")
        .args(args)
        .arg(common::temp_project(&format!("p165d_{name}"), source))
        .output()
        .expect("run mfb build")
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Build, requiring success, and return the executable and the build output.
fn build(name: &str, source: &str, args: &[&str]) -> (PathBuf, String) {
    let output = build_output(name, source, args);
    let text = combined(&output);
    assert!(output.status.success(), "{name} failed to build:\n{text}");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let chosen = written
        .iter()
        .find(|path| path.ends_with(common::host_executable_suffix()))
        .or_else(|| written.first())
        .unwrap_or_else(|| panic!("{name}: no executable in build output:\n{stdout}"));
    (PathBuf::from(chosen.trim()), text)
}

fn run(case: &str, exe: &Path, args: &[&str]) -> (String, String) {
    let output = Command::new(exe).args(args).output().expect("run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{case} exited {:?}:\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );
    (stdout, stderr)
}

/// Build `body` as `main` (with a function-level handler that would print
/// `fh …` for an escaped error) at -O0 and -O3; require `expected`. Returns the
/// -O0 build output so a case can look for warnings.
fn expect_main(name: &str, body: &str, expected: &str) -> String {
    let source = format!(
        "{HEAD}FUNC main() AS Integer\n{body}  RETURN 0\n  TRAP(f)\n    io::print(\"fh \" & toString(f.code))\n    RETURN 3\n  END TRAP\nEND FUNC\n"
    );
    let mut first = String::new();
    for level in ["0", "3"] {
        let (exe, text) = build(&format!("{name}_o{level}"), &source, &["-O", level]);
        if first.is_empty() {
            first = text;
        }
        let (stdout, _) = run(&format!("{name} -O{level}"), &exe, &[]);
        assert_eq!(stdout, expected, "{name} -O{level}");
    }
    first
}

/// `1 / 2 / 3 / 4`-style chains: a zero divisor in the first, middle or last
/// position enters the handler at once, and RECOVER replaces the whole result.
#[test]
fn a_chained_division_failure_anywhere_recovers_the_whole_result() {
    expect_main(
        "chain",
        r#"  MUT a = 1000
  MUT one = 1
  MUT zero = 0
  LET first = a / zero / one / one TRAP(e)
    io::print("first " & toString(e.code))
    RECOVER 99
  END TRAP
  LET middle = a / one / zero / one TRAP(e)
    io::print("middle " & toString(e.code))
    RECOVER 98
  END TRAP
  LET last = a / one / one / zero TRAP(e)
    io::print("last " & toString(e.code))
    RECOVER 97
  END TRAP
  LET fine = a / 10 / 5 / 2 TRAP(e)
    io::print("fine " & toString(e.code))
    RECOVER 96
  END TRAP
  LET grouped = (a + one) * (a / zero) TRAP(e)
    io::print("grouped " & toString(e.code))
    RECOVER 95
  END TRAP
  io::print(toString(first) & "," & toString(middle) & "," & toString(last) & "," & toString(fine) & "," & toString(grouped))
"#,
        "first 77050002\nmiddle 77050002\nlast 77050002\ngrouped 77050002\n99,98,97,10,95\n",
    );
}

/// Checked Integer, Byte, Fixed and Money operations, each with its own literal
/// spelling (bug-820: never a Float literal in a Money slot).
#[test]
fn checked_integer_byte_fixed_and_money_operations_are_captured() {
    expect_main(
        "checked",
        r#"  MUT big = 9223372036854775807
  MUT b AS Byte = toByte(200)
  MUT fx = 2000000000.5F
  MUT m = 12.50m
  MUT zero = 0
  LET i = big + 1 TRAP(e)
    io::print("integer " & toString(e.code))
    RECOVER -1
  END TRAP
  LET y = b + b TRAP(e)
    io::print("byte " & toString(e.code))
    RECOVER toByte(1)
  END TRAP
  LET x = fx * fx TRAP(e)
    io::print("fixed " & toString(e.code))
    RECOVER 0F
  END TRAP
  LET n = m / zero TRAP(e)
    io::print("money " & toString(e.code))
    RECOVER 0.00m
  END TRAP
  LET ok = m * 2 TRAP(e)
    io::print("money-ok " & toString(e.code))
    RECOVER 0.00m
  END TRAP
  io::print(toString(i) & "," & toString(y) & "," & toString(x = 0F) & "," & toString(n = 0.00m) & "," & toString(ok = 25.00m))
"#,
        "integer 77050010\nbyte 77050010\nfixed 77050010\nmoney 77050002\n-1,1,TRUE,TRUE,TRUE\n",
    );
}

/// The Float model: a final overflow is observed at the binding and caught; a
/// transient infinity that becomes finite is not; `x * y - z` gives the same
/// bits trapped and untrapped.
#[test]
fn float_observation_timing_is_unchanged_by_the_trap() {
    expect_main(
        "float",
        r#"  MUT big = 1.0e200
  MUT x = 0.1
  MUT y = 0.2
  MUT z = 0.3
  LET over = big * big TRAP(e)
    io::print("overflow " & toString(e.code))
    RECOVER -1.0
  END TRAP
  LET back = 1.0 / (big * big) TRAP(e)
    io::print("transient " & toString(e.code))
    RECOVER -1.0
  END TRAP
  LET plain = x * y - z
  LET fused = x * y - z TRAP(e)
    io::print("fma " & toString(e.code))
    RECOVER -1.0
  END TRAP
  io::print(toString(over = -1.0) & "," & toString(back = 0.0) & "," & toString(fused = plain))
"#,
        "overflow 77050015\nTRUE,TRUE,TRUE\n",
    );
}

/// Comparisons: promoting into `Fixed` raises at the comparison (out of range →
/// `ErrOverflow`, non-finite Float including a transient one →
/// `ErrInvalidFormat`); a non-finite `Float` compared with a `Float`, `Integer`
/// or `Byte` is not observed and does not trap.
#[test]
fn comparisons_raise_only_when_they_promote_into_fixed() {
    let warnings = expect_main(
        "compare",
        r#"  MUT fx = 1.5F
  MUT big = 3000000000
  MUT huge = 1.0e300
  MUT one = 1
  MUT b1 AS Byte = toByte(1)
  LET a = fx < big TRAP(e)
    io::print("range " & toString(e.code))
    RECOVER FALSE
  END TRAP
  LET b = big > fx TRAP(e)
    io::print("range-rev " & toString(e.code))
    RECOVER FALSE
  END TRAP
  LET c = fx < huge * huge TRAP(e)
    io::print("nonfinite " & toString(e.code))
    RECOVER FALSE
  END TRAP
  LET d = huge * huge > 1.0 TRAP(e)
    io::print("float " & toString(e.code))
    RECOVER FALSE
  END TRAP
  LET g = huge * huge > one TRAP(e)
    io::print("integer " & toString(e.code))
    RECOVER FALSE
  END TRAP
  LET h = huge * huge > b1 TRAP(e)
    io::print("byte " & toString(e.code))
    RECOVER FALSE
  END TRAP
  io::print(toString(a) & "," & toString(b) & "," & toString(c) & "," & toString(d) & "," & toString(g) & "," & toString(h))
"#,
        "range 77050010\nrange-rev 77050010\nnonfinite 77050003\nFALSE,FALSE,FALSE,TRUE,TRUE,TRUE\n",
    );
    let _ = warnings;
}

/// `&` is never fallible (owner decision 2026-10-04): `a & b TRAP(e)` builds with
/// the dead-handler warning and runs, as does any other expression that cannot
/// fail; a function containing only `&` stays infallible, so its handler may
/// read a local declared after it.
#[test]
fn a_known_total_expression_carries_a_dead_handler() {
    let text = expect_main(
        "total",
        r#"  MUT a = "ab"
  MUT b = "cd"
  MUT n = 3
  LET s = a & b TRAP(e)
    io::print("concat " & toString(e.code))
    RECOVER ""
  END TRAP
  LET same = n = 3 TRAP(e)
    io::print("compare " & toString(e.code))
    RECOVER FALSE
  END TRAP
  LET copy = n TRAP(e)
    io::print("copy " & toString(e.code))
    RECOVER 0
  END TRAP
  io::print(s & "," & toString(same) & "," & toString(copy) & "," & cat("x", "y"))
"#,
        "abcd,TRUE,3,xym\n",
    );
    assert_eq!(
        text.matches("TYPE_INLINE_TRAP_DEAD_HANDLER").count(),
        3,
        "each known-total trap warns once:\n{text}"
    );
}

/// Parenthesized expressions, assignment to an existing target (unchanged until
/// RECOVER), a conditional operator without any call, and the bare-statement
/// form; a failure skips the later effects.
#[test]
fn every_position_takes_the_whole_value() {
    expect_main(
        "positions",
        r#"  MUT target = 7
  MUT zero = 0
  MUT t = TRUE
  MUT no = FALSE
  target = (note("left", 1) + 2) * (10 / zero) + note("never", 1) TRAP(e)
    io::print("assign " & toString(e.code) & " target=" & toString(target))
    RECOVER 70
  END TRAP
  io::print("target " & toString(target))
  LET c = t AND 1 / zero > 0 TRAP(e)
    io::print("conditional " & toString(e.code))
    RECOVER FALSE
  END TRAP
  LET s = no AND 1 / zero > 0 TRAP(e)
    io::print("skipped " & toString(e.code))
    RECOVER TRUE
  END TRAP
  target / zero TRAP(e)
    io::print("discard " & toString(e.code))
    RECOVER 0
  END TRAP
  io::print(toString(c) & "," & toString(s))
"#,
        "eval left\nassign 77050002 target=7\ntarget 70\nconditional 77050002\n\
         discard 77050002\nFALSE,FALSE\n",
    );
}

/// bug-797: an operator-only trapped statement is captured by its handler, so a
/// local declared after it stays readable in the function-level handler.
#[test]
fn an_operator_only_trap_is_not_a_function_handler_failure_edge() {
    let source = format!(
        "{HEAD}FUNC census(z AS Integer) AS Integer\n  LET q = 10 / z TRAP(e)\n    RECOVER -1\n  END TRAP\n  LET marker = 7\n  io::print(toString(q))\n  RETURN marker\n  TRAP(f)\n    RETURN marker\n  END TRAP\nEND FUNC\n\nFUNC main() AS Integer\n  io::print(toString(census(0)))\n  RETURN 0\nEND FUNC\n"
    );
    let (exe, _) = build("census", &source, &[]);
    let (stdout, _) = run("census", &exe, &[]);
    assert_eq!(stdout, "-1\n7\n");
}

/// The contracts Option 1 does not touch: RECOVER typing, handler fall-through,
/// and the testing assertions' own call-only rule.
#[test]
fn recover_typing_fallthrough_and_testing_assertions_are_unchanged() {
    for (name, body, rule) in [
        (
            "recover_type",
            "  MUT z = 0\n  LET a = 1 / z TRAP(e)\n    RECOVER \"x\"\n  END TRAP\n  io::print(toString(a))\n",
            "TYPE_RECOVER_TYPE_MISMATCH",
        ),
        (
            "fallthrough",
            "  MUT z = 0\n  LET a = 1 / z TRAP(e)\n    io::print(\"x\")\n  END TRAP\n  io::print(toString(a))\n",
            "TYPE_INLINE_TRAP_FALLS_THROUGH",
        ),
    ] {
        let source = format!("{HEAD}FUNC main() AS Integer\n{body}  RETURN 0\nEND FUNC\n");
        let output = build_output(name, &source, &[]);
        let text = combined(&output);
        assert!(
            !output.status.success() && text.contains(rule),
            "{name}: expected {rule}:\n{text}"
        );
    }
    let testing = format!(
        "{HEAD}FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n\nTESTING\n  TGROUP \"g\"\n    TCASE \"operator\"\n      expectTrap(1 + 1)\n    END TCASE\n  END TGROUP\nEND TESTING\n"
    );
    let output = Command::new(common::mfb_exe())
        .arg("test")
        .arg(common::temp_project("p165d_testing", &testing))
        .output()
        .expect("run mfb test");
    let text = combined(&output);
    assert!(
        text.contains("TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE"),
        "expectTrap keeps its call-only rule:\n{text}"
    );
}

/// Allocation parity (bug-778): an operator-rooted trapped expression and a
/// known-total one allocate per evaluation exactly what their untrapped twins
/// do, at -O0/-O1/-O3 (N/2N delta); the failing operator form balances the arena
/// with nothing freed twice.
#[test]
fn operator_rooted_traps_allocate_what_the_plain_expression_does() {
    const SHAPES: &[(&str, &str, &str)] = &[
        ("arith", "Integer", "i * 2 + 3"),
        ("concat", "String", "a & b"),
        ("ratio", "Integer", "i / d"),
    ];
    let program = |trapped: bool| {
        let mut source = String::from(HEAD);
        for (name, type_, expr) in SHAPES {
            let statement = if trapped {
                format!("    LET v AS {type_} = {expr} TRAP(err)\n      RETURN -1\n    END TRAP\n")
            } else {
                format!("    LET v AS {type_} = {expr}\n")
            };
            let use_ = if *type_ == "String" { "len(v)" } else { "v" };
            source.push_str(&format!(
                "FUNC s_{name}(n AS Integer) AS Integer\n  MUT s AS Integer = 0\n  LET a AS String = \"ab\"\n  LET b AS String = \"cd\"\n  LET d AS Integer = 3\n  FOR i = 1 TO n\n{statement}    s = s + {use_}\n  NEXT\n  RETURN s\nEND FUNC\n\n"
            ));
        }
        source.push_str("FUNC main AS Integer\n  LET args AS List OF String = os::args()\n  LET which AS String = collections::get(args, 0)\n  LET n AS Integer = toInt(collections::get(args, 1))\n  MUT r AS Integer = 0\n");
        for (name, _, _) in SHAPES {
            source.push_str(&format!(
                "  IF which = \"{name}\" THEN\n    r = s_{name}(n)\n  END IF\n"
            ));
        }
        source.push_str("  io::print(toString(r))\n  RETURN 0\nEND FUNC\n");
        source
    };
    let trapped_source = program(true);
    let plain_source = program(false);
    for level in ["0", "1", "3"] {
        let (trapped, _) = build(
            &format!("alloc_trap_o{level}"),
            &trapped_source,
            &["--debug", "-O", level],
        );
        let (plain, _) = build(
            &format!("alloc_plain_o{level}"),
            &plain_source,
            &["--debug", "-O", level],
        );
        for (name, _, _) in SHAPES {
            let case = format!("{name} -O{level}");
            let delta = |exe: &Path| {
                let count = |n: &str| {
                    let (stdout, stderr) = run(&case, exe, &[name, n]);
                    let lines = arena_lines(&case, &stderr);
                    (counter(&case, &lines, 0, "alloc_calls"), stdout)
                };
                let (small, _) = count("1000");
                let (large, stdout) = count("2000");
                (large - small, stdout)
            };
            let (trapped_delta, trapped_value) = delta(&trapped);
            let (plain_delta, plain_value) = delta(&plain);
            assert_eq!(trapped_value, plain_value, "{case}: same result");
            assert_eq!(
                trapped_delta, plain_delta,
                "{case}: trapped {trapped_delta} allocations, plain {plain_delta}"
            );
        }
    }
    // The failing form, alternating with success, balances the arena.
    let failing = format!(
        "{HEAD}FUNC main() AS Integer\n  MUT caught = 0\n  MUT total = 0\n  FOR i = 1 TO 200\n    LET d = i MOD 2\n    LET q = 100 / d + 1 TRAP(e)\n      caught = caught + 1\n      RECOVER 0\n    END TRAP\n    LET s = toString(i) & \"x\" TRAP(e)\n      RECOVER \"\"\n    END TRAP\n    total = total + q + len(s)\n  NEXT\n  io::print(toString(caught) & \" \" & toString(total))\n  RETURN 0\nEND FUNC\n"
    );
    for level in ["0", "1", "3"] {
        let case = format!("failing -O{level}");
        let (exe, _) = build(
            &format!("failing_o{level}"),
            &failing,
            &["--debug", "-O", level],
        );
        let (stdout, stderr) = run(&case, &exe, &[]);
        assert!(stdout.starts_with("100 "), "{case}: {stdout}");
        let lines = arena_lines(&case, &stderr);
        assert_eq!(
            counter(&case, &lines, 0, "live_bytes"),
            0,
            "{case}:\n{stderr}"
        );
        assert_eq!(
            counter(&case, &lines, 0, "double_free_skips"),
            0,
            "{case}:\n{stderr}"
        );
    }
}
