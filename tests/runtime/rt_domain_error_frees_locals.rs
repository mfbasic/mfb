//! bug-695: an inline domain error — a failing conversion (`toInt`), a division
//! by zero, an integer overflow, an index out of range — that leaves its function
//! must free what a failing CALL at the same point frees: the statement's pending
//! temporaries and every live owned local (`mfb spec language memory-semantics`
//! §14.7: an error edge is a scope edge). Before the fix
//! `emit_error_register_return` (`src/codegen/error/emission/builder_error_emission.rs`)
//! ended every inline raise with a bare return (no scope-drop walk), and its
//! function-level-`TRAP` route skipped the statement's temporaries.
//!
//! Each case raises three times, trapped by `main`, and must end with
//! `alloc_calls = free_calls` and `live_bytes 0` in the `--debug` report. The
//! `failingCall`, `failingFail` and `trappedTempsCall` cases are controls: the
//! same shapes failing through a call or a `FAIL`, which were leak-free already.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const PROGRAM: &str = r#"IMPORT collections
IMPORT fs
IMPORT io
IMPORT os

TYPE D
  lon AS Float
  age AS Integer
END TYPE

FUNC conv(text AS String) AS Integer
  RETURN toInt(text)
END FUNC

FUNC failingCall(text AS String, v AS Integer) AS Nothing
  LET q AS D = D[lon := 0.5, age := 1]
  LET n AS Integer = conv(text)
  io::print(toString(n + q.age))
END FUNC

FUNC failingFail(text AS String, v AS Integer) AS Nothing
  LET q AS D = D[lon := 0.5, age := 1]
  IF text = "x" THEN FAIL error(77050002, "neg")
  io::print(toString(q.age))
END FUNC

FUNC conversionRecord(text AS String, v AS Integer) AS Nothing
  LET q AS D = D[lon := 0.5, age := 1]
  LET n AS Integer = toInt(text)
  io::print(toString(n + q.age))
END FUNC

FUNC divisionRecord(text AS String, v AS Integer) AS Nothing
  LET q AS D = D[lon := 0.5, age := 1]
  LET n AS Integer = 10 / v
  io::print(toString(n + q.age))
END FUNC

FUNC overflowRecord(text AS String, v AS Integer) AS Nothing
  LET q AS D = D[lon := 0.5, age := 1]
  LET n AS Integer = 9223372036854775807 - v + q.age
  io::print(toString(n))
END FUNC

FUNC conversionList(text AS String, v AS Integer) AS Nothing
  LET xs AS List OF Integer = [1, 2, 3]
  LET n AS Integer = toInt(text)
  io::print(toString(n + len(xs)))
END FUNC

FUNC conversionString(text AS String, v AS Integer) AS Nothing
  LET s AS String = "owned-" & toString(v) & "-string"
  LET n AS Integer = toInt(text)
  io::print(s & toString(n))
END FUNC

FUNC resourceLocal(text AS String, v AS Integer) AS Nothing
  RES f AS fs::File = fs::createTempFile(".")
  LET q AS D = D[lon := 0.5, age := 1]
  LET n AS Integer = toInt(text)
  io::print(toString(n + q.age))
END FUNC

FUNC failingWith(text AS String, v AS Integer) AS Nothing
  LET q AS D = D[lon := 0.5, age := 1]
  LET r AS D = WITH q { age := toInt(text) }
  io::print(toString(r.age))
END FUNC

FUNC inForEach(text AS String, v AS Integer) AS Nothing
  LET q AS D = D[lon := 0.5, age := 1]
  LET names AS List OF String = ["a", "b", "c"]
  FOR EACH name IN names
    LET tag AS String = name & toString(q.age)
    IF name = "b" THEN
      io::print(tag & toString(toInt(text)))
    END IF
  NEXT
END FUNC

FUNC nestedBlock(text AS String, v AS Integer) AS Nothing
  LET q AS D = D[lon := 0.5, age := 1]
  MUT i AS Integer = 0
  WHILE i < 3
    LET inner AS D = D[lon := 1.5, age := i]
    IF inner.age = 1 THEN
      LET xs AS List OF Integer = [i, i, i]
      io::print(toString(len(xs) / v))
    END IF
    i = i + 1
  END WHILE
  io::print(toString(q.age))
END FUNC

FUNC statementTemps(text AS String, v AS Integer) AS Nothing
  LET q AS D = D[lon := 0.5, age := 1]
  LET s AS String = "a" & toString(q.age) & toString(10 / v)
  io::print(s)
END FUNC

FUNC indexOutOfRange(text AS String, v AS Integer) AS Nothing
  LET q AS D = D[lon := 0.5, age := 1]
  LET xs AS List OF Integer = [1, 2, 3]
  io::print(toString(collections::get(xs, 9 + v) + q.age))
END FUNC

FUNC trappedTemps(text AS String, v AS Integer) AS Nothing
  LET q AS D = D[lon := 0.5, age := 1]
  LET s AS String = "a" & toString(q.age) & toString(10 / v)
  io::print(s)
  RETURN
  TRAP(err)
    io::print("handler " & toString(q.age))
    RETURN
  END TRAP
END FUNC

FUNC trappedTempsCall(text AS String, v AS Integer) AS Nothing
  LET q AS D = D[lon := 0.5, age := 1]
  LET s AS String = "a" & toString(q.age) & toString(conv(text))
  io::print(s)
  RETURN
  TRAP(err)
    io::print("handler " & toString(q.age))
    RETURN
  END TRAP
END FUNC

FUNC run(which AS String) AS Nothing
  IF which = "failingCall" THEN failingCall("x", 0)
  IF which = "failingFail" THEN failingFail("x", 0)
  IF which = "conversionRecord" THEN conversionRecord("x", 0)
  IF which = "divisionRecord" THEN divisionRecord("x", 0)
  IF which = "overflowRecord" THEN overflowRecord("x", -5)
  IF which = "conversionList" THEN conversionList("x", 0)
  IF which = "conversionString" THEN conversionString("x", 0)
  IF which = "resourceLocal" THEN resourceLocal("x", 0)
  IF which = "failingWith" THEN failingWith("x", 0)
  IF which = "inForEach" THEN inForEach("x", 0)
  IF which = "nestedBlock" THEN nestedBlock("x", 0)
  IF which = "statementTemps" THEN statementTemps("x", 0)
  IF which = "indexOutOfRange" THEN indexOutOfRange("x", 0)
  IF which = "trappedTemps" THEN trappedTemps("x", 0)
  IF which = "trappedTempsCall" THEN trappedTempsCall("x", 0)
END FUNC

FUNC main() AS Integer
  LET which AS String = os::getEnvOr("CASE", "")
  FOR i = 1 TO 3
    run(which) TRAP(e)
      io::print("trapped " & toString(e.code))
      RECOVER
    END TRAP
  NEXT
  RETURN 0
END FUNC
"#;

/// `(case, the line printed by each of the three trapped iterations, what it covers)`.
const CASES: &[(&str, &str, &str)] = &[
    (
        "failingCall",
        "trapped 77050003",
        "control: a failing call frees the local",
    ),
    (
        "failingFail",
        "trapped 77050002",
        "control: FAIL frees the local",
    ),
    (
        "conversionRecord",
        "trapped 77050003",
        "toInt fails inline with a record local live",
    ),
    (
        "divisionRecord",
        "trapped 77050002",
        "10 / v with v = 0, record local live",
    ),
    (
        "overflowRecord",
        "trapped 77050010",
        "checked Integer + overflows, record local live",
    ),
    (
        "conversionList",
        "trapped 77050003",
        "toInt fails inline with a List local live",
    ),
    (
        "conversionString",
        "trapped 77050003",
        "toInt fails inline with an owned String local live",
    ),
    (
        "resourceLocal",
        "trapped 77050003",
        "toInt fails inline with a RES File and a record live",
    ),
    (
        "failingWith",
        "trapped 77050003",
        "a WITH field value fails inline",
    ),
    (
        "inForEach",
        "trapped 77050003",
        "the raise is inside a FOR EACH body with a loop-scoped local",
    ),
    (
        "nestedBlock",
        "trapped 77050002",
        "the raise is inside an IF inside a WHILE with block locals",
    ),
    (
        "statementTemps",
        "trapped 77050002",
        "the raise leaves the statement's pending temporaries",
    ),
    (
        "indexOutOfRange",
        "trapped 77050001",
        "collections::get past the end, record and List locals live",
    ),
    (
        "trappedTemps",
        "handler 1",
        "function-level TRAP route: the statement's pending temporaries",
    ),
    (
        "trappedTempsCall",
        "handler 1",
        "control: the same shape through a failing call",
    ),
];

fn arena_sum(report: &str, key: &str) -> u64 {
    report
        .lines()
        .filter_map(|line| line.strip_prefix("arena."))
        .filter_map(|rest| rest.split_once(key))
        .filter(|(arena, _)| !arena.contains('.'))
        .map(|(_, n)| n.trim().parse::<u64>().expect("a count"))
        .sum()
}

#[test]
fn an_inline_domain_error_frees_the_scope_it_leaves() {
    let project = common::temp_project("domain_error_frees_locals", PROGRAM);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "build failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exe = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .find(|p| p.ends_with("-glibc.out"))
        .or_else(|| {
            stdout
                .lines()
                .find_map(|line| line.strip_prefix("Wrote executable to "))
        })
        .expect("an executable")
        .to_string();

    let mut failures = Vec::new();
    for (case, line, covers) in CASES {
        // `resourceLocal` creates its temp file in the working directory, so the
        // project directory (removed below) collects it.
        let run = Command::new(&exe)
            .current_dir(&project)
            .env("CASE", case)
            .output()
            .expect("run the program");
        let text = String::from_utf8_lossy(&run.stdout).into_owned();
        let report = String::from_utf8_lossy(&run.stderr).into_owned();
        assert!(
            run.status.success(),
            "{case}: program failed ({}):\n{text}\n{report}",
            common::exit_description(&run.status)
        );
        let want = vec![line.to_string(); 3];
        let got: Vec<String> = text.lines().map(str::to_string).collect();
        assert_eq!(got, want, "{case} ({covers}): wrong output");
        let allocs = arena_sum(&report, ".alloc_calls ");
        let frees = arena_sum(&report, ".free_calls ");
        let live = arena_sum(&report, ".live_bytes ");
        if allocs != frees || live != 0 {
            failures.push(format!(
                "{case} ({covers}): alloc_calls {allocs}, free_calls {frees}, live_bytes {live}"
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&project);
    assert!(
        failures.is_empty(),
        "an inline raise leaked what it left:\n{}",
        failures.join("\n")
    );
}
