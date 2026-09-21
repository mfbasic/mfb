//! plan-142: a self-update that fails leaves its binding unchanged.
//!
//! `x = op(x, …)` is an assignment, and an assignment whose value fails never
//! happens (`mfb spec language memory-semantics` §14): a function-level `TRAP`
//! handler that reads `x` must see the value from before the statement. An
//! in-place arm writes `x`'s own block, so it must detect every failure it can
//! raise — a callback's error, an invalid range — **before** its first write. Each
//! case runs the failing self-update inside a function whose handler prints `x`,
//! and requires the original contents (`tests/runtime/rt_inplace_self_update.rs`
//! separately proves the arm is the path taken).

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const HELPERS: &str = "\
IMPORT collections
IMPORT io
IMPORT math

FUNC failOnThree(n AS Integer) AS Boolean
  IF n = 3 THEN
    FAIL error(77050002, \"three\")
  END IF
  RETURN n MOD 2 = 0
END FUNC

FUNC failOnThreeStr(s AS String) AS Boolean
  IF s = \"c\" THEN
    FAIL error(77050002, \"three\")
  END IF
  RETURN s <> \"a\"
END FUNC

FUNC showInts(xs AS List OF Integer) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & toString(v) & \",\"
  NEXT
  RETURN out
END FUNC

FUNC showFloats(xs AS List OF Float) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & toString(v) & \",\"
  NEXT
  RETURN out
END FUNC

FUNC showStrs(xs AS List OF String) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & v & \",\"
  NEXT
  RETURN out
END FUNC
";

/// `(case, declaration of x, the failing self-update, renderer, expected output)`.
const CASES: &[(&str, &str, &str, &str, &str)] = &[
    (
        "filter predicate fails on the 3rd element",
        "MUT x AS List OF Integer = [1, 2, 3, 4, 5]",
        "x = collections::filter(x, failOnThree)",
        "showInts",
        "1,2,3,4,5,",
    ),
    (
        "filter predicate fails, String list",
        "MUT x AS List OF String = [\"a\", \"b\", \"c\", \"d\"]",
        "x = collections::filter(x, failOnThreeStr)",
        "showStrs",
        "a,b,c,d,",
    ),
    (
        "mid with a negative count",
        "MUT x AS List OF Integer = [1, 2, 3, 4, 5]",
        "x = collections::mid(x, 1, -1)",
        "showInts",
        "1,2,3,4,5,",
    ),
    (
        "mid past the end",
        "MUT x AS List OF String = [\"a\", \"b\", \"c\"]",
        "x = collections::mid(x, 2, 5)",
        "showStrs",
        "a,b,c,",
    ),
    (
        "math::sqrt over a negative element (plan-142-C)",
        "MUT x AS List OF Float = [4.0, 9.0, -1.0, 16.0]",
        "x = math::sqrt(x)",
        "showFloats",
        "4.00,9.00,-1.00,16.00,",
    ),
    (
        "math::log over a zero element (plan-142-C)",
        "MUT x AS List OF Float = [1.0, 0.0, 2.0]",
        "x = math::log(x)",
        "showFloats",
        "1.00,0.00,2.00,",
    ),
    (
        "mid with a negative start",
        "MUT x AS List OF Integer = [1, 2, 3]",
        "x = collections::mid(x, -1, 1)",
        "showInts",
        "1,2,3,",
    ),
];

fn program(decl: &str, statement: &str, show: &str) -> String {
    format!(
        "{HELPERS}
FUNC run() AS Integer
  {decl}
  {statement}
  io::print(\"not reached \" & {show}(x))
  RETURN 0

  TRAP(e)
    io::print({show}(x))
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
fn a_failing_self_update_leaves_the_binding_unchanged() {
    let mut failures = Vec::new();
    for (index, (case, decl, statement, show, want)) in CASES.iter().enumerate() {
        let project = common::temp_project(
            &format!("inplace_atomic_{index}"),
            &program(decl, statement, show),
        );
        let exe = common::build_project(&project);
        let output = Command::new(&exe).output().expect("run program");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let expected = format!("{want}\nrun -> 1\n");
        if !output.status.success() || stdout != expected {
            failures.push(format!(
                "{case}: `{statement}` — want {expected:?}, got {stdout:?} ({}), stderr {:?}",
                common::exit_description(&output.status),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
