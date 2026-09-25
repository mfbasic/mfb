//! A failing callback inside a collections builtin reaches the function's `TRAP`.
//!
//! `collections::filter`, `transform`, `forEach`, `reduce`, `sortBy`, `mapValues`
//! and `findLastIndex` call a user function per element. When that function fails
//! and the call carries no inline `TRAP`, the error must travel like any other
//! failed call (`mfb man errors`, "Function-level TRAP": the function-level
//! `TRAP(e)` "traps every error from the body"), freeing the scope on the way.
//! `emit_callback_failure_exit` instead emitted a bare `ret` when no inline-`TRAP`
//! capture was active: the enclosing function's `TRAP` never ran, none of its
//! live locals were freed, and the error surfaced one frame too high — found
//! while writing plan-142-B's in-place `filter` (probe: the handler below printed
//! nothing and the program died with `Error: 7-705-0002`).

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const HELPERS: &str = "\
IMPORT collections
IMPORT io

FUNC failOnThree(n AS Integer) AS Boolean
  IF n = 3 THEN
    FAIL error(77050002, \"three\")
  END IF
  RETURN n > 0
END FUNC

FUNC mapFailOnThree(n AS Integer) AS Integer
  IF n = 3 THEN
    FAIL error(77050002, \"three\")
  END IF
  RETURN n * 10
END FUNC

FUNC addFailOnThree(total AS Integer, n AS Integer) AS Integer
  IF n = 3 THEN
    FAIL error(77050002, \"three\")
  END IF
  RETURN total + n
END FUNC

FUNC noneUntilThree(n AS Integer) AS Boolean
  IF n = 3 THEN
    FAIL error(77050002, \"three\")
  END IF
  RETURN FALSE
END FUNC

SUB visitFailOnThree(n AS Integer)
  IF n = 3 THEN
    FAIL error(77050002, \"three\")
  END IF
END SUB
";

/// Each member's failing call, as the initializer of a `LET` (or a bare statement
/// for `forEach`), inside a function with a function-level `TRAP`.
const CASES: &[(&str, &str)] = &[
    (
        "filter",
        "LET r AS List OF Integer = collections::filter(xs, failOnThree)",
    ),
    (
        "transform",
        "LET r AS List OF Integer = collections::transform(xs, mapFailOnThree)",
    ),
    ("forEach", "collections::forEach(xs, visitFailOnThree)"),
    (
        "reduce",
        "LET r AS Integer = collections::reduce(xs, 0, addFailOnThree)",
    ),
    (
        "sortBy",
        "LET r AS List OF Integer = collections::sortBy(xs, mapFailOnThree)",
    ),
    (
        "mapValues",
        "LET r AS Map OF String TO Integer = collections::mapValues(m, mapFailOnThree)",
    ),
    (
        "findLastIndex",
        // `findLastIndex` walks from the end, so its predicate must reject 4 to
        // reach the failing 3.
        "LET r AS Integer = collections::findLastIndex(xs, noneUntilThree)",
    ),
];

fn program(statement: &str) -> String {
    format!(
        "{HELPERS}
FUNC run() AS Integer
  LET xs AS List OF Integer = [1, 2, 3, 4]
  LET m AS Map OF String TO Integer = Map OF String TO Integer {{ \"a\" := 3 }}
  {statement}
  io::print(\"not reached\")
  RETURN 0

  TRAP(e)
    io::print(\"handler: \" & e.message & \" \" & toString(len(xs)))
    RETURN 1
  END TRAP
END FUNC

FUNC main() AS Integer
  io::print(\"run -> \" & toString(run()))
  RETURN 0
END FUNC
"
    )
}

#[test]
fn a_failing_callback_runs_the_enclosing_function_trap() {
    let mut failures = Vec::new();
    for (member, statement) in CASES {
        let project = common::temp_project(&format!("cbtrap_{member}"), &program(statement));
        let exe = common::build_project(&project);
        let output = Command::new(&exe).output().expect("run program");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let want = "handler: three 4\nrun -> 1\n";
        if !output.status.success() || stdout != want {
            failures.push(format!(
                "{member}: want {want:?} and exit 0, got {stdout:?} ({}), stderr {:?}",
                common::exit_description(&output.status),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
