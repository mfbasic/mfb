//! plan-165-C: an inline `TRAP` covers a fallible right operand of `AND`/`OR`
//! while preserving conditional evaluation.
//!
//! The right operand runs only when the left one does not decide the result (for
//! `AND` when it is true, for `OR` when it is false), in source order, and a
//! failure in it — a fallible call, a raising operator, or a raising comparison
//! (a `Fixed` compared with an out-of-range `Integer`, or with a non-finite
//! `Float`) — reaches the one inline handler and skips the rest of the guarded
//! expression. An unselected operand is never evaluated and never raises. Before
//! plan-165-C the build rejected these with `TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`
//! (bug-457, bug-471, bug-825); every case here also carries a function-level
//! `TRAP`, so an error that escaped the inline handler would print `fh …`.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use common::debug_report::{arena_lines, counter};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Helpers every program shares: `probe` prints and optionally fails, `mark`
/// records an effect in a global without any possibility of failure, `finish`
/// and `pair` consume a Boolean argument.
const HEAD: &str = r#"IMPORT io
IMPORT bits

MUT marks AS Integer = 0

FUNC probe(tag AS String, result AS Boolean, fails AS Boolean) AS Boolean
  io::print("eval " & tag)
  IF fails THEN
    FAIL error(90000001, tag)
  END IF
  RETURN result
END FUNC

FUNC mark(bit AS Integer) AS Boolean
  marks = bits::bor(marks, bit)
  RETURN TRUE
END FUNC

FUNC finish(tag AS String, flag AS Boolean) AS Integer
  io::print("finish " & tag & " " & toString(flag))
  RETURN 1
END FUNC

FUNC pair(flag AS Boolean, n AS Integer) AS Integer
  io::print("pair " & toString(flag) & " " & toString(n))
  RETURN n
END FUNC

FUNC boom(tag AS String) AS Integer
  io::print("eval " & tag)
  FAIL error(90000002, tag)
END FUNC

FUNC note(tag AS String, n AS Integer) AS Integer
  io::print("eval " & tag)
  RETURN n
END FUNC

"#;

fn project(name: &str, source: &str) -> PathBuf {
    common::temp_project(&format!("p165c_{name}"), source)
}

fn build_output(name: &str, source: &str, args: &[&str]) -> Output {
    Command::new(common::mfb_exe())
        .arg("build")
        .args(args)
        .arg(project(name, source))
        .output()
        .expect("run mfb build")
}

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
        .find(|path| path.ends_with(common::host_executable_suffix()))
        .or_else(|| written.first())
        .unwrap_or_else(|| panic!("{name}: no executable in build output:\n{stdout}"));
    PathBuf::from(chosen.trim())
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

/// Build `body` (a `main` body, with a function-level handler appended) at
/// `-O0` and `-O3` and require `expected` from both.
fn expect_main(name: &str, body: &str, expected: &str) {
    let source = format!(
        "{HEAD}FUNC main() AS Integer\n{body}  RETURN 0\n  TRAP(f)\n    io::print(\"fh \" & toString(f.code) & \" \" & f.message)\n    RETURN 3\n  END TRAP\nEND FUNC\n"
    );
    for level in ["0", "3"] {
        let exe = build(&format!("{name}_o{level}"), &source, &["-O", level]);
        let (stdout, _) = run(&format!("{name} -O{level}"), &exe, &[]);
        assert_eq!(stdout, expected, "{name} -O{level}");
    }
}

/// Boolean roots: an unselected operand is not evaluated; a selected one runs
/// and its failure reaches the inline handler, whose RECOVER supplies the whole
/// expression's value.
#[test]
fn a_selected_right_call_is_captured_and_an_unselected_one_never_runs() {
    expect_main(
        "roots",
        r#"  MUT yes = TRUE
  MUT no = FALSE
  LET a = no AND probe("skipped-and", TRUE, TRUE) TRAP(e)
    io::print("caught " & e.message)
    RECOVER TRUE
  END TRAP
  LET b = yes OR probe("skipped-or", FALSE, TRUE) TRAP(e)
    io::print("caught " & e.message)
    RECOVER FALSE
  END TRAP
  LET c = yes AND probe("and", TRUE, TRUE) TRAP(e)
    io::print("caught " & e.message)
    RECOVER FALSE
  END TRAP
  LET d = no OR probe("or", FALSE, TRUE) TRAP(e)
    io::print("caught " & e.message)
    RECOVER TRUE
  END TRAP
  LET g = yes AND probe("and-ok", TRUE, FALSE) TRAP(e)
    io::print("caught " & e.message)
    RECOVER FALSE
  END TRAP
  LET h = no OR probe("or-ok", FALSE, FALSE) TRAP(e)
    io::print("caught " & e.message)
    RECOVER TRUE
  END TRAP
  io::print(toString(a) & "," & toString(b) & "," & toString(c) & "," & toString(d) & "," & toString(g) & "," & toString(h))
"#,
        "eval and\ncaught and\neval or\ncaught or\neval and-ok\neval or-ok\n\
         FALSE,TRUE,FALSE,TRUE,TRUE,FALSE\n",
    );
}

/// Nested conditions, a failing left operand, and a failure in a conditional
/// that skips the later arguments and the call that consumes them.
#[test]
fn nested_conditions_and_failures_skip_exactly_the_rest() {
    expect_main(
        "nested",
        r#"  MUT yes = TRUE
  MUT no = FALSE
  LET a = probe("left", TRUE, FALSE) AND (no OR probe("nested", TRUE, FALSE)) TRAP(e)
    io::print("caught " & e.message)
    RECOVER FALSE
  END TRAP
  LET b = probe("left-fails", TRUE, TRUE) AND probe("never", TRUE, FALSE) TRAP(e)
    io::print("caught " & e.message)
    RECOVER FALSE
  END TRAP
  LET c = pair(yes AND probe("arg", TRUE, TRUE), note("later", 1)) TRAP(e)
    io::print("caught " & e.message)
    RECOVER -1
  END TRAP
  LET d = yes AND (yes AND (no OR probe("deep", TRUE, TRUE))) TRAP(e)
    io::print("caught " & e.message)
    RECOVER FALSE
  END TRAP
  LET g = pair(no AND probe("skipped", TRUE, TRUE), note("after", 2)) TRAP(e)
    io::print("caught " & e.message)
    RECOVER -1
  END TRAP
  io::print(toString(a) & "," & toString(b) & "," & toString(c) & "," & toString(d) & "," & toString(g))
"#,
        "eval left\neval nested\neval left-fails\ncaught left-fails\neval arg\ncaught arg\n\
         eval deep\ncaught deep\neval after\npair FALSE 2\nTRUE,FALSE,-1,FALSE,2\n",
    );
}

/// A conditional buried in a list element, a map value or a nested call
/// argument is captured the same way.
#[test]
fn a_conditional_inside_a_collection_or_nested_call_is_captured() {
    expect_main(
        "kinds",
        r#"  MUT yes = TRUE
  LET a = len([pair(yes AND probe("list", TRUE, TRUE), 1)]) TRAP(e)
    io::print("caught " & e.message)
    RECOVER -1
  END TRAP
  LET b = len(Map OF String TO Integer { "k" := pair(yes AND probe("map", TRUE, TRUE), 2) }) TRAP(e)
    io::print("caught " & e.message)
    RECOVER -2
  END TRAP
  LET c = pair(TRUE, pair(yes AND probe("nested-ok", TRUE, FALSE), 3)) TRAP(e)
    io::print("caught " & e.message)
    RECOVER -3
  END TRAP
  io::print(toString(a) & "," & toString(b) & "," & toString(c))
"#,
        "eval list\ncaught list\neval map\ncaught map\neval nested-ok\npair TRUE 3\n\
         pair TRUE 3\n-1,-2,3\n",
    );
}

/// A conditional whose right call cannot fail but has an effect still runs
/// before a later failing sibling, and the effect is not undone.
#[test]
fn an_infallible_side_effect_stays_ordered_before_a_later_failure() {
    expect_main(
        "order",
        r#"  MUT yes = TRUE
  LET a = pair(yes AND mark(1), boom("later")) TRAP(e)
    io::print("caught " & e.message & " marks=" & toString(marks))
    RECOVER -1
  END TRAP
  LET b = pair(yes AND mark(2), note("fine", 5)) TRAP(e)
    io::print("caught " & e.message)
    RECOVER -1
  END TRAP
  io::print(toString(a) & "," & toString(b) & " marks=" & toString(marks))
"#,
        "eval later\ncaught later marks=1\neval fine\npair TRUE 5\n-1,5 marks=3\n",
    );
}

/// Raising operators and raising comparisons in a selected right operand reach
/// the inline handler, in both operand orders; unselected, they never raise.
#[test]
fn raising_operators_and_comparisons_in_a_selected_operand_are_captured() {
    expect_main(
        "raising",
        r#"  MUT yes = TRUE
  MUT no = FALSE
  MUT zero = 0
  MUT fx = 1.5F
  MUT big = 3000000000
  MUT bigFloat = 1.0e300
  LET a = finish("div", yes AND 1 / zero > 0) TRAP(e)
    io::print("caught div " & toString(e.code))
    RECOVER -1
  END TRAP
  LET b = finish("div-skipped", no AND 1 / zero > 0) TRAP(e)
    io::print("caught div-skipped " & toString(e.code))
    RECOVER -1
  END TRAP
  LET c = finish("cmp", yes AND fx < big) TRAP(e)
    io::print("caught cmp " & toString(e.code))
    RECOVER -1
  END TRAP
  LET d = finish("cmp-rev", no OR big > fx) TRAP(e)
    io::print("caught cmp-rev " & toString(e.code))
    RECOVER -1
  END TRAP
  LET g = finish("cmp-skipped", no AND fx < big) TRAP(e)
    io::print("caught cmp-skipped " & toString(e.code))
    RECOVER -1
  END TRAP
  LET h = finish("nonfinite", yes AND fx < bigFloat * bigFloat) TRAP(e)
    io::print("caught nonfinite " & toString(e.code))
    RECOVER -1
  END TRAP
  LET k = finish("nonfinite-rev", yes AND bigFloat * bigFloat > fx) TRAP(e)
    io::print("caught nonfinite-rev " & toString(e.code))
    RECOVER -1
  END TRAP
  io::print(toString(a) & "," & toString(b) & "," & toString(c) & "," & toString(d) & "," & toString(g) & "," & toString(h) & "," & toString(k))
"#,
        "caught div 77050002\nfinish div-skipped FALSE\ncaught cmp 77050010\n\
         caught cmp-rev 77050010\nfinish cmp-skipped FALSE\ncaught nonfinite 77050003\n\
         caught nonfinite-rev 77050003\n-1,1,-1,-1,1,-1,-1\n",
    );
}

/// bug-798/bug-816's Float model holds in a selected right operand: a transient
/// non-finite compared with a `Float`, `Integer` or `Byte` is not observed and
/// does not trap; `a * b - c` contraction gives the same bits trapped or not.
#[test]
fn a_float_comparison_in_a_selected_operand_keeps_the_observation_model() {
    expect_main(
        "float",
        r#"  MUT yes = TRUE
  MUT bigFloat = 1.0e300
  MUT one = 1
  MUT b1 AS Byte = 1
  MUT x = 0.1
  MUT y = 0.2
  MUT z = 0.3
  LET a = finish("float", yes AND bigFloat * bigFloat > 1.0) TRAP(e)
    io::print("caught float " & toString(e.code))
    RECOVER -1
  END TRAP
  LET b = finish("integer", yes AND bigFloat * bigFloat > one) TRAP(e)
    io::print("caught integer " & toString(e.code))
    RECOVER -1
  END TRAP
  LET c = finish("byte", yes AND bigFloat * bigFloat > b1) TRAP(e)
    io::print("caught byte " & toString(e.code))
    RECOVER -1
  END TRAP
  LET plainValue = x * y - z
  LET d = finish("fma", yes AND x * y - z = plainValue) TRAP(e)
    io::print("caught fma " & toString(e.code))
    RECOVER -1
  END TRAP
  io::print(toString(a) & "," & toString(b) & "," & toString(c) & "," & toString(d))
"#,
        "finish float TRUE\nfinish integer TRUE\nfinish byte TRUE\nfinish fma TRUE\n1,1,1,1\n",
    );
}

/// The conditional result reaches an assignment, a discarded expression and a
/// binding; RECOVER supplies the whole value, and a failure leaves the
/// assignment target unchanged until then.
#[test]
fn conditional_results_reach_every_target() {
    expect_main(
        "targets",
        r#"  MUT yes = TRUE
  MUT target = TRUE
  target = yes AND probe("assign", FALSE, FALSE) TRAP(e)
    io::print("caught " & e.message)
    RECOVER TRUE
  END TRAP
  io::print("after assign " & toString(target))
  target = yes AND probe("assign-fails", FALSE, TRUE) TRAP(e)
    io::print("caught " & e.message & " target=" & toString(target))
    RECOVER TRUE
  END TRAP
  io::print("after recover " & toString(target))
  finish("discard", yes AND probe("discarded", TRUE, TRUE)) TRAP(e)
    io::print("caught " & e.message)
    RECOVER 0
  END TRAP
  finish("discard-ok", yes AND probe("discarded-ok", TRUE, FALSE)) TRAP(e)
    io::print("caught " & e.message)
    RECOVER 0
  END TRAP
"#,
        "eval assign\nafter assign FALSE\neval assign-fails\ncaught assign-fails target=FALSE\n\
         after recover TRUE\neval discarded\ncaught discarded\neval discarded-ok\n\
         finish discard-ok TRUE\n",
    );
}

/// A resource's `STATE` assignment target takes the conditional result the same
/// way, and keeps its old state until RECOVER supplies a new one.
#[test]
fn a_conditional_result_reaches_a_state_target() {
    let source = format!(
        "IMPORT fs\n{}TYPE Flag\n  ok AS Boolean\nEND TYPE\n\nFUNC main() AS Integer\n  MUT yes = TRUE\n  RES f AS fs::File STATE Flag = fs::createTempFile()\n  f.state = WITH f.state {{ ok := yes AND probe(\"state\", TRUE, FALSE) }} TRAP(e)\n    io::print(\"caught \" & e.message)\n    RECOVER WITH f.state {{ ok := FALSE }}\n  END TRAP\n  io::print(\"state \" & toString(f.state.ok))\n  f.state = WITH f.state {{ ok := yes AND probe(\"state-fails\", FALSE, TRUE) }} TRAP(e)\n    io::print(\"caught \" & e.message & \" old=\" & toString(f.state.ok))\n    RECOVER WITH f.state {{ ok := FALSE }}\n  END TRAP\n  io::print(\"state \" & toString(f.state.ok))\n  fs::close(f)\n  RETURN 0\nEND FUNC\n",
        HEAD
    );
    let exe = build("state", &source, &[]);
    let (stdout, _) = run("state", &exe, &[]);
    assert_eq!(
        stdout,
        "eval state\nstate TRUE\neval state-fails\ncaught state-fails old=TRUE\nstate FALSE\n"
    );
}

/// bug-797: the conditional's right operand is captured by the inline TRAP, so
/// it is not a failure edge into the function handler — a local declared after
/// the trapped statement stays readable there.
#[test]
fn a_trapped_conditional_is_not_a_function_handler_failure_edge() {
    let source = format!(
        "{HEAD}FUNC census(t AS Boolean) AS Integer\n  LET v = t AND probe(\"census\", TRUE, TRUE) TRAP(e)\n    RECOVER FALSE\n  END TRAP\n  LET marker = 7\n  io::print(toString(v))\n  RETURN marker\n  TRAP(f)\n    RETURN marker\n  END TRAP\nEND FUNC\n\nFUNC main() AS Integer\n  io::print(toString(census(TRUE)))\n  RETURN 0\nEND FUNC\n"
    );
    let exe = build("census", &source, &[]);
    let (stdout, _) = run("census", &exe, &[]);
    assert_eq!(stdout, "eval census\nFALSE\n7\n");
}

/// Payload temporaries in a right operand (String, record, List) are bound and
/// freed only on the path that evaluates them: skipped, selected-and-succeeding
/// and selected-and-failing branches alternate in one loop and the arena
/// balances with nothing freed twice, at -O0, -O1 and -O3.
const PAYLOAD_HEAD: &str = r#"IMPORT io
IMPORT os
IMPORT collections

TYPE Duo
  name AS String
  n AS Integer
END TYPE

FUNC makeStr(i AS Integer) AS String
  IF i MOD 3 = 2 THEN
    FAIL error(90000003, "str " & toString(i))
  END IF
  RETURN "s" & toString(i)
END FUNC

FUNC makeDuo(i AS Integer) AS Duo
  IF i MOD 3 = 2 THEN
    FAIL error(90000004, "duo " & toString(i))
  END IF
  RETURN Duo["d" & toString(i), i]
END FUNC

FUNC makeList(i AS Integer) AS List OF String
  IF i MOD 3 = 2 THEN
    FAIL error(90000005, "list " & toString(i))
  END IF
  RETURN ["l" & toString(i), "m"]
END FUNC

FUNC strOk(s AS String) AS Boolean
  RETURN len(s) > 1
END FUNC

FUNC duoOk(d AS Duo) AS Boolean
  RETURN d.n >= 0
END FUNC

FUNC listOk(l AS List OF String) AS Boolean
  RETURN len(l) = 2
END FUNC

"#;

/// One payload shape: the right operand that builds and consumes it.
const PAYLOAD_RIGHTS: &[(&str, &str)] = &[
    ("str", "strOk(makeStr(i))"),
    ("duo", "duoOk(makeDuo(i))"),
    ("list", "listOk(makeList(i))"),
];

#[test]
fn right_operand_payloads_balance_the_arena_on_every_path() {
    let mut source = String::from(PAYLOAD_HEAD);
    source.push_str("FUNC main() AS Integer\n  MUT caught = 0\n  MUT trues = 0\n  FOR i = 1 TO 60\n    LET sel = i MOD 2 = 0\n");
    for (name, right) in PAYLOAD_RIGHTS {
        source.push_str(&format!(
            "    LET v_{name} = sel AND {right} TRAP(e)\n      caught = caught + 1\n      RECOVER FALSE\n    END TRAP\n    IF v_{name} THEN\n      trues = trues + 1\n    END IF\n    LET w_{name} = NOT sel OR {right} TRAP(e)\n      caught = caught + 1\n      RECOVER FALSE\n    END TRAP\n    IF w_{name} THEN\n      trues = trues + 1\n    END IF\n"
        ));
    }
    source.push_str("  NEXT\n  io::print(\"caught=\" & toString(caught) & \" trues=\" & toString(trues))\n  RETURN 0\nEND FUNC\n");
    // Per payload and per operator, i runs 1..60: selected on even i (30), of
    // which i MOD 3 = 2 fails (i = 2, 8, …, 56: 10); the rest succeed (20).
    // AND: unselected -> FALSE (30). OR with NOT sel: unselected -> TRUE (30).
    let caught = 3 * 2 * 10;
    let trues = 3 * (20 + (30 + 20));
    let expected = format!("caught={caught} trues={trues}\n");
    for level in ["0", "1", "3"] {
        let case = format!("payload -O{level}");
        let exe = build(
            &format!("payload_o{level}"),
            &source,
            &["--debug", "-O", level],
        );
        let (stdout, stderr) = run(&case, &exe, &[]);
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

/// The success path allocates per evaluation exactly what the plain (untrapped)
/// conditional does: the right operand's temporaries stay on bug-778's frame
/// slots instead of falling back to a heap `Result`. N/2N delta method of
/// `rt_inline_trap_success_alloc.rs`; every input succeeds.
#[test]
fn the_trapped_conditional_success_path_allocates_what_the_plain_one_does() {
    let program = |trapped: bool| {
        let mut source = String::from(PAYLOAD_HEAD);
        for (name, right) in PAYLOAD_RIGHTS {
            let statement = if trapped {
                format!("    LET v = sel AND {right} TRAP(err)\n      RETURN -1\n    END TRAP\n")
            } else {
                format!("    LET v = sel AND {right}\n")
            };
            source.push_str(&format!(
                "FUNC s_{name}(n AS Integer) AS Integer\n  MUT s AS Integer = 0\n  FOR k = 1 TO n\n    LET i = (k MOD 2) * 3\n    LET sel = k MOD 4 < 2\n{statement}    IF v THEN\n      s = s + 1\n    END IF\n  NEXT\n  RETURN s\nEND FUNC\n\n"
            ));
        }
        source.push_str("FUNC main AS Integer\n  LET a AS List OF String = os::args()\n  LET which AS String = collections::get(a, 0)\n  LET n AS Integer = toInt(collections::get(a, 1))\n  MUT r AS Integer = 0\n");
        for (name, _) in PAYLOAD_RIGHTS {
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
        let trapped = build(
            &format!("alloc_trap_o{level}"),
            &trapped_source,
            &["--debug", "-O", level],
        );
        let plain = build(
            &format!("alloc_plain_o{level}"),
            &plain_source,
            &["--debug", "-O", level],
        );
        for (name, _) in PAYLOAD_RIGHTS {
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
                "{case}: 1000 trapped evaluations made {trapped_delta} allocations, \
                 the plain conditional {plain_delta}"
            );
        }
    }
}
