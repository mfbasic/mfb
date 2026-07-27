use super::*;
use crate::ast::build::*;

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

    let covdata = project_dir.join(super::super::COVDATA_FILE);
    let covfail = project_dir.join(super::super::COVFAIL_FILE);
    let sink = ast
        .files
        .first_mut()
        .expect("a project has at least one source file");
    super::super::ensure_import(sink, "collections");
    super::super::ensure_import(sink, "fs");
    for helper in coverage_helpers(slots.len(), &covdata, &covfail) {
        sink.items.push(helper);
    }

    slots
}

/// The generated driver and `__mfb_cov*` helpers must not be instrumented.
fn is_generated(name: &str) -> bool {
    name == super::super::DRIVER_NAME || name.starts_with("__mfb_cov")
}

/// Prepend a `__mfb_cov_hit(slot)` call before every real-line statement in a
/// block, recursing into nested blocks first so inner slots precede outer ones in
/// source order is not required — only that each executed statement bumps its slot.
pub(super) fn instrument_block(block: &mut Vec<Statement>, file: &str, slots: &mut Vec<CovSlot>) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::build::*;
    use crate::ast::{AstFile, ExitTarget, LoopKind, MatchCase, MatchPattern, Trap};

    fn expr_stmt(line: usize) -> Statement {
        Statement::Expression {
            expression: call("io.print", vec![ident("x")]),
            line,
        }
    }

    fn trapped(handler_line: usize) -> Expression {
        Expression::Trapped {
            expression: Box::new(call("fs.readText", vec![ident("p")])),
            binding: "err".to_string(),
            handler: vec![expr_stmt(handler_line)],
            line: 0,
        }
    }

    fn lines(slots: &[CovSlot]) -> Vec<usize> {
        slots.iter().map(|s| s.line).collect()
    }

    #[test]
    fn instrument_block_covers_every_statement_kind() {
        // One block carrying every statement variant, so `instrument_nested` walks
        // each compound arm, `instrument_trapped_handler` runs for the value-bearing
        // arms, and `statement_line` maps every variant.
        let mut block = vec![
            Statement::If {
                condition: ident("c"),
                then_body: vec![expr_stmt(11)],
                else_body: vec![expr_stmt(12)],
                line: 10,
            },
            Statement::For {
                name: "i".to_string(),
                start: num(1),
                end: num(3),
                step: Some(num(1)),
                body: vec![expr_stmt(21)],
                line: 20,
            },
            Statement::ForEach {
                name: "e".to_string(),
                iterable: ident("xs"),
                body: vec![expr_stmt(31)],
                line: 30,
            },
            Statement::While {
                kind: LoopKind::While,
                condition: ident("c"),
                body: vec![expr_stmt(41)],
                line: 40,
            },
            Statement::DoUntil {
                body: vec![expr_stmt(51)],
                condition: ident("c"),
                line: 50,
            },
            Statement::Match {
                expression: ident("m"),
                cases: vec![MatchCase {
                    pattern: MatchPattern::Else,
                    guard: None,
                    body: vec![expr_stmt(61)],
                    line: 60,
                }],
                line: 60,
            },
            Statement::Let {
                mutable: false,
                resource: false,
                state_type: None,
                name: "a".to_string(),
                type_name: None,
                value: Some(trapped(71)),
                line: 70,
            },
            Statement::Return {
                value: Some(trapped(81)),
                line: 80,
            },
            Statement::Assign {
                name: "a".to_string(),
                value: trapped(91),
                line: 90,
            },
            Statement::StateAssign {
                resource: "r".to_string(),
                value: trapped(101),
                line: 100,
            },
            Statement::Expression {
                expression: trapped(111),
                line: 110,
            },
            // Leaf statements: exercise `statement_line` arms and the `_ => {}`
            // no-nested-block branch of `instrument_nested`.
            Statement::Exit {
                target: ExitTarget::Sub,
                code: None,
                line: 120,
            },
            Statement::Continue {
                kind: LoopKind::For,
                line: 130,
            },
            Statement::Fail {
                error: ident("e"),
                line: 140,
            },
            Statement::Propagate { line: 150 },
            Statement::Recover {
                value: None,
                line: 160,
            },
        ];
        let mut slots: Vec<CovSlot> = Vec::new();
        instrument_block(&mut block, "app.mfb", &mut slots);
        let got = lines(&slots);
        // Nested-block bodies were instrumented.
        for line in [11, 12, 21, 31, 41, 51, 61] {
            assert!(got.contains(&line), "missing nested slot {line}: {got:?}");
        }
        // Trapped-handler blocks of the value-bearing arms were instrumented.
        for line in [71, 81, 91, 101, 111] {
            assert!(got.contains(&line), "missing trap-handler slot {line}: {got:?}");
        }
        // Every top-level statement got its own slot.
        for line in [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160] {
            assert!(got.contains(&line), "missing statement slot {line}: {got:?}");
        }
    }

    #[test]
    fn instrument_coverage_instruments_the_function_trap_body() {
        let mut f = func("main", Vec::new(), Some("Integer"), vec![expr_stmt(5)]);
        f.trap = Some(Trap {
            name: "err".to_string(),
            body: vec![expr_stmt(9)],
            line: 8,
        });
        let file = AstFile {
            path: "main.mfb".to_string(),
            imports: Vec::new(),
            items: vec![Item::Function(f)],
            internal: false,
        };
        let mut ast = AstProject {
            name: "p".to_string(),
            files: vec![file],
        };
        let slots = instrument_coverage(&mut ast, Path::new("/tmp/cov"));
        let got = lines(&slots);
        assert!(got.contains(&5), "body line missing: {got:?}");
        assert!(got.contains(&9), "trap-body line missing: {got:?}");
    }

    #[test]
    fn is_generated_skips_driver_and_cov_helpers() {
        assert!(is_generated(COV_HIT));
        assert!(is_generated(crate::testing::DRIVER_NAME));
        assert!(!is_generated("user_func"));
    }
}
