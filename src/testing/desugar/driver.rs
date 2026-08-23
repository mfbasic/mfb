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

/// Cap on how many cases one generated chunk function holds (bug-445 follow-up).
///
/// The driver used to inline every case's run-and-report code into one
/// `#mfb_test_main` function. Each case contributes stack slots for its inline
/// TRAP handler's temporaries (the `[F]`/detail string concatenations), and the
/// slots are not reused across the flat sequence, so the function's frame grew
/// with the case count. Past a few hundred cases that frame consumed most of the
/// 8 MiB main-thread stack, leaving too little headroom for the deepest-recursing
/// case under test (e.g. a 48-level nested-JSON round-trip or a bounded-backtrack
/// regex) — which then overflowed the stack mid-suite with an EXC_BAD_ACCESS,
/// even though every case passed in isolation. Splitting the report loop across
/// many small chunk functions bounds each frame to this many cases' worth of
/// slots, restoring the headroom; the entry function then only sums their
/// results, so its own frame is tiny regardless of suite size. 8 keeps each chunk
/// frame small relative to the deep-recursing cases that run beneath it: measured
/// on a 547-case suite, a monolithic driver overflowed on ~every build and a
/// 32-case chunk was still occasionally over the edge, while an 8-case chunk
/// passed every build with wide margin.
const DRIVER_CHUNK_SIZE: usize = 8;

/// Build the synthesized driver from the registration table (plan-18-B §3.5).
///
/// Returns the entry function (`#mfb_test_main`) plus the chunk functions it
/// calls. The entry is thin — it calls each `#mfb_test_chunk_N`, accumulating
/// their failure counts, then prints the summary and returns the exit status —
/// so its stack frame does not grow with the suite (bug-445 follow-up; see
/// [`DRIVER_CHUNK_SIZE`]). With `coverage`, each case records its failed source
/// line and the entry flushes the coverage counters before returning.
pub(crate) fn build_driver(steps: &[DriverStep], coverage: bool) -> Vec<Function> {
    let total = steps
        .iter()
        .filter(|step| matches!(step, DriverStep::Case { .. }))
        .count() as i64;

    // Partition the flat step list into runs of at most DRIVER_CHUNK_SIZE cases.
    // A cut is made only *after* a case, so a group header always stays ahead of
    // (at least the first of) the cases it introduces; a group that spans a
    // boundary simply continues, unheaded, into the next chunk — the header was
    // already printed, and the report is a flat stream of lines either way.
    let mut chunks: Vec<Function> = Vec::new();
    let mut chunk_names: Vec<String> = Vec::new();
    let mut start = 0usize;
    let mut cases_in_run = 0usize;
    for (index, step) in steps.iter().enumerate() {
        if matches!(step, DriverStep::Case { .. }) {
            cases_in_run += 1;
        }
        if cases_in_run == DRIVER_CHUNK_SIZE {
            let name = format!("__mfb_test_chunk_{}", chunk_names.len());
            chunks.push(build_chunk(&name, &steps[start..=index], coverage));
            chunk_names.push(name);
            start = index + 1;
            cases_in_run = 0;
        }
    }
    if start < steps.len() {
        let name = format!("__mfb_test_chunk_{}", chunk_names.len());
        chunks.push(build_chunk(&name, &steps[start..], coverage));
        chunk_names.push(name);
    }

    let mut functions = chunks;
    functions.push(build_entry(&chunk_names, total, coverage));
    functions
}

/// One chunk function: run its slice of cases (headers + case calls) and return
/// this chunk's failure count. Structurally identical to the old monolithic
/// driver body, minus the summary/coverage-flush/exit-status tail (the entry
/// owns those). Kept small so its stack frame stays bounded (see
/// [`DRIVER_CHUNK_SIZE`]).
fn build_chunk(name: &str, steps: &[DriverStep], coverage: bool) -> Function {
    let mut body: Vec<Statement> = Vec::new();
    body.push(let_mut("#failed", "Integer", num(0)));
    body.push(let_mut("#ok", "Boolean", boolean(true)));
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
    body.push(ret(ident("#failed")));
    driver_function(name.to_string(), body)
}

/// The entry function (`#mfb_test_main`): call every chunk, accumulate their
/// failure counts, print the trailing blank line + summary, optionally flush
/// coverage, and return the exit status (1 iff any case failed). Its frame is
/// constant in the suite size.
fn build_entry(chunk_names: &[String], total: i64, coverage: bool) -> Function {
    let mut body: Vec<Statement> = Vec::new();
    body.push(let_mut("#failed", "Integer", num(0)));
    for name in chunk_names {
        body.push(assign(
            "#failed",
            binary(ident("#failed"), "+", call(name, Vec::new())),
        ));
    }
    body.push(print_line(str_lit(String::new())));
    body.push(print_line(summary_line(total)));
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
    driver_function(super::super::DRIVER_NAME.to_string(), body)
}

/// Shared shell for the entry and chunk functions: a public, parameterless
/// `FUNC … AS Integer` with the given name and body. Public so `scope_privates`
/// never mangles the name — the entry point pins `#mfb_test_main` verbatim and
/// the entry calls each chunk by literal name. (The generated case SUBs are
/// likewise Public, since the driver calls them across file boundaries; each
/// stays in its own originating file so its body keeps that file's import scope.)
fn driver_function(name: String, body: Vec<Statement>) -> Function {
    Function {
        kind: FunctionKind::Func,
        visibility: Visibility::Public,
        isolated: false,
        name,
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
