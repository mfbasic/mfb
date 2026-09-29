//! bug-780: a `FAIL` frees the temporaries of the statements that enclose it.
//!
//! `IF s = longKey() THEN FAIL error(…) END IF` holds `longKey()`'s block as a
//! pending temporary of the `IF` statement, freed at the `IF`'s end — which a
//! `FAIL` jumps past. A failing call at the same point frees it
//! (`emit_call_error_exit`) and so does a `RETURN`, but the `FAIL` exit
//! (`emit_error_value_exit`) did not: every failure leaked the condition's block
//! (64 B for the 45-byte string here). Output was right, so only the arena
//! counters show it.
//!
//! Each case fails `REPS` times — the error returned to a caller's function-level
//! `TRAP`, or routed to the failing function's own `TRAP`, with and without an
//! owned local alive (the two lowering paths: a direct exit, and one through the
//! scope cleanups). A `--debug` run must end with `alloc_calls = free_calls` and
//! `live_bytes 0`.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const REPS: u32 = 50;

const HELPERS: &str = "\
IMPORT collections
IMPORT io

FUNC longKey() AS String
  RETURN \"fail-fail-fail-fail-fail-fail-fail-fail-fail!\"
END FUNC
";

/// `(name, check body, caller)`. `check(s)` fails when `s` is `longKey()`.
/// `caller` is `\"outer\"` (the error returns to `tryOnce`'s `TRAP`) or
/// `\"own\"` (`check` has its own `TRAP`, which returns -1).
const CASES: &[(&str, &str, &str)] = &[
    (
        "if_condition_direct",
        "  IF s = longKey() THEN\n    FAIL error(77050002, \"boom\")\n  END IF\n  RETURN 0",
        "outer",
    ),
    (
        "if_condition_with_local",
        "  LET extra AS List OF String = [s, \"x\"]\n  IF s = longKey() THEN\n    FAIL error(77050002, \"boom\")\n  END IF\n  RETURN len(extra)",
        "outer",
    ),
    (
        "if_condition_own_trap",
        "  IF s = longKey() THEN\n    FAIL error(77050002, \"boom\")\n  END IF\n  RETURN 0\n  TRAP(e)\n    RETURN -1\n  END TRAP",
        "own",
    ),
    (
        "if_condition_own_trap_with_local",
        "  LET extra AS List OF String = [s, \"x\"]\n  IF s = longKey() THEN\n    FAIL error(77050002, \"boom\")\n  END IF\n  RETURN len(extra)\n  TRAP(e)\n    RETURN -1\n  END TRAP",
        "own",
    ),
    (
        "nested_if_conditions",
        "  IF len(s) > len(longKey()) - 100 THEN\n    IF s = longKey() THEN\n      FAIL error(77050002, \"boom\")\n    END IF\n  END IF\n  RETURN 0",
        "outer",
    ),
    (
        "while_condition",
        "  MUT n AS Integer = 0\n  WHILE s = longKey()\n    n = n + 1\n    IF n > 0 THEN\n      FAIL error(77050002, \"boom\")\n    END IF\n  END WHILE\n  RETURN n",
        "outer",
    ),
];

fn program(body: &str, caller: &str) -> String {
    let call = if caller == "own" {
        "  RETURN check(s)"
    } else {
        "  RETURN check(s)\n  TRAP(err)\n    RETURN -1\n  END TRAP"
    };
    format!(
        "{HELPERS}
FUNC check(s AS String) AS Integer
{body}
END FUNC

FUNC tryOnce(s AS String) AS Integer
{call}
END FUNC

FUNC main() AS Integer
  LET k AS String = longKey()
  MUT acc AS Integer = 0
  FOR i = 1 TO {REPS}
    acc = acc + tryOnce(k)
  NEXT
  io::print(toString(acc))
  RETURN 0
END FUNC
"
    )
}

/// Build with `--debug`, run, and return `(stdout, alloc_calls, free_calls,
/// live_bytes)` summed over the arenas.
fn run(name: &str, source: &str) -> (String, u64, u64, u64) {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name}: build failed:\n{stdout}\n{}\n--- source ---\n{source}",
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
    let out = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let report = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "{name}: program failed:\n{text}\n{report}"
    );
    let sum = |key: &str| -> u64 {
        report
            .lines()
            .filter_map(|line| line.strip_prefix("arena."))
            .filter_map(|rest| rest.split_once(key))
            .filter(|(arena, _)| !arena.contains('.'))
            .map(|(_, n)| n.trim().parse::<u64>().expect("a count"))
            .sum()
    };
    (
        text,
        sum(".alloc_calls "),
        sum(".free_calls "),
        sum(".live_bytes "),
    )
}

#[test]
fn a_fail_frees_the_enclosing_statements_temporaries() {
    let mut failures = Vec::new();
    for (name, body, caller) in CASES {
        let (text, alloc, free, live) = run(&format!("b780_{name}"), &program(body, caller));
        let want = format!("-{REPS}\n");
        if text != want {
            failures.push(format!(
                "{name}: the FAIL was not trapped every time: printed {text:?}, want {want:?}"
            ));
        } else if alloc != free || live != 0 {
            failures.push(format!(
                "{name}: leaked {} block(s), {live} B live over {REPS} failures",
                alloc as i64 - free as i64
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "bug-780: a FAIL must free the temporaries of the statements it jumps out of:\n  {}",
        failures.join("\n  ")
    );
}
