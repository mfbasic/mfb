//! AST construction for the `mfb test` driver, plus assertion-placement
//! validation (plan-18-B §3.4–3.5).
//!
//! The driver is built directly as MFBASIC AST — an ordinary `FUNC … AS Integer`
//! entry point — so it flows through resolve/typecheck/lowering exactly like
//! hand-written source and needs no new runtime. Each `TCASE` becomes a
//! parameterless `SUB`; the driver calls each under an inline `TRAP`, reads the
//! error to tell an assertion failure (the reserved [`TEST_ABORT_CODE`]) from a
//! genuine runtime error, streams the pass/fail tree, and returns a non-zero exit
//! code iff any case failed.

use crate::ast::{
    AstProject, CallArg, Expression, Function, FunctionKind, Item, Statement, Visibility,
};
use crate::builtins::testing::{is_expect_call, TEST_ABORT_CODE};

/// One registered test case: its group/case descriptions (verbatim from source)
/// and the generated `SUB` name the driver invokes, in declaration order.
pub(crate) struct Registration {
    pub(crate) group: String,
    pub(crate) case: String,
    pub(crate) sub_name: String,
}

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
    use crate::builtins::testing::{EXPECT_EQ, EXPECT_NQ, EXPECT_NTRAP, EXPECT_TRAP};
    let argument = |index: usize| arguments.get(index).map(call_arg_value).cloned();
    match callee {
        EXPECT_EQ => expand_eq(argument(0), argument(1), uid, line, false),
        EXPECT_NQ => expand_eq(argument(0), argument(1), uid, line, true),
        EXPECT_TRAP => expand_trap(argument(0), argument(1), uid, line),
        EXPECT_NTRAP => expand_ntrap(argument(0), uid, line),
        _ => Vec::new(),
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
    handler.push(Statement::Recover {
        value: None,
        line,
    });
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

// ---------------------------------------------------------------------------
// Driver construction
// ---------------------------------------------------------------------------

/// Build the synthesized `#mfb_test_main AS Integer` driver from the registration
/// table (plan-18-B §3.5).
pub(crate) fn build_driver(registrations: &[Registration]) -> Function {
    let mut body: Vec<Statement> = Vec::new();
    // Failure tally; total is a compile-time constant.
    body.push(let_mut("#failed", "Integer", num(0)));
    body.push(let_mut("#ok", "Boolean", boolean(true)));

    let mut last_group: Option<&str> = None;
    for registration in registrations {
        if last_group != Some(registration.group.as_str()) {
            body.push(print_line(str_lit(format!("* {}", registration.group))));
            last_group = Some(registration.group.as_str());
        }
        body.push(assign("#ok", boolean(true)));
        body.push(case_call(registration));
        body.push(if_then(
            ident("#ok"),
            vec![print_line(str_lit(format!("  * [P] {}", registration.case)))],
            0,
        ));
    }

    let total = registrations.len() as i64;
    body.push(print_line(str_lit(String::new())));
    body.push(print_line(summary_line(total)));
    body.push(if_then(
        binary(ident("#failed"), ">", num(0)),
        vec![ret(num(1))],
        0,
    ));
    body.push(ret(num(0)));

    Function {
        kind: FunctionKind::Func,
        // Public so `scope_privates` never mangles the name — the entry point
        // pins it verbatim. (The case SUBs stay Private; their references are
        // rewritten consistently within this file.)
        visibility: Visibility::Public,
        isolated: false,
        name: super::DRIVER_NAME.to_string(),
        template_params: Vec::new(),
        params: Vec::new(),
        return_type: Some("Integer".to_string()),
        return_resource: false,
        return_state_type: None,
        body,
        trap: None,
        line: 0,
    }
}

/// `<sub>() TRAP(#e) …handler… END TRAP` — run one case under trap isolation.
fn case_call(registration: &Registration) -> Statement {
    let handler = vec![
        assign("#ok", boolean(false)),
        assign("#failed", binary(ident("#failed"), "+", num(1))),
        print_line(str_lit(format!("  * [F] {}", registration.case))),
        if_else(
            binary(member(ident("#e"), "code"), "=", num(TEST_ABORT_CODE)),
            vec![print_line(assertion_detail())],
            vec![print_line(runtime_detail())],
            0,
        ),
        Statement::Recover {
            value: None,
            line: 0,
        },
    ];
    Statement::Expression {
        expression: Expression::Trapped {
            expression: Box::new(call(&registration.sub_name, Vec::new())),
            binding: "#e".to_string(),
            handler,
            line: 0,
        },
        line: 0,
    }
}

/// `    X <message>  (<file>:<line>)` for an assertion failure — the message the
/// assertion baked into the reserved-code error, plus its stamped origin.
fn assertion_detail() -> Expression {
    concat(vec![
        str_lit("    X ".to_string()),
        member(ident("#e"), "message"),
        error_location(),
    ])
}

/// `    X runtime error [<code>] <message>  (<file>:<line>)` for a genuine trap.
fn runtime_detail() -> Expression {
    concat(vec![
        str_lit("    X runtime error [".to_string()),
        to_string(member(ident("#e"), "code")),
        str_lit("] ".to_string()),
        member(ident("#e"), "message"),
        error_location(),
    ])
}

/// `  (<e.source.filename>:<e.source.line>)`.
fn error_location() -> Expression {
    let source = member(ident("#e"), "source");
    concat(vec![
        str_lit("  (".to_string()),
        member(source.clone(), "filename"),
        str_lit(":".to_string()),
        to_string(member(source, "line")),
        str_lit(")".to_string()),
    ])
}

/// `Tests: N  Pass: <N - #failed>  Fail: <#failed>`.
fn summary_line(total: i64) -> Expression {
    concat(vec![
        str_lit(format!("Tests: {total}  Pass: ")),
        to_string(binary(num(total), "-", ident("#failed"))),
        str_lit("  Fail: ".to_string()),
        to_string(ident("#failed")),
    ])
}

// ---------------------------------------------------------------------------
// Placement validation (expect* only inside a TCASE)
// ---------------------------------------------------------------------------

/// Report every assertion builtin used outside a `TCASE` body. The `TESTING`
/// blocks themselves are validated in-place (their case bodies are the only legal
/// home for `expect*`); every other item — top-level bindings and ordinary
/// FUNC/SUB bodies — is walked for stray assertion calls. Returns `true` iff a
/// misplacement was reported. Runs on the parsed AST before any lowering, in both
/// build and test mode.
pub(crate) fn validate_expect_placement(ast: &AstProject) -> bool {
    let mut found = false;
    for file in &ast.files {
        for item in &file.items {
            match item {
                Item::Function(function) => {
                    walk_statements(&function.body, &file.path, &mut found);
                    if let Some(trap) = &function.trap {
                        walk_statements(&trap.body, &file.path, &mut found);
                    }
                }
                Item::Binding(binding) => {
                    if let Some(value) = &binding.value {
                        walk_expression(value, binding.line, &file.path, &mut found);
                    }
                }
                // TESTING case bodies are the one legal home for `expect*`.
                Item::Testing(_) => {}
                _ => {}
            }
        }
    }
    found
}

fn walk_statements(statements: &[Statement], path: &str, found: &mut bool) {
    for statement in statements {
        walk_statement(statement, path, found);
    }
}

fn walk_statement(statement: &Statement, path: &str, found: &mut bool) {
    match statement {
        Statement::Let { value, line, .. } => {
            if let Some(value) = value {
                walk_expression(value, *line, path, found);
            }
        }
        Statement::Return { value, line } | Statement::Recover { value, line } => {
            if let Some(value) = value {
                walk_expression(value, *line, path, found);
            }
        }
        Statement::Exit { code, line, .. } => {
            if let Some(code) = code {
                walk_expression(code, *line, path, found);
            }
        }
        Statement::Fail { error, line } => walk_expression(error, *line, path, found),
        Statement::Assign { value, line, .. } | Statement::StateAssign { value, line, .. } => {
            walk_expression(value, *line, path, found);
        }
        Statement::Expression { expression, line } => {
            walk_expression(expression, *line, path, found);
        }
        Statement::If {
            condition,
            then_body,
            else_body,
            line,
        } => {
            walk_expression(condition, *line, path, found);
            walk_statements(then_body, path, found);
            walk_statements(else_body, path, found);
        }
        Statement::Match {
            expression,
            cases,
            line,
        } => {
            walk_expression(expression, *line, path, found);
            for case in cases {
                if let Some(guard) = &case.guard {
                    walk_expression(guard, case.line, path, found);
                }
                walk_statements(&case.body, path, found);
            }
        }
        Statement::For {
            start,
            end,
            step,
            body,
            line,
            ..
        } => {
            walk_expression(start, *line, path, found);
            walk_expression(end, *line, path, found);
            if let Some(step) = step {
                walk_expression(step, *line, path, found);
            }
            walk_statements(body, path, found);
        }
        Statement::ForEach {
            iterable,
            body,
            line,
            ..
        } => {
            walk_expression(iterable, *line, path, found);
            walk_statements(body, path, found);
        }
        Statement::While {
            condition,
            body,
            line,
            ..
        } => {
            walk_expression(condition, *line, path, found);
            walk_statements(body, path, found);
        }
        Statement::DoUntil {
            body,
            condition,
            line,
        } => {
            walk_statements(body, path, found);
            walk_expression(condition, *line, path, found);
        }
        Statement::Continue { .. } | Statement::Propagate { .. } => {}
    }
}

fn walk_expression(expression: &Expression, line: usize, path: &str, found: &mut bool) {
    match expression {
        Expression::Call {
            callee,
            arguments,
            line: call_line,
            ..
        } => {
            if is_expect_call(callee) {
                *found = true;
                crate::rules::show_diagnostic(
                    "TESTING_EXPECT_OUTSIDE_TCASE",
                    &format!(
                        "`{callee}` is a test assertion and is valid only inside a TCASE body."
                    ),
                    std::path::Path::new(path),
                    *call_line,
                    1,
                    1,
                );
            }
            for argument in arguments {
                walk_expression(call_arg_value(argument), *call_line, path, found);
            }
        }
        Expression::Binary { left, right, .. } => {
            walk_expression(left, line, path, found);
            walk_expression(right, line, path, found);
        }
        Expression::Unary { operand, .. } => walk_expression(operand, line, path, found),
        Expression::Lambda { body, .. } => walk_expression(body, line, path, found),
        Expression::Constructor { arguments, .. } => {
            for argument in arguments {
                walk_expression(constructor_arg_value(argument), line, path, found);
            }
        }
        Expression::WithUpdate { target, updates } => {
            walk_expression(target, line, path, found);
            for update in updates {
                walk_expression(&update.value, update.line, path, found);
            }
        }
        Expression::ListLiteral(values) => {
            for value in values {
                walk_expression(value, line, path, found);
            }
        }
        Expression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                walk_expression(key, line, path, found);
                walk_expression(value, line, path, found);
            }
        }
        Expression::MemberAccess { target, .. } => walk_expression(target, line, path, found),
        Expression::Trapped {
            expression,
            handler,
            ..
        } => {
            walk_expression(expression, line, path, found);
            walk_statements(handler, path, found);
        }
        Expression::String(_)
        | Expression::Number(_)
        | Expression::Boolean(_)
        | Expression::Identifier(_) => {}
    }
}

fn call_arg_value(argument: &CallArg) -> &Expression {
    match argument {
        CallArg::Positional(value) => value,
        CallArg::Named { value, .. } => value,
    }
}

fn constructor_arg_value(argument: &crate::ast::ConstructorArg) -> &Expression {
    match argument {
        crate::ast::ConstructorArg::Positional(value) => value,
        crate::ast::ConstructorArg::Named { value, .. } => value,
    }
}

// ---------------------------------------------------------------------------
// AST builder helpers
// ---------------------------------------------------------------------------

fn str_lit(value: String) -> Expression {
    Expression::String(value)
}

fn num(value: i64) -> Expression {
    Expression::Number(value.to_string())
}

fn boolean(value: bool) -> Expression {
    Expression::Boolean(value)
}

fn ident(name: &str) -> Expression {
    Expression::Identifier(name.to_string())
}

fn binary(left: Expression, operator: &str, right: Expression) -> Expression {
    Expression::Binary {
        left: Box::new(left),
        operator: operator.to_string(),
        right: Box::new(right),
        line: 0,
        column: 0,
    }
}

fn member(target: Expression, name: &str) -> Expression {
    Expression::MemberAccess {
        target: Box::new(target),
        member: name.to_string(),
    }
}

fn call(callee: &str, arguments: Vec<Expression>) -> Expression {
    Expression::Call {
        callee: callee.to_string(),
        arguments: arguments.into_iter().map(CallArg::Positional).collect(),
        line: 0,
        column: 0,
    }
}

fn to_string(value: Expression) -> Expression {
    call("toString", vec![value])
}

/// Fold `parts` left-to-right with the string-concatenation operator `&`.
fn concat(parts: Vec<Expression>) -> Expression {
    let mut iter = parts.into_iter();
    let mut acc = iter.next().expect("concat needs at least one part");
    for part in iter {
        acc = binary(acc, "&", part);
    }
    acc
}

fn print_line(value: Expression) -> Statement {
    Statement::Expression {
        expression: call("io.print", vec![value]),
        line: 0,
    }
}

fn not(operand: Expression) -> Expression {
    Expression::Unary {
        operator: "NOT".to_string(),
        operand: Box::new(operand),
        line: 0,
        column: 0,
    }
}

fn let_mut(name: &str, type_name: &str, value: Expression) -> Statement {
    let_mut_at(name, type_name, value, 0)
}

fn let_mut_at(name: &str, type_name: &str, value: Expression, line: usize) -> Statement {
    Statement::Let {
        mutable: true,
        resource: false,
        state_type: None,
        name: name.to_string(),
        type_name: Some(type_name.to_string()),
        value: Some(value),
        line,
    }
}

fn let_imm(name: &str, value: Expression, line: usize) -> Statement {
    Statement::Let {
        mutable: false,
        resource: false,
        state_type: None,
        name: name.to_string(),
        type_name: None,
        value: Some(value),
        line,
    }
}

fn assign(name: &str, value: Expression) -> Statement {
    assign_at(name, value, 0)
}

fn assign_at(name: &str, value: Expression, line: usize) -> Statement {
    Statement::Assign {
        name: name.to_string(),
        value,
        line,
    }
}

fn if_then(condition: Expression, then_body: Vec<Statement>, line: usize) -> Statement {
    Statement::If {
        condition,
        then_body,
        else_body: Vec::new(),
        line,
    }
}

fn if_else(
    condition: Expression,
    then_body: Vec<Statement>,
    else_body: Vec<Statement>,
    line: usize,
) -> Statement {
    Statement::If {
        condition,
        then_body,
        else_body,
        line,
    }
}

/// `<inner> TRAP(binding) …handler… END TRAP` as a bare expression statement.
fn trap_stmt(
    inner: Expression,
    binding: &str,
    handler: Vec<Statement>,
    line: usize,
) -> Statement {
    Statement::Expression {
        expression: Expression::Trapped {
            expression: Box::new(inner),
            binding: binding.to_string(),
            handler,
            line,
        },
        line,
    }
}

fn ret(value: Expression) -> Statement {
    Statement::Return {
        value: Some(value),
        line: 0,
    }
}
