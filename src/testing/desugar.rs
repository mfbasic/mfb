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
    AstProject, CallArg, Expression, Function, FunctionKind, Item, LoopKind, Param, Statement,
    TopLevelBinding, Visibility,
};
use crate::builtins::testing::{is_expect_call, TEST_ABORT_CODE};
use crate::coverage::CovSlot;
use std::path::Path;

// Names of the generated coverage runtime helpers (plan-18-C). Plain (non-sigil)
// names so they lower as ordinary declarations; the `__mfb_cov` prefix makes a
// user collision vanishingly unlikely.
const COV_ARRAY: &str = "__mfb_cov";
const COV_FAILED: &str = "__mfb_cov_failed";
const COV_HIT: &str = "__mfb_cov_hit";
const COV_FAIL: &str = "__mfb_cov_fail";
const COV_DUMP: &str = "__mfb_cov_dump";
const COV_ZEROS: &str = "__mfb_cov_zeros";
const COV_EMPTY_STRINGS: &str = "__mfb_cov_empty_strings";

/// One line-emitting step in the driver's streamed report, in declaration order.
/// The flat step list carries the tree shape as per-step indentation so nested
/// `TGROUP`s render as an indented tree without the driver tracking depth.
pub(crate) enum DriverStep {
    /// A `TGROUP` header line: `<indent spaces>* <description>`.
    Group { indent: usize, description: String },
    /// A `TCASE` invocation. `indent` is the leading-space width of its
    /// `* [P]/[F]` line; the failure detail sits two columns deeper.
    Case {
        sub_name: String,
        description: String,
        indent: usize,
    },
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
/// table (plan-18-B §3.5). With `coverage`, each case records its failed source
/// line and the driver flushes the coverage counters before returning.
pub(crate) fn build_driver(steps: &[DriverStep], coverage: bool) -> Function {
    let mut body: Vec<Statement> = Vec::new();
    // Failure tally; total is a compile-time constant.
    body.push(let_mut("#failed", "Integer", num(0)));
    body.push(let_mut("#ok", "Boolean", boolean(true)));

    let mut total = 0i64;
    for step in steps {
        match step {
            DriverStep::Group { indent, description } => {
                let pad = " ".repeat(*indent);
                body.push(print_line(str_lit(format!("{pad}* {description}"))));
            }
            DriverStep::Case {
                sub_name,
                description,
                indent,
            } => {
                total += 1;
                let pad = " ".repeat(*indent);
                body.push(assign("#ok", boolean(true)));
                body.push(case_call(sub_name, description, *indent, coverage));
                body.push(if_then(
                    ident("#ok"),
                    vec![print_line(str_lit(format!("{pad}* [P] {description}")))],
                    0,
                ));
            }
        }
    }

    body.push(print_line(str_lit(String::new())));
    body.push(print_line(summary_line(total)));
    // Flush the coverage counters after every case has run.
    if coverage {
        body.push(Statement::Expression {
            expression: call(COV_DUMP, Vec::new()),
            line: 0,
        });
    }
    body.push(if_then(
        binary(ident("#failed"), ">", num(0)),
        vec![ret(num(1))],
        0,
    ));
    body.push(ret(num(0)));

    Function {
        kind: FunctionKind::Func,
        // Public so `scope_privates` never mangles the name — the entry point
        // pins it verbatim. (The generated case SUBs are likewise Public, since
        // the driver calls them across file boundaries; each stays in its own
        // originating file so its body keeps that file's import scope.)
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
/// `indent` is the leading-space width of the `* [F]` line; the failure detail
/// sits two columns deeper.
fn case_call(sub_name: &str, description: &str, indent: usize, coverage: bool) -> Statement {
    let pad = " ".repeat(indent);
    let detail_indent = indent + 2;
    let mut handler = vec![
        assign("#ok", boolean(false)),
        assign("#failed", binary(ident("#failed"), "+", num(1))),
        print_line(str_lit(format!("{pad}* [F] {description}"))),
        if_else(
            binary(member(ident("#e"), "code"), "=", num(TEST_ABORT_CODE)),
            vec![print_line(assertion_detail(detail_indent))],
            vec![print_line(runtime_detail(detail_indent))],
            0,
        ),
    ];
    // Record the failed source line for the coverage report's annotation.
    if coverage {
        let source = member(ident("#e"), "source");
        let loc = concat(vec![
            member(source.clone(), "filename"),
            str_lit(":".to_string()),
            to_string(member(source, "line")),
        ]);
        handler.push(Statement::Expression {
            expression: call(COV_FAIL, vec![loc]),
            line: 0,
        });
    }
    handler.push(Statement::Recover {
        value: None,
        line: 0,
    });
    Statement::Expression {
        expression: Expression::Trapped {
            expression: Box::new(call(sub_name, Vec::new())),
            binding: "#e".to_string(),
            handler,
            line: 0,
        },
        line: 0,
    }
}

/// `<indent>X <message>  (<file>:<line>)` for an assertion failure — the message
/// the assertion baked into the reserved-code error, plus its stamped origin.
fn assertion_detail(indent: usize) -> Expression {
    concat(vec![
        str_lit(format!("{}X ", " ".repeat(indent))),
        member(ident("#e"), "message"),
        error_location(),
    ])
}

/// `<indent>X runtime error [<code>] <message>  (<file>:<line>)` for a genuine trap.
fn runtime_detail(indent: usize) -> Expression {
    concat(vec![
        str_lit(format!("{}X runtime error [", " ".repeat(indent))),
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
// Coverage instrumentation (plan-18-C)
// ---------------------------------------------------------------------------

/// Instrument every user statement with a hit counter and append the coverage
/// runtime helpers to the first source file. Returns the `slot -> (file, line)`
/// map. Runs after the TESTING desugaring, so the generated driver and case SUBs
/// are already present; the driver and the `__mfb_cov*` helpers are skipped.
/// `project_dir` (absolute) fixes where the runtime writes its sidecar files.
pub(crate) fn instrument_coverage(ast: &mut AstProject, project_dir: &Path) -> Vec<CovSlot> {
    let mut slots: Vec<CovSlot> = Vec::new();
    for file in &mut ast.files {
        if file.internal || file.path.starts_with('<') {
            continue;
        }
        let path = file.path.clone();
        for item in &mut file.items {
            if let Item::Function(function) = item {
                if is_generated(&function.name) {
                    continue;
                }
                instrument_block(&mut function.body, &path, &mut slots);
                if let Some(trap) = function.trap.as_mut() {
                    instrument_block(&mut trap.body, &path, &mut slots);
                }
            }
        }
    }

    let covdata = project_dir.join(super::COVDATA_FILE);
    let covfail = project_dir.join(super::COVFAIL_FILE);
    let sink = ast
        .files
        .first_mut()
        .expect("a project has at least one source file");
    super::ensure_import(sink, "collections");
    super::ensure_import(sink, "fs");
    for helper in coverage_helpers(slots.len(), &covdata, &covfail) {
        sink.items.push(helper);
    }

    slots
}

/// The generated driver and `__mfb_cov*` helpers must not be instrumented.
fn is_generated(name: &str) -> bool {
    name == super::DRIVER_NAME || name.starts_with("__mfb_cov")
}

/// Prepend a `__mfb_cov_hit(slot)` call before every real-line statement in a
/// block, recursing into nested blocks first so inner slots precede outer ones in
/// source order is not required — only that each executed statement bumps its slot.
fn instrument_block(block: &mut Vec<Statement>, file: &str, slots: &mut Vec<CovSlot>) {
    let original = std::mem::take(block);
    for mut statement in original {
        instrument_nested(&mut statement, file, slots);
        let line = statement_line(&statement);
        if line > 0 {
            let slot = slots.len();
            slots.push(CovSlot {
                file: file.to_string(),
                line,
            });
            block.push(Statement::Expression {
                expression: call(COV_HIT, vec![num(slot as i64)]),
                line: 0,
            });
        }
        block.push(statement);
    }
}

/// Recurse coverage instrumentation into a statement's nested blocks.
fn instrument_nested(statement: &mut Statement, file: &str, slots: &mut Vec<CovSlot>) {
    match statement {
        Statement::If {
            then_body,
            else_body,
            ..
        } => {
            instrument_block(then_body, file, slots);
            instrument_block(else_body, file, slots);
        }
        Statement::For { body, .. }
        | Statement::ForEach { body, .. }
        | Statement::While { body, .. }
        | Statement::DoUntil { body, .. } => instrument_block(body, file, slots),
        Statement::Match { cases, .. } => {
            for case in cases.iter_mut() {
                instrument_block(&mut case.body, file, slots);
            }
        }
        // An inline `TRAP` handler rides on a statement's value expression
        // (`Expression::Trapped`). Its `handler` block is executed on failure but
        // was previously never instrumented, so those lines rendered Neutral even
        // when they ran (bug-93.2). Instrument the handler with the same
        // block/slot mechanism used for a `TRAP` statement body.
        Statement::Let {
            value: Some(expression),
            ..
        }
        | Statement::Return {
            value: Some(expression),
            ..
        } => instrument_trapped_handler(expression, file, slots),
        Statement::Assign { value, .. } | Statement::StateAssign { value, .. } => {
            instrument_trapped_handler(value, file, slots)
        }
        Statement::Expression { expression, .. } => {
            instrument_trapped_handler(expression, file, slots)
        }
        _ => {}
    }
}

/// If `expression` is an inline-`TRAP` value, instrument its handler block so the
/// handler's executed lines are counted (bug-93.2).
fn instrument_trapped_handler(expression: &mut Expression, file: &str, slots: &mut Vec<CovSlot>) {
    if let Expression::Trapped { handler, .. } = expression {
        instrument_block(handler, file, slots);
    }
}

fn statement_line(statement: &Statement) -> usize {
    match statement {
        Statement::Let { line, .. }
        | Statement::Return { line, .. }
        | Statement::Exit { line, .. }
        | Statement::Continue { line, .. }
        | Statement::Fail { line, .. }
        | Statement::Propagate { line, .. }
        | Statement::Recover { line, .. }
        | Statement::Assign { line, .. }
        | Statement::StateAssign { line, .. }
        | Statement::Expression { line, .. }
        | Statement::If { line, .. }
        | Statement::Match { line, .. }
        | Statement::For { line, .. }
        | Statement::ForEach { line, .. }
        | Statement::While { line, .. }
        | Statement::DoUntil { line, .. } => *line,
    }
}

/// Build the coverage runtime: the counter/failed-line globals, the increment and
/// record SUBs, and the shutdown dump SUB (plan-18-C §3).
fn coverage_helpers(slot_count: usize, covdata: &Path, covfail: &Path) -> Vec<Item> {
    let mut items = Vec::new();

    // FUNC __mfb_cov_zeros(n) AS List OF Integer — build `n` zeros at runtime
    // (a global list *literal* initializer is miscompiled; see bug-05).
    items.push(Item::Function(func(
        COV_ZEROS,
        vec![param("n", "Integer")],
        Some("List OF Integer"),
        vec![
            let_mut_at("r", "List OF Integer", empty_list(), 0),
            let_mut_at("i", "Integer", num(0), 0),
            while_loop(
                binary(ident("i"), "<", ident("n")),
                vec![
                    assign_at("r", call("collections.append", vec![ident("r"), num(0)]), 0),
                    assign_at("i", binary(ident("i"), "+", num(1)), 0),
                ],
            ),
            ret(ident("r")),
        ],
    )));

    // FUNC __mfb_cov_empty_strings() AS List OF String
    items.push(Item::Function(func(
        COV_EMPTY_STRINGS,
        Vec::new(),
        Some("List OF String"),
        vec![
            let_mut_at("r", "List OF String", empty_list(), 0),
            ret(ident("r")),
        ],
    )));

    // MUT __mfb_cov = __mfb_cov_zeros(N)
    items.push(Item::Binding(global_mut(
        COV_ARRAY,
        "List OF Integer",
        call(COV_ZEROS, vec![num(slot_count as i64)]),
    )));
    // MUT __mfb_cov_failed = __mfb_cov_empty_strings()
    items.push(Item::Binding(global_mut(
        COV_FAILED,
        "List OF String",
        call(COV_EMPTY_STRINGS, Vec::new()),
    )));

    // SUB __mfb_cov_hit(slot) — counters[slot] += 1
    items.push(Item::Function(sub(
        COV_HIT,
        vec![param("slot", "Integer")],
        vec![assign_at(
            COV_ARRAY,
            call(
                "collections.set",
                vec![
                    ident(COV_ARRAY),
                    ident("slot"),
                    binary(
                        call("collections.get", vec![ident(COV_ARRAY), ident("slot")]),
                        "+",
                        num(1),
                    ),
                ],
            ),
            0,
        )],
    )));

    // SUB __mfb_cov_fail(loc) — record a failed source line
    items.push(Item::Function(sub(
        COV_FAIL,
        vec![param("loc", "String")],
        vec![assign_at(
            COV_FAILED,
            call("collections.append", vec![ident(COV_FAILED), ident("loc")]),
            0,
        )],
    )));

    // SUB __mfb_cov_dump() — write counts + failed lines to the sidecar files
    items.push(Item::Function(sub(
        COV_DUMP,
        Vec::new(),
        vec![
            dump_list_to_file(COV_ARRAY, true, covdata),
            dump_list_to_file(COV_FAILED, false, covfail),
        ]
        .into_iter()
        .flatten()
        .collect(),
    )));

    items
}

/// Statements that serialize `list` (numeric when `numeric`, else String) to
/// `path`, one element per line, ignoring any write error.
fn dump_list_to_file(list: &str, numeric: bool, path: &Path) -> Vec<Statement> {
    let acc = format!("#dump_{list}");
    let idx = format!("#idx_{list}");
    let element = call("collections.get", vec![ident(list), ident(&idx)]);
    let rendered = if numeric { to_string(element) } else { element };
    vec![
        let_mut_at(&acc, "String", str_lit(String::new()), 0),
        let_mut_at(&idx, "Integer", num(0), 0),
        while_loop(
            binary(ident(&idx), "<", call("len", vec![ident(list)])),
            vec![
                assign_at(
                    &acc,
                    binary(
                        binary(ident(&acc), "&", rendered),
                        "&",
                        str_lit("\n".to_string()),
                    ),
                    0,
                ),
                assign_at(&idx, binary(ident(&idx), "+", num(1)), 0),
            ],
        ),
        // fs::writeText(path, acc) — swallow any failure (best-effort coverage).
        // bug-176 F: RECOVER (continue past the trap), not EXIT SUB — the two dump
        // blocks are flattened into one SUB, so an EXIT SUB on the first file's
        // write error would skip the second dump. RECOVER keeps each dump
        // independently best-effort.
        Statement::Expression {
            expression: Expression::Trapped {
                expression: Box::new(call(
                    "fs.writeText",
                    vec![str_lit(path.to_string_lossy().into_owned()), ident(&acc)],
                )),
                binding: "#dumpErr".to_string(),
                handler: vec![Statement::Recover {
                    value: None,
                    line: 0,
                }],
                line: 0,
            },
            line: 0,
        },
    ]
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

fn empty_list() -> Expression {
    Expression::ListLiteral(Vec::new())
}

fn while_loop(condition: Expression, body: Vec<Statement>) -> Statement {
    Statement::While {
        kind: LoopKind::While,
        condition,
        body,
        line: 0,
    }
}

fn param(name: &str, type_name: &str) -> Param {
    Param {
        name: name.to_string(),
        type_name: Some(type_name.to_string()),
        resource: false,
        state_type: None,
        default: None,
        line: 0,
    }
}

fn func(
    name: &str,
    params: Vec<Param>,
    return_type: Option<&str>,
    body: Vec<Statement>,
) -> Function {
    Function {
        kind: FunctionKind::Func,
        visibility: Visibility::Public,
        isolated: false,
        name: name.to_string(),
        template_params: Vec::new(),
        params,
        return_type: return_type.map(str::to_string),
        return_resource: false,
        return_state_type: None,
        body,
        trap: None,
        line: 0,
    }
}

fn sub(name: &str, params: Vec<Param>, body: Vec<Statement>) -> Function {
    Function {
        kind: FunctionKind::Sub,
        visibility: Visibility::Public,
        isolated: false,
        name: name.to_string(),
        template_params: Vec::new(),
        params,
        return_type: None,
        return_resource: false,
        return_state_type: None,
        body,
        trap: None,
        line: 0,
    }
}

fn global_mut(name: &str, type_name: &str, value: Expression) -> TopLevelBinding {
    TopLevelBinding {
        visibility: Visibility::Public,
        mutable: true,
        resource: false,
        state_type: None,
        name: name.to_string(),
        type_name: Some(type_name.to_string()),
        value: Some(value),
        line: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruments_inline_trap_handler_lines() {
        // A bare-expression statement carrying an inline `TRAP` whose handler runs
        // one statement at line 42. Instrumenting the block must emit a CovSlot for
        // the handler line so it is not rendered Neutral (bug-93.2).
        let handler = vec![Statement::Expression {
            expression: call("io.print", vec![ident("boom")]),
            line: 42,
        }];
        let mut block = vec![Statement::Expression {
            expression: Expression::Trapped {
                expression: Box::new(call("fs.readText", vec![ident("path")])),
                binding: "err".to_string(),
                handler,
                line: 7,
            },
            line: 7,
        }];
        let mut slots: Vec<CovSlot> = Vec::new();
        instrument_block(&mut block, "app.mfb", &mut slots);
        assert!(
            slots.iter().any(|slot| slot.line == 42 && slot.file == "app.mfb"),
            "expected a CovSlot for the inline-TRAP handler line, got {slots:?}"
        );
    }

    use crate::builtins::testing::{
        EXPECT_EQUAL, EXPECT_NEQUAL, EXPECT_NTRAP, EXPECT_TRAP,
    };

    /// Positional call arguments from a list of expressions.
    fn pos(values: Vec<Expression>) -> Vec<CallArg> {
        values.into_iter().map(CallArg::Positional).collect()
    }

    #[test]
    fn expand_equality_lowers_to_two_lets_and_a_guarded_fail() {
        // `expectEqual(a, e)` binds both operands, then FAILs when they differ.
        let out = expand_expect(EXPECT_EQUAL, &pos(vec![num(1), num(2)]), 0, 9);
        assert_eq!(out.len(), 3);
        assert!(matches!(out[0], Statement::Let { .. }));
        assert!(matches!(out[1], Statement::Let { .. }));
        // The guard's condition is the *negated* equality (fail on mismatch).
        let Statement::If { condition, .. } = &out[2] else {
            panic!("expected an IF guard");
        };
        assert!(matches!(condition, Expression::Unary { .. }));
    }

    #[test]
    fn expand_inequality_fails_when_operands_are_equal() {
        // `expectNEqual(a, e)` FAILs on a *positive* equality (the two matched).
        let out = expand_expect(EXPECT_NEQUAL, &pos(vec![num(1), num(1)]), 3, 4);
        assert_eq!(out.len(), 3);
        let Statement::If { condition, .. } = &out[2] else {
            panic!("expected an IF guard");
        };
        assert!(matches!(condition, Expression::Binary { .. }));
    }

    #[test]
    fn expand_trap_without_a_code_guards_the_flag() {
        // No expected code → flag init, the guarded expression, and one FAIL when
        // the trap never fired.
        let out = expand_expect(EXPECT_TRAP, &pos(vec![ident("risky")]), 1, 5);
        assert_eq!(out.len(), 3);
        assert!(matches!(out[0], Statement::Let { .. }));
        assert!(matches!(out[1], Statement::Expression { .. }));
        assert!(matches!(out[2], Statement::If { .. }));
    }

    #[test]
    fn expand_trap_with_a_code_checks_both_presence_and_value() {
        // An expected code adds a code capture and two guards (missing + mismatch).
        let out = expand_expect(EXPECT_TRAP, &pos(vec![ident("risky"), num(42)]), 2, 6);
        assert_eq!(out.len(), 6);
        // Two of the emitted statements are the missing/mismatch FAIL guards.
        let guards = out
            .iter()
            .filter(|statement| matches!(statement, Statement::If { .. }))
            .count();
        assert_eq!(guards, 2);
    }

    #[test]
    fn expand_ntrap_is_a_single_trap_whose_handler_fails() {
        let out = expand_expect(EXPECT_NTRAP, &pos(vec![ident("safe")]), 0, 7);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], Statement::Expression { .. }));
    }

    #[test]
    fn expand_expect_yields_nothing_for_unknown_or_argless_calls() {
        // An unrecognized callee produces no lowering.
        assert!(expand_expect("notAnAssertion", &pos(vec![num(1)]), 0, 0).is_empty());
        // Missing operands short-circuit every family.
        assert!(expand_expect(EXPECT_EQUAL, &[], 0, 0).is_empty());
        assert!(expand_expect(EXPECT_TRAP, &[], 0, 0).is_empty());
        assert!(expand_expect(EXPECT_NTRAP, &[], 0, 0).is_empty());
    }

    #[test]
    fn validate_flags_a_stray_assertion_outside_a_tcase() {
        // A broad program: `expectEqual` sits in an ordinary FUNC body (illegal) and
        // the walker also descends through every common statement/expression kind.
        let source = "\
TYPE Point
  x AS Integer
  y AS Integer
END TYPE

MUT g AS Integer = 0

FUNC f AS Integer
  MUT total AS Integer = 0
  LET a AS Integer = 1
  LET b AS Integer = 2
  IF total > 0 THEN
    expectEqual(a, b)
  ELSE
    total = 0
  END IF
  MATCH total
    CASE 0 WHEN a > 0
      total = 1
    CASE ELSE
      total = 2
  END MATCH
  FOR i = 1 TO 10 STEP 2
    total = total + i
  NEXT
  FOR EACH item IN [1, 2, 3]
    total = total + item
  NEXT
  WHILE total < 100
    total = total + 1
  WEND
  DO
    total = total + 1
  LOOP UNTIL total > 200
  LET lam AS Integer = LAMBDA(x AS Integer) -> x + 1
  LET neg AS Integer = -total
  LET truth AS Boolean = NOT (a = b)
  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"k\" := 1 }
  LET lst AS List OF Integer = [1, 2]
  LET pt AS Point = Point[x := 1, y := 2]
  LET up AS Point = WITH pt { x := 9 }
  LET member AS Integer = pt.x
  LET trapped AS Integer = risky() TRAP(err)
    RECOVER 0
  END TRAP
  EXIT FOR
  CONTINUE FOR
  FAIL \"boom\"
  PROPAGATE
  EXIT PROGRAM 1
  RETURN total
END FUNC
";
        let project = crate::testutil::project_from_src(source);
        assert!(
            validate_expect_placement(&project),
            "an `expectEqual` outside a TCASE must be flagged"
        );
    }

    #[test]
    fn validate_accepts_a_project_with_no_stray_assertions() {
        // The same shape without any `expect*` call is clean; the walker visits the
        // FUNC body (and its function-level TRAP) and finds nothing to report.
        let source = "\
FUNC f AS Integer
  MUT total AS Integer = 0
  IF total > 0 THEN
    total = 1
  END IF
  RETURN total
TRAP(err)
  RETURN 0
END TRAP
END FUNC
";
        let project = crate::testutil::project_from_src(source);
        assert!(!validate_expect_placement(&project));
    }
}
