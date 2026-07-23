use super::*;
use crate::ast::build::*;

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
            DriverStep::Group {
                indent,
                description,
            } => {
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
        name: super::super::DRIVER_NAME.to_string(),
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
