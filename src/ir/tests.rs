use super::*;

/// plan-20-D: IR lowering must be **total** — it never panics on ill-typed but
/// parse/resolve-clean input. The 371 `*-invalid` fixtures are exactly such
/// programs (they parse and resolve but fail a semantic rule). Today the AST
/// type checker rejects them before lowering; once the checker moves onto the
/// IR (plan-20-Z) lowering runs first, so it must survive them. This test
/// drives every `*-invalid` fixture through parse → resolve → monomorph →
/// **lower** (skipping typecheck) and asserts lowering does not panic. Fixtures
/// that fail before lowering (parse/resolve/monomorph errors — also pre-lowering
/// rejections) are skipped; the assertion is purely "if it reaches lowering, it
/// does not panic".
#[cfg(test)]
mod lowering_totality_tests {
    use crate::ast;
    use crate::manifest::validate_project_manifest;
    use crate::monomorph;
    use crate::resolver;
    use std::path::{Path, PathBuf};

    fn invalid_fixture_dirs() -> Vec<PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let mut dirs = Vec::new();
        let Ok(entries) = std::fs::read_dir(&root) else {
            return dirs;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Compile-time-invalid fixtures reach lowering only if they parse
            // and resolve; runtime-invalid (`*-invalid-rt`) fixtures are valid
            // programs that fail at run time, so they lower normally too.
            if name.ends_with("-invalid") && path.join("project.json").is_file() {
                dirs.push(path);
            }
        }
        dirs.sort();
        dirs
    }

    /// Silence the diagnostics the front end prints for invalid fixtures so the
    /// test output stays readable; we only care whether lowering panics.
    fn lower_fixture_without_panic(dir: &Path) -> Result<bool, ()> {
        let manifest = validate_project_manifest(&dir.join("project.json"))?;
        let name = manifest
            .get("name")
            .and_then(|v| v.get::<String>())
            .cloned()
            .ok_or(())?;
        let ast = ast::parse_project(&name, dir, &manifest)?;
        resolver::resolve_project(dir, &manifest, &ast)?;
        let concrete = monomorph::monomorphize_project(dir, &ast)?;
        resolver::resolve_project_with(dir, &manifest, &concrete, false)?;
        // Reached lowering: it must not panic. Entry + external package
        // functions are irrelevant to totality, so pass the minimal inputs.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::lower_project_with_external_functions(
                &concrete,
                None,
                &std::collections::HashMap::new(),
                &std::collections::HashMap::new(),
            );
        }));
        Ok(result.is_ok())
    }

    #[test]
    fn lowering_is_total_over_invalid_fixtures() {
        // Suppress the front end's diagnostic noise (invalid fixtures print
        // many errors on the way to the resolve/monomorph gate).
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let mut reached = 0usize;
        let mut panicked = Vec::new();
        for dir in invalid_fixture_dirs() {
            match lower_fixture_without_panic(&dir) {
                Ok(true) => reached += 1,
                Ok(false) => panicked.push(dir.display().to_string()),
                Err(()) => {} // rejected before lowering — not our concern
            }
        }
        std::panic::set_hook(prev_hook);
        assert!(
            panicked.is_empty(),
            "IR lowering panicked on {} fixture(s) (not total): {:?}",
            panicked.len(),
            panicked
        );
        // Sanity: a meaningful number of fixtures actually reached lowering,
        // so the test is exercising the total paths, not vacuously passing.
        assert!(
            reached > 50,
            "only {reached} invalid fixtures reached lowering; expected the \
             typecheck-invalid majority — the pipeline wiring may be broken"
        );
    }

    /// The `(line, rule_id)` diagnostic sequence the golden build.log records —
    /// the AST type checker's output for this invalid fixture, our porting
    /// oracle. Parses lines of the form `<file>:<line> error[<code> <RULE>]:`.
    fn golden_diagnostics(dir: &Path) -> Vec<(u32, String)> {
        let Ok(log) = std::fs::read_to_string(dir.join("golden").join("build.log")) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for line in log.lines() {
            // `...main.mfb:4 error[2-203-0001 TYPE_BINARY_OPERATOR_MISMATCH]: ...`
            let Some(err_at) = line.find(" error[") else {
                continue;
            };
            let before = &line[..err_at];
            let Some(colon) = before.rfind(':') else {
                continue;
            };
            let Ok(lineno) = before[colon + 1..].parse::<u32>() else {
                continue;
            };
            let bracket = &line[err_at + " error[".len()..];
            let Some(close) = bracket.find(']') else {
                continue;
            };
            let inner = &bracket[..close];
            let Some(rule) = inner.split_whitespace().nth(1) else {
                continue;
            };
            out.push((lineno, rule.to_string()));
        }
        out
    }

    /// The `(line, rule_id)` sequence `ir::verify` produces for the lowered IR.
    /// Returns `None` if the fixture is rejected before lowering.
    fn verify_diagnostics(dir: &Path) -> Option<Vec<(u32, String)>> {
        let manifest = validate_project_manifest(&dir.join("project.json")).ok()?;
        let name = manifest.get("name").and_then(|v| v.get::<String>()).cloned()?;
        let ast = ast::parse_project(&name, dir, &manifest).ok()?;
        resolver::resolve_project(dir, &manifest, &ast).ok()?;
        let concrete = monomorph::monomorphize_project(dir, &ast).ok()?;
        resolver::resolve_project_with(dir, &manifest, &concrete, false).ok()?;
        let ir = super::lower_project_with_external_functions(
            &concrete,
            None,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        );
        // No packages here, so the lowered project IR is what the checker sees.
        Some(
            super::verify::collect_diagnostics(&ir)
                .into_iter()
                .map(|d| (d.line, d.rule))
                .collect(),
        )
    }

    /// Porting-progress report (plan-20-E..I): for every invalid fixture that
    /// reaches lowering, compare the rule ids `ir::verify` produces against the
    /// golden (the AST checker). Not an assertion — a census that names which
    /// rules are still only in `typecheck` (MISSING) so the port can drive them
    /// to zero. Run with `--nocapture`.
    #[test]
    #[ignore = "porting census (plan-20-E..I); run with --ignored --nocapture"]
    fn verify_vs_typecheck_diagnostic_parity() {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        use std::collections::BTreeMap;
        let mut missing: BTreeMap<String, usize> = BTreeMap::new();
        let mut matched: BTreeMap<String, usize> = BTreeMap::new();
        let mut extra: BTreeMap<String, usize> = BTreeMap::new();
        let mut fixtures = 0usize;
        for dir in invalid_fixture_dirs() {
            let expected = golden_diagnostics(&dir);
            let Some(actual) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                verify_diagnostics(&dir)
            }))
            .ok()
            .flatten() else {
                continue;
            };
            fixtures += 1;
            // Count expected rule ids not produced by verify (per fixture, as a
            // multiset of rule ids — line-agnostic for the census).
            let mut act_rules: Vec<String> = actual.iter().map(|(_, r)| r.clone()).collect();
            for (_, rule) in &expected {
                if let Some(pos) = act_rules.iter().position(|r| r == rule) {
                    act_rules.remove(pos);
                    *matched.entry(rule.clone()).or_default() += 1;
                } else {
                    *missing.entry(rule.clone()).or_default() += 1;
                }
            }
            for rule in act_rules {
                if std::env::var("CENSUS_EXTRA").is_ok() {
                    eprintln!("EXTRA {rule} in {}", dir.display());
                }
                *extra.entry(rule).or_default() += 1;
            }
        }
        std::panic::set_hook(prev);
        eprintln!("\n=== verify-vs-typecheck census ({fixtures} fixtures reached lowering) ===");
        eprintln!("MISSING (typecheck emits, ir::verify does not) — port these:");
        for (rule, n) in &missing {
            eprintln!("  {n:3}  {rule}");
        }
        eprintln!("MATCHED (ir::verify already emits):");
        for (rule, n) in &matched {
            eprintln!("  {n:3}  {rule}");
        }
        eprintln!("EXTRA (ir::verify emits, typecheck did not — over-rejection risk):");
        for (rule, n) in &extra {
            eprintln!("  {n:3}  {rule}");
        }
    }
}

#[cfg(test)]
mod binary_repr_tests {
    use super::*;

    fn sample_value() -> IrValue {
        IrValue::Binary {
            op: "+".to_string(),
            left: Box::new(IrValue::Const {
                type_: "Integer".to_string(),
                value: "1".to_string(),
            }),
            right: Box::new(IrValue::Unary {
                op: "-".to_string(),
                operand: Box::new(IrValue::Local("x".to_string())),
                loc: IrSourceLoc::default(),
                type_: "Unknown".to_string(),
            }),
            loc: IrSourceLoc::default(),
            type_: "Unknown".to_string(),
        }
    }

    // Build a project exercising every IrType, IrOp, IrValue, and IrMatchPattern kind.
    fn corpus_project() -> IrProject {
        let every_value = vec![
            IrValue::Const {
                type_: "String".to_string(),
                value: "hi".to_string(),
            },
            IrValue::Local("a".to_string()),
            IrValue::Global("g".to_string()),
            IrValue::LocalRef {
                name: "a".to_string(),
                type_: "Integer".to_string(),
            },
            IrValue::FunctionRef {
                name: "f".to_string(),
                type_: "() -> Integer".to_string(),
            },
            IrValue::Closure {
                name: "lam".to_string(),
                type_: "() -> Integer".to_string(),
                captures: vec![IrValue::Local("a".to_string())],
            },
            IrValue::Capture {
                index: 3,
                type_: "Integer".to_string(),
                by_ref: true,
            },
            IrValue::Call {
                target: "g".to_string(),
                args: vec![sample_value()],
                loc: IrSourceLoc::default(),
                type_: "Unknown".to_string(),
            },
            IrValue::CallResult {
                target: "toInt".to_string(),
                args: vec![IrValue::Local("s".to_string())],
                loc: IrSourceLoc::default(),
                type_: "Unknown".to_string(),
            },
            IrValue::Constructor {
                type_: "Point".to_string(),
                args: vec![sample_value(), sample_value()],
            },
            IrValue::UnionWrap {
                union_type: "Shape".to_string(),
                member_type: "Point".to_string(),
                value: Box::new(IrValue::Local("p".to_string())),
            },
            IrValue::UnionExtract {
                type_: "Point".to_string(),
                value: Box::new(IrValue::Local("s".to_string())),
            },
            IrValue::ResultIsOk {
                value: Box::new(IrValue::Local("r".to_string())),
            },
            IrValue::ResultValue {
                value: Box::new(IrValue::Local("r".to_string())),
                type_: "Unknown".to_string(),
            },
            IrValue::ResultError {
                value: Box::new(IrValue::Local("r".to_string())),
            },
            IrValue::WithUpdate {
                type_: "Point".to_string(),
                target: Box::new(IrValue::Local("p".to_string())),
                updates: vec![IrRecordUpdate {
                    field: "x".to_string(),
                    value: sample_value(),
                }],
            },
            IrValue::ListLiteral {
                type_: "List OF Integer".to_string(),
                values: vec![sample_value()],
            },
            IrValue::MapLiteral {
                type_: "Map OF String TO Integer".to_string(),
                entries: vec![(
                    IrValue::Const {
                        type_: "String".to_string(),
                        value: "k".to_string(),
                    },
                    sample_value(),
                )],
            },
            IrValue::MemberAccess {
                target: Box::new(IrValue::Local("p".to_string())),
                member: "x".to_string(),
                type_: "Unknown".to_string(),
            },
            sample_value(),
            IrValue::Unary {
                op: "NOT".to_string(),
                operand: Box::new(IrValue::Local("b".to_string())),
                loc: IrSourceLoc::default(),
                type_: "Unknown".to_string(),
            },
        ];

        let body = vec![
            IrOp::Bind {
                mutable: true,
                name: "a".to_string(),
                type_: "Integer".to_string(),
                value: Some(sample_value()),
                loc: IrSourceLoc::default(),
                explicit_type: false,
            },
            IrOp::Assign {
                name: "a".to_string(),
                value: sample_value(),
                loc: IrSourceLoc::default(),
            },
            IrOp::AssignGlobal {
                name: "g".to_string(),
                value: sample_value(),
                loc: IrSourceLoc::default(),
            },
            IrOp::Eval {
                value: IrValue::Call {
                    target: "g".to_string(),
                    args: every_value.clone(),
                    loc: IrSourceLoc::default(),
                    type_: "Unknown".to_string(),
                },
                loc: IrSourceLoc::default(),
            },
            IrOp::If {
                condition: IrValue::Local("b".to_string()),
                then_body: vec![IrOp::Return {
                    value: Some(IrValue::Local("a".to_string())),
                    loc: IrSourceLoc::default(),
                }],
                else_body: vec![IrOp::Return {
                    value: None,
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrOp::While {
                kind: LoopKind::While,
                condition: IrValue::Local("b".to_string()),
                body: vec![IrOp::Eval {
                    value: IrValue::Local("a".to_string()),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrOp::ForEach {
                name: "item".to_string(),
                type_: "Integer".to_string(),
                iterable: IrValue::Local("list".to_string()),
                body: vec![IrOp::Eval {
                    value: IrValue::Local("item".to_string()),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrOp::Match {
                value: IrValue::Local("s".to_string()),
                cases: vec![
                    IrMatchCase {
                        pattern: IrMatchPattern::Value(IrValue::Local("p".to_string())),
                        guard: Some(IrValue::Local("b".to_string())),
                        body: vec![IrOp::Eval {
                            value: IrValue::Local("p".to_string()),
                            loc: IrSourceLoc::default(),
                        }],
                        loc: IrSourceLoc::default(),
                    },
                    IrMatchCase {
                        pattern: IrMatchPattern::OneOf(vec![
                            IrValue::Local("p".to_string()),
                            IrValue::Local("q".to_string()),
                        ]),
                        guard: None,
                        body: vec![],
                        loc: IrSourceLoc::default(),
                    },
                    IrMatchCase {
                        pattern: IrMatchPattern::Else,
                        guard: None,
                        body: vec![IrOp::Fail {
                            error: IrValue::Local("e".to_string()),
                            loc: IrSourceLoc::default(),
                        }],
                        loc: IrSourceLoc::default(),
                    },
                ],
                loc: IrSourceLoc::default(),
            },
            IrOp::Trap {
                name: "err".to_string(),
                body: vec![IrOp::Eval {
                    value: IrValue::CallResult {
                        target: "toInt".to_string(),
                        args: vec![IrValue::Local("s".to_string())],
                        loc: IrSourceLoc::default(),
                        type_: "Unknown".to_string(),
                    },
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
        ];

        IrProject {
            name: "corpus".to_string(),
            entry: Some(EntryPoint {
                name: "main".to_string(),
                returns: "Integer".to_string(),
                accepts_args: true,
            }),
            bindings: vec![IrBinding {
                name: "g".to_string(),
                visibility: "package".to_string(),
                mutable: false,
                type_: "Integer".to_string(),
                value: Some(sample_value()),
                loc: IrSourceLoc::default(),
                file: String::new(),
                explicit_type: false,
            }],
            types: vec![
                IrType {
                    kind: "type".to_string(),
                    visibility: "export".to_string(),
                    name: "Point".to_string(),
                    fields: vec![
                        IrField {
                            visibility: Some("export".to_string()),
                            name: "x".to_string(),
                            type_: "Integer".to_string(),
                            loc: IrSourceLoc::default(),
                        },
                        IrField {
                            visibility: None,
                            name: "y".to_string(),
                            type_: "Integer".to_string(),
                            loc: IrSourceLoc::default(),
                        },
                    ],
                    includes: vec![],
                    variants: vec![],
                    members: vec![],
                    loc: IrSourceLoc::default(),
                    file: "src/main.mfb".to_string(),
                },
                IrType {
                    kind: "union".to_string(),
                    visibility: "export".to_string(),
                    name: "Shape".to_string(),
                    fields: vec![],
                    includes: vec!["Base".to_string()],
                    variants: vec![IrVariant {
                        name: "Point".to_string(),
                        fields: vec![IrField {
                            visibility: None,
                            name: "x".to_string(),
                            type_: "Integer".to_string(),
                            loc: IrSourceLoc::default(),
                        }],
                        loc: IrSourceLoc::default(),
                    }],
                    members: vec![],
                    loc: IrSourceLoc::default(),
                    file: "src/main.mfb".to_string(),
                },
                IrType {
                    kind: "enum".to_string(),
                    visibility: "private".to_string(),
                    name: "Color".to_string(),
                    fields: vec![],
                    includes: vec![],
                    variants: vec![],
                    members: vec![
                        IrEnumMember {
                            name: "Red".to_string(),
                        },
                        IrEnumMember {
                            name: "Green".to_string(),
                        },
                    ],
                    loc: IrSourceLoc::default(),
                    file: "src/main.mfb".to_string(),
                },
            ],
            functions: vec![IrFunction {
                name: "main".to_string(),
                visibility: "export".to_string(),
                kind: "function".to_string(),
                isolated: false,
                params: vec![
                    IrParam {
                        name: "x".to_string(),
                        type_: "Integer".to_string(),
                        default: None,
                        loc: IrSourceLoc::default(),
                    },
                    IrParam {
                        name: "y".to_string(),
                        type_: "Integer".to_string(),
                        default: Some(IrValue::Const {
                            type_: "Integer".to_string(),
                            value: "0".to_string(),
                        }),
                        loc: IrSourceLoc::default(),
                    },
                ],
                returns: "Integer".to_string(),
                body,
                file: "src/main.mfb".to_string(),
                resource_owners: HashMap::new(),
                loc: IrSourceLoc::default(),
            }],
            native_resources: vec![],
            link_functions: vec![],
            link_aliases: vec![],
            docs: ProjectDocs::default(),
        }
    }

    #[test]
    fn binary_repr_round_trip_is_identity() {
        let project = corpus_project();
        let bytes = encode_binary_repr(&project);
        let decoded = decode_binary_repr(&bytes).expect("decode");
        // The JSON projection is a faithful view of every field; comparing it
        // proves the decode reconstructed the project exactly.
        assert_eq!(project.to_json(), decoded.to_json());
        // Re-encoding the decoded project must be byte-identical.
        let bytes2 = encode_binary_repr(&decoded);
        assert_eq!(bytes, bytes2);
    }

    #[test]
    fn binary_repr_rejects_bad_magic() {
        let mut bytes = encode_binary_repr(&corpus_project());
        bytes[0] = b'X';
        assert!(decode_binary_repr(&bytes).is_err());
    }

    #[test]
    fn binary_repr_rejects_bad_version() {
        let mut bytes = encode_binary_repr(&corpus_project());
        bytes[4] = 0xFF;
        bytes[5] = 0xFF;
        assert!(decode_binary_repr(&bytes).is_err());
    }

    fn fn_named(name: &str, body: Vec<IrOp>) -> IrFunction {
        IrFunction {
            name: name.to_string(),
            visibility: "export".to_string(),
            kind: "function".to_string(),
            isolated: false,
            params: vec![],
            returns: "Integer".to_string(),
            body,
            file: "src/main.mfb".to_string(),
            resource_owners: HashMap::new(),
            loc: IrSourceLoc::default(),
        }
    }

    fn project_named(name: &str, functions: Vec<IrFunction>) -> IrProject {
        IrProject {
            name: name.to_string(),
            entry: None,
            bindings: vec![],
            types: vec![],
            functions,
            native_resources: vec![],
            link_functions: vec![],
            link_aliases: vec![],
            docs: ProjectDocs::default(),
        }
    }

    // The identity prefix `<id>.package.symbol` must be applied consistently to
    // a package's definitions and to every external reference, so the consumer's
    // `package.symbol` call resolves to the merged, identity-prefixed definition.
    #[test]
    fn package_identity_prefix_is_applied_consistently() {
        // Package `pkg`: `f` calls its own `g`.
        let mut pkg = project_named(
            "pkg",
            vec![
                fn_named(
                    "f",
                    vec![IrOp::Eval {
                        value: IrValue::Call {
                            target: "g".to_string(),
                            args: vec![],
                            loc: IrSourceLoc::default(),
                            type_: "Unknown".to_string(),
                        },
                        loc: IrSourceLoc::default(),
                    }],
                ),
                fn_named("g", vec![]),
            ],
        );

        // Names the consumer references, captured before the rename.
        let (ref_fns, ref_globals) = package_qualified_reference_names(&pkg);
        assert!(ref_fns.contains("pkg.f"));
        assert!(ref_fns.contains("pkg.g"));

        let id = "abcd1234";
        prefix_package_symbols(&mut pkg, id);

        // Definitions carry the full `<id>.package.symbol` prefix...
        assert_eq!(pkg.functions[0].name, "abcd1234.pkg.f");
        assert_eq!(pkg.functions[1].name, "abcd1234.pkg.g");
        // ...and the package's own internal reference was rewritten to match.
        match &pkg.functions[0].body[0] {
            IrOp::Eval {
                value: IrValue::Call { target, .. },
                ..
            } => assert_eq!(target, "abcd1234.pkg.g"),
            _ => panic!("expected an Eval(Call) op"),
        }

        // A consumer that calls `pkg.f` has that reference rewritten to the
        // identity-prefixed definition name.
        let mut consumer = project_named(
            "app",
            vec![fn_named(
                "main",
                vec![IrOp::Eval {
                    value: IrValue::Call {
                        target: "pkg.f".to_string(),
                        args: vec![],
                        loc: IrSourceLoc::default(),
                        type_: "Unknown".to_string(),
                    },
                    loc: IrSourceLoc::default(),
                }],
            )],
        );
        apply_package_identity(&mut consumer, &ref_fns, &ref_globals, id);
        match &consumer.functions[0].body[0] {
            IrOp::Eval {
                value: IrValue::Call { target, .. },
                ..
            } => assert_eq!(target, "abcd1234.pkg.f"),
            _ => panic!("expected an Eval(Call) op"),
        }
    }

    // Decoded package IR is verified against the package-format invariants
    // before it is merged; these exercise the PACKAGE_BINARY_REPRESENTATION_VERIFY_* diagnostics.
    #[test]
    fn verify_package_rejects_duplicate_function() {
        let pir = project_named("pkg", vec![fn_named("f", vec![]), fn_named("f", vec![])]);
        let err = verify_package(&pir).expect_err("duplicate function must be rejected");
        assert!(
            err.contains("PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE"),
            "{err}"
        );
    }

    #[test]
    fn verify_package_rejects_unnamed_function() {
        let pir = project_named("pkg", vec![fn_named("", vec![])]);
        let err = verify_package(&pir).expect_err("unnamed function must be rejected");
        assert!(
            err.contains("PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE"),
            "{err}"
        );
    }

    #[test]
    fn verify_package_rejects_empty_match() {
        let body = vec![IrOp::Match {
            value: IrValue::Local("x".to_string()),
            cases: vec![],
            loc: IrSourceLoc::default(),
        }];
        let pir = project_named("pkg", vec![fn_named("f", body)]);
        let err = verify_package(&pir).expect_err("non-exhaustive MATCH must be rejected");
        assert!(
            err.contains("PACKAGE_BINARY_REPRESENTATION_VERIFY_MATCH"),
            "{err}"
        );
    }
}
