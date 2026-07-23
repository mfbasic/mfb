use super::*;
use crate::ast::build::*;

/// The case body is used verbatim as the generated `SUB` body — the assertion
/// builtins inside it are lowered later, in `src/ir/lower.rs`. Kept as a named
/// seam so coverage instrumentation (plan-18-C) has a hook.
pub(crate) fn desugar_case_body(body: Vec<Statement>) -> Vec<Statement> {
    body
}

// ---------------------------------------------------------------------------
// Assertion lowering (called from src/ir/lower.rs)
// ---------------------------------------------------------------------------

/// Expand one assertion builtin call into the equivalent MFBASIC statements. The
/// result is lowered by the caller through the ordinary statement path, so no new
/// IR is introduced. `uid` disambiguates the synthesized temporaries. `line` is
/// the assertion's source line (stamped into the raised error's `ErrorLoc`).
///
/// A failed assertion raises `error(TEST_ABORT_CODE, <detail>)`; the synthesized
/// driver recognizes the code and formats `<detail>` plus the stamped location.
pub(crate) fn expand_expect(
    callee: &str,
    arguments: &[CallArg],
    uid: usize,
    line: usize,
) -> Vec<Statement> {
    use crate::builtins::testing::{
        is_equality_assert, is_inequality_assert, EXPECT_NTRAP, EXPECT_TRAP,
    };
    let argument = |index: usize| arguments.get(index).map(call_arg_value).cloned();
    // Every equality assertion (`expectEqual` and the typed `expectFloat`/… ) lowers
    // the same way — a `=` comparison and a FAIL on mismatch; the operand-type check
    // is a typecheck concern. The inequality family mirrors it with `<>`.
    if is_equality_assert(callee) {
        expand_eq(argument(0), argument(1), uid, line, false)
    } else if is_inequality_assert(callee) {
        expand_eq(argument(0), argument(1), uid, line, true)
    } else if callee == EXPECT_TRAP {
        expand_trap(argument(0), argument(1), uid, line)
    } else if callee == EXPECT_NTRAP {
        expand_ntrap(argument(0), uid, line)
    } else {
        Vec::new()
    }
}

fn expand_eq(
    actual: Option<Expression>,
    expected: Option<Expression>,
    uid: usize,
    line: usize,
    negate: bool,
) -> Vec<Statement> {
    let (Some(actual), Some(expected)) = (actual, expected) else {
        return Vec::new();
    };
    let actual_name = format!("$expect_a{uid}");
    let expected_name = format!("$expect_e{uid}");
    let equal = binary(ident(&actual_name), "=", ident(&expected_name));
    if negate {
        // expectNQ: fail when the two are equal.
        let detail = concat(vec![
            str_lit("expected values to differ, but both were ".to_string()),
            to_string(ident(&actual_name)),
        ]);
        vec![
            let_imm(&actual_name, actual, line),
            let_imm(&expected_name, expected, line),
            if_then(equal, vec![fail_test(detail, line)], line),
        ]
    } else {
        // expectEQ: fail when the two differ.
        let detail = concat(vec![
            str_lit("expected ".to_string()),
            to_string(ident(&expected_name)),
            str_lit(", got ".to_string()),
            to_string(ident(&actual_name)),
        ]);
        vec![
            let_imm(&actual_name, actual, line),
            let_imm(&expected_name, expected, line),
            if_then(not(equal), vec![fail_test(detail, line)], line),
        ]
    }
}

fn expand_trap(
    expression: Option<Expression>,
    expected_code: Option<Expression>,
    uid: usize,
    line: usize,
) -> Vec<Statement> {
    let Some(expression) = expression else {
        return Vec::new();
    };
    let trapped = format!("$expect_trapped{uid}");
    let error_binding = format!("$expect_err{uid}");
    let code_name = format!("$expect_code{uid}");

    let mut statements = vec![let_mut_at(&trapped, "Boolean", boolean(false), line)];

    // Guard the expression: a trap flips the flag (pass); no trap falls through.
    let mut handler = vec![assign_at(&trapped, boolean(true), line)];
    if expected_code.is_some() {
        statements.push(let_mut_at(&code_name, "Integer", num(0), line));
        handler.push(assign_at(
            &code_name,
            member(ident(&error_binding), "code"),
            line,
        ));
    }
    handler.push(Statement::Recover { value: None, line });
    statements.push(trap_stmt(expression, &error_binding, handler, line));

    match expected_code {
        None => {
            let detail = str_lit("expected a trap, but none occurred".to_string());
            statements.push(if_then(
                not(ident(&trapped)),
                vec![fail_test(detail, line)],
                line,
            ));
        }
        Some(code) => {
            let code_name_expected = format!("$expect_want{uid}");
            statements.push(let_imm(&code_name_expected, code, line));
            let missing_detail = concat(vec![
                str_lit("expected a trap with code ".to_string()),
                to_string(ident(&code_name_expected)),
                str_lit(", but none occurred".to_string()),
            ]);
            statements.push(if_then(
                not(ident(&trapped)),
                vec![fail_test(missing_detail, line)],
                line,
            ));
            let mismatch_detail = concat(vec![
                str_lit("expected trap code ".to_string()),
                to_string(ident(&code_name_expected)),
                str_lit(", got ".to_string()),
                to_string(ident(&code_name)),
            ]);
            statements.push(if_then(
                binary(
                    ident(&trapped),
                    "AND",
                    binary(ident(&code_name), "<>", ident(&code_name_expected)),
                ),
                vec![fail_test(mismatch_detail, line)],
                line,
            ));
        }
    }
    statements
}

fn expand_ntrap(expression: Option<Expression>, uid: usize, line: usize) -> Vec<Statement> {
    let Some(expression) = expression else {
        return Vec::new();
    };
    let error_binding = format!("$expect_err{uid}");
    // A trap here is a failure; the handler diverges (no RECOVER needed). No trap
    // falls through as a pass.
    let detail = concat(vec![
        str_lit("unexpected trap: ".to_string()),
        member(ident(&error_binding), "message"),
    ]);
    let handler = vec![fail_test(detail, line)];
    vec![trap_stmt(expression, &error_binding, handler, line)]
}

/// `FAIL error(TEST_ABORT_CODE, <detail>)` — raise the reserved assertion-abort
/// error carrying the failure detail.
fn fail_test(detail: Expression, line: usize) -> Statement {
    let error = Expression::Call {
        callee: "error".to_string(),
        arguments: vec![
            CallArg::Positional(num(TEST_ABORT_CODE)),
            CallArg::Positional(detail),
        ],
        line,
        column: 1,
    };
    Statement::Fail { error, line }
}
