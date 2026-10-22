//! What an inline `TRAP` lifts out of its expression (bug-457 / bug-471 /
//! plan-165-D), pinned on the lowered IR of three shapes the other suites do
//! not lower: a fallible call under an implicit union wrap, a raising operator
//! over infallible calls, and an overload-dependent call over a file-scope
//! binding. Each program must also pass the checkers with no diagnostic — the IR
//! verifier rejects a `Checked` that still holds a call, or a raise left outside
//! the capture.

use crate::testutil::{check_src_details, lower_src_concrete};

const PRELUDE: &str = "\
IMPORT io

TYPE Circle
  r AS Float
END TYPE

TYPE Square
  side AS Float
END TYPE

UNION Shape
  Circle
  Square
END UNION

ENUM Mode
  Fast, Off, Slow
END ENUM

LET favorite AS Mode = Mode.Slow

FUNC inner(n AS Integer) AS Integer
  IF n < 0 THEN FAIL error(90000001, \"inner failed\")
  RETURN n * 2
END FUNC

FUNC mkf(n AS Integer) AS Square
  IF n < 0 THEN FAIL error(1, \"neg\")
  RETURN Square[2.0]
END FUNC

FUNC area(s AS Shape) AS Integer
  RETURN 1
END FUNC

";

/// The lowered IR (JSON) of a `main` binding `d` to `expression` under an
/// inline `TRAP`, after checking that the program is accepted.
fn trapped_ir(expression: &str) -> String {
    let source = format!(
        "{PRELUDE}\
FUNC main() AS Integer
  MUT n AS Integer = 5
  MUT s AS String = \"abc\"
  LET d = {expression} TRAP(e)
    RECOVER -1
  END TRAP
  io::print(toString(d))
  RETURN d
END FUNC
"
    );
    let diagnostics = check_src_details(&source);
    assert!(
        diagnostics.is_empty(),
        "expected acceptance of {expression}, got {:?}",
        diagnostics
            .iter()
            .map(|d| (d.rule.as_str(), d.detail.as_str()))
            .collect::<Vec<_>>()
    );
    lower_src_concrete(&source, None).to_json()
}

/// A fallible call whose result is union-wrapped into the outer call's `Shape`
/// parameter is found through the wrap and lifted as the checked step; the
/// outer call then wraps the lifted local, so the wrap survives the lift.
#[test]
fn a_fallible_call_under_a_union_wrap_is_lifted_through_it() {
    let ir = trapped_ir("area(mkf(n))");
    assert!(
        ir.contains(
            r##""name": "$trap_res1", "type": "#Result OF Square", "value": { "kind": "callResult", "type": "Square", "target": "mkf""##
        ),
        "{ir}"
    );
    assert!(
        ir.contains(
            r#""target": "area", "args": [{ "kind": "unionWrap", "union": "Shape", "member": "Square", "value": { "kind": "local", "name": "$trap_arg0" } }]"#
        ),
        "{ir}"
    );
}

/// plan-165-D: a raising operator over calls that cannot fail. The root becomes
/// the one check, a `Checked`, and a `Checked` may hold no call, so both `len`
/// calls are lifted ahead of it — unchecked, as plain calls.
#[test]
fn a_raising_operator_over_infallible_calls_lifts_every_call_unchecked() {
    let ir = trapped_ir("len(s) + len(s)");
    for arg in ["$trap_arg0", "$trap_arg1"] {
        assert!(
            ir.contains(&format!(
                r#""name": "{arg}", "type": "Integer", "value": {{ "kind": "call", "type": "Integer", "target": "len""#
            )),
            "{arg} is a plain lifted `len`: {ir}"
        );
    }
    assert!(
        ir.contains(
            r#""value": { "kind": "checked", "type": "Integer", "value": { "kind": "binary", "type": "Integer", "op": "+", "left": { "kind": "local", "name": "$trap_arg0" }, "right": { "kind": "local", "name": "$trap_arg1" } } }"#
        ),
        "{ir}"
    );
    assert!(
        !ir.contains("callResult"),
        "nothing here is a checked call: {ir}"
    );
}

/// bug-679 / bug-486: `toInt` of an enum cannot fail, and a file-scope `LET` is
/// typed from the binding environment, not left `Unknown` — so `toInt(favorite)`
/// stays in place as a plain call, and the only check is the fallible `inner`.
#[test]
fn an_overload_dependent_call_over_a_file_scope_binding_is_typed_from_it() {
    let ir = trapped_ir("inner(toInt(favorite))");
    assert!(
        ir.contains(
            r#""value": { "kind": "callResult", "type": "Integer", "target": "inner", "args": [{ "kind": "call", "type": "Integer", "target": "toInt", "args": [{ "kind": "global", "name": "favorite" }] }] }"#
        ),
        "{ir}"
    );
    assert_eq!(ir.matches("callResult").count(), 1, "{ir}");
}
