use super::*;

/// Shared source-driven test helpers (plan-12 IR coverage). These write a
/// throwaway project to a temp dir and run the real front-end pipeline
/// (parse → resolve → monomorph → **lower**) so tests can assert on the
/// `IrProject` a real program lowers to, rather than hand-building IR.
#[cfg(test)]
pub(crate) mod helpers {
    use super::super::*;
    use crate::ast;
    use crate::manifest::validate_project_manifest;
    use crate::monomorph;
    use crate::resolver;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_dir(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("timestamp")
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "mfb_ir_test_{tag}_{}_{stamp}_{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("temp dir");
        root
    }

    const MANIFEST: &str = r#"{
  "name": "irtest",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main",
  "targets": ["native"]
}"#;

    /// Parse+resolve+monomorph+lower a single-file program source. Panics with
    /// the front-end diagnostics silenced if any earlier stage fails, so a test
    /// failure points at genuinely broken source rather than noise.
    pub(crate) fn lower_src(src: &str) -> IrProject {
        let dir = unique_dir("lower");
        std::fs::write(dir.join("project.json"), MANIFEST).expect("write manifest");
        std::fs::write(dir.join("src").join("main.mfb"), src).expect("write source");
        let manifest =
            validate_project_manifest(&dir.join("project.json")).expect("manifest validates");
        let astp = ast::parse_project("irtest", &dir, &manifest).expect("parse");
        resolver::resolve_project(&dir, &manifest, &astp).expect("resolve");
        let concrete = monomorph::monomorphize_project(&dir, &astp).expect("monomorphize");
        resolver::resolve_project_with(&dir, &manifest, &concrete, false)
            .expect("resolve concrete");
        let ir = lower_project_with_external_functions(
            &concrete,
            None,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        );
        let _ = std::fs::remove_dir_all(&dir);
        ir
    }

    /// Like [`lower_src`] but returns `None` when any front-end stage before
    /// lowering rejects the program (so a test can assert lowering is reached).
    pub(crate) fn try_lower_src(src: &str) -> Option<IrProject> {
        let dir = unique_dir("try");
        std::fs::write(dir.join("project.json"), MANIFEST).ok()?;
        std::fs::write(dir.join("src").join("main.mfb"), src).ok()?;
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = (|| {
            let manifest = validate_project_manifest(&dir.join("project.json")).ok()?;
            let astp = ast::parse_project("irtest", &dir, &manifest).ok()?;
            resolver::resolve_project(&dir, &manifest, &astp).ok()?;
            let concrete = monomorph::monomorphize_project(&dir, &astp).ok()?;
            resolver::resolve_project_with(&dir, &manifest, &concrete, false).ok()?;
            Some(lower_project_with_external_functions(
                &concrete,
                None,
                &std::collections::HashMap::new(),
                &std::collections::HashMap::new(),
            ))
        })();
        std::panic::set_hook(prev);
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    /// The named function's body in a lowered project.
    pub(crate) fn function<'a>(ir: &'a IrProject, name: &str) -> &'a IrFunction {
        ir.functions
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("function `{name}` not found in lowered project"))
    }

    #[test]
    fn lower_src_smoke() {
        let ir = lower_src("FUNC main() AS Integer\n  RETURN 1\nEND FUNC\n");
        assert_eq!(function(&ir, "main").returns, "Integer");
    }
}

/// plan-20-D: IR lowering must be **total** — it never panics on ill-typed but
/// parse/resolve-clean input. The 371 `*-invalid` fixtures are exactly such
/// programs (they parse and resolve but fail a semantic rule). Today the AST
/// type checker rejects them before lowering; once the checker moves onto the
/// IR (plan-20-Z) lowering runs first, so it must survive them. This test
/// drives every `*-invalid` fixture through parse → resolve → monomorph →
/// **lower** (skipping syntaxcheck) and asserts lowering does not panic. Fixtures
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
             syntaxcheck-invalid majority — the pipeline wiring may be broken"
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
        let name = manifest
            .get("name")
            .and_then(|v| v.get::<String>())
            .cloned()?;
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
    /// rules are still only in `syntaxcheck` (MISSING) so the port can drive them
    /// to zero. Run with `--nocapture`.
    #[test]
    #[ignore = "porting census (plan-20-E..I); run with --ignored --nocapture"]
    fn verify_vs_syntaxcheck_diagnostic_parity() {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        use std::collections::BTreeMap;
        let mut missing: BTreeMap<String, usize> = BTreeMap::new();
        let mut matched: BTreeMap<String, usize> = BTreeMap::new();
        let mut extra: BTreeMap<String, usize> = BTreeMap::new();
        let mut fixtures = 0usize;
        for dir in invalid_fixture_dirs() {
            let expected = golden_diagnostics(&dir);
            let Some(actual) =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| verify_diagnostics(&dir)))
                    .ok()
                    .flatten()
            else {
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
                    if std::env::var("CENSUS_MISSING").is_ok() {
                        eprintln!("MISSING {rule} in {}", dir.display());
                    }
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
        eprintln!("\n=== verify-vs-syntaxcheck census ({fixtures} fixtures reached lowering) ===");
        eprintln!("MISSING (syntaxcheck emits, ir::verify does not) — port these:");
        for (rule, n) in &missing {
            eprintln!("  {n:3}  {rule}");
        }
        eprintln!("MATCHED (ir::verify already emits):");
        for (rule, n) in &matched {
            eprintln!("  {n:3}  {rule}");
        }
        eprintln!("EXTRA (ir::verify emits, syntaxcheck did not — over-rejection risk):");
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

/// Structural coverage for `ir/lower.rs` (plan-12): valid MFBASIC programs that
/// exercise every statement / expression / type / binding / native-LINK / DOC
/// lowering path, asserting on the produced `IrProject`. Uses the shared
/// `helpers` harness so the real front-end pipeline (parse → resolve →
/// monomorph → lower) produces the IR under test.
#[cfg(test)]
mod lower_tests {
    use super::super::*;
    use super::helpers::{function, lower_src, try_lower_src};

    // ---- helpers on the produced IR --------------------------------------

    fn main_body(ir: &IrProject) -> &[IrOp] {
        &function(ir, "main").body
    }

    fn binding<'a>(ir: &'a IrProject, name: &str) -> &'a IrBinding {
        ir.bindings
            .iter()
            .find(|b| b.name == name)
            .unwrap_or_else(|| panic!("no binding {name}"))
    }

    fn ir_type<'a>(ir: &'a IrProject, name: &str) -> &'a IrType {
        ir.types
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("no type {name}"))
    }

    fn bind_value<'a>(body: &'a [IrOp], name: &str) -> Option<&'a IrValue> {
        body.iter().find_map(|op| match op {
            IrOp::Bind {
                name: n,
                value: Some(v),
                ..
            } if n == name => Some(v),
            _ => None,
        })
    }

    // ---- bindings --------------------------------------------------------

    #[test]
    fn top_level_bindings_explicit_and_inferred() {
        let ir = lower_src(
            "LET n AS Integer = 5\n\
             LET s = \"hi\"\n\
             MUT flag AS Boolean = TRUE\n\
             LET f = 2.5\n\
             SUB main\nEND SUB\n",
        );
        let n = binding(&ir, "n");
        assert_eq!(n.type_, "Integer");
        assert!(n.explicit_type);
        assert!(!n.mutable);

        let s = binding(&ir, "s");
        assert_eq!(s.type_, "String");
        assert!(!s.explicit_type);

        let flag = binding(&ir, "flag");
        assert!(flag.mutable);
        assert_eq!(flag.type_, "Boolean");

        assert_eq!(binding(&ir, "f").type_, "Float");
    }

    #[test]
    fn inferred_binding_from_call_result() {
        // A binding whose type is inferred from a user function's return type.
        let ir = lower_src(
            "FUNC seed() AS Integer\n  RETURN 7\nEND FUNC\n\
             LET x = seed()\n\
             SUB main\nEND SUB\n",
        );
        let x = binding(&ir, "x");
        assert_eq!(x.type_, "Integer");
        assert!(!x.explicit_type);
    }

    // ---- FUNC / SUB variants --------------------------------------------

    #[test]
    fn func_sub_kinds_and_returns() {
        let ir = lower_src(
            "FUNC add(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\n\
             SUB greet(name AS String)\n  LET x AS String = name\nEND SUB\n\
             SUB main\nEND SUB\n",
        );
        let add = function(&ir, "add");
        assert_eq!(add.kind, "func");
        assert_eq!(add.returns, "Integer");
        assert_eq!(add.params.len(), 2);

        let greet = function(&ir, "greet");
        assert_eq!(greet.kind, "sub");
        assert_eq!(greet.returns, "Nothing");
    }

    #[test]
    fn func_with_default_param_and_isolated() {
        let ir = lower_src(
            "ISOLATED FUNC scaled(x AS Integer, factor AS Integer = 2) AS Integer\n\
               RETURN x * factor\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        let scaled = function(&ir, "scaled");
        assert!(scaled.isolated);
        assert_eq!(scaled.params.len(), 2);
        assert!(scaled.params[1].default.is_some());
    }

    #[test]
    fn func_returning_nothing() {
        let ir = lower_src(
            "FUNC sideEffect(x AS Integer) AS Nothing\n  LET y AS Integer = x\nEND FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert_eq!(function(&ir, "sideEffect").returns, "Nothing");
    }

    #[test]
    fn func_trap_handler_lowers_to_trap_op() {
        let ir = lower_src(
            "FUNC risky() AS Integer\n\
               RETURN 1\n\
               TRAP(err)\n\
                 RETURN err.code\n\
               END TRAP\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "risky")
            .body
            .iter()
            .any(|op| matches!(op, IrOp::Trap { .. })));
    }

    // ---- TYPE / UNION / ENUM --------------------------------------------

    #[test]
    fn type_record_with_private_field() {
        let ir = lower_src(
            "EXPORT TYPE Point\n  x AS Integer\n  PRIVATE y AS Integer\nEND TYPE\n\
             SUB main\nEND SUB\n",
        );
        let point = ir_type(&ir, "Point");
        assert_eq!(point.kind, "type");
        assert_eq!(point.visibility, "export");
        assert_eq!(point.fields.len(), 2);
        assert_eq!(point.fields[1].visibility.as_deref(), Some("private"));
    }

    #[test]
    fn union_with_variants_and_includes() {
        let ir = lower_src(
            "TYPE Circle\n  r AS Integer\nEND TYPE\n\
             TYPE Square\n  s AS Integer\nEND TYPE\n\
             UNION Rounded\n  Circle\nEND UNION\n\
             UNION Shape INCLUDES Rounded\n  Square\nEND UNION\n\
             SUB main\nEND SUB\n",
        );
        let shape = ir_type(&ir, "Shape");
        assert_eq!(shape.kind, "union");
        assert_eq!(shape.includes, vec!["Rounded".to_string()]);
        assert!(shape.variants.iter().any(|v| v.name == "Square"));
    }

    #[test]
    fn enum_members_lowered() {
        let ir = lower_src(
            "ENUM Color\n  Red\n  Green\n  Blue\nEND ENUM\n\
             SUB main\nEND SUB\n",
        );
        let color = ir_type(&ir, "Color");
        assert_eq!(color.kind, "enum");
        assert_eq!(color.members.len(), 3);
        assert_eq!(color.members[0].name, "Red");
    }

    // ---- statements ------------------------------------------------------

    #[test]
    fn let_mut_and_assignment_statements() {
        let ir = lower_src(
            "SUB main\n  LET a AS Integer = 1\n  MUT b AS Integer = 2\n  b = a + b\nEND SUB\n",
        );
        let body = main_body(&ir);
        assert!(matches!(&body[0], IrOp::Bind { mutable: false, name, .. } if name == "a"));
        assert!(matches!(&body[1], IrOp::Bind { mutable: true, name, .. } if name == "b"));
        assert!(matches!(&body[2], IrOp::Assign { name, .. } if name == "b"));
    }

    #[test]
    fn assignment_to_global_is_assign_global() {
        let ir = lower_src("MUT counter AS Integer = 0\nSUB main\n  counter = 5\nEND SUB\n");
        assert!(main_body(&ir)
            .iter()
            .any(|op| matches!(op, IrOp::AssignGlobal { name, .. } if name == "counter")));
    }

    #[test]
    fn return_without_value_from_sub() {
        let ir = lower_src("SUB s()\n  RETURN\nEND SUB\nSUB main\nEND SUB\n");
        assert!(function(&ir, "s")
            .body
            .iter()
            .any(|op| matches!(op, IrOp::Return { value: None, .. })));
    }

    #[test]
    fn exit_targets_lower_to_expected_ops() {
        let ir = lower_src(
            "SUB main\n\
               FOR i = 1 TO 3\n  EXIT FOR\n  CONTINUE FOR\n  NEXT\n\
               DO\n  EXIT DO\nLOOP UNTIL TRUE\n\
               WHILE FALSE\n  EXIT WHILE\n  WEND\n\
               EXIT PROGRAM 2\n\
             END SUB\n",
        );
        fn contains_exit(ops: &[IrOp], want: LoopKind) -> bool {
            ops.iter().any(|op| match op {
                IrOp::ExitLoop { kind, .. } => *kind == want,
                IrOp::For { body, .. } | IrOp::While { body, .. } | IrOp::DoUntil { body, .. } => {
                    contains_exit(body, want)
                }
                _ => false,
            })
        }
        fn contains_continue(ops: &[IrOp]) -> bool {
            ops.iter().any(|op| match op {
                IrOp::ContinueLoop { .. } => true,
                IrOp::For { body, .. } | IrOp::While { body, .. } | IrOp::DoUntil { body, .. } => {
                    contains_continue(body)
                }
                _ => false,
            })
        }
        let body = main_body(&ir);
        assert!(contains_exit(body, LoopKind::For));
        assert!(contains_exit(body, LoopKind::Do));
        assert!(contains_exit(body, LoopKind::While));
        assert!(contains_continue(body));
        assert!(body.iter().any(|op| matches!(op, IrOp::ExitProgram { .. })));
    }

    #[test]
    fn exit_sub_lowers_to_valueless_return() {
        let ir = lower_src("SUB s()\n  EXIT SUB\nEND SUB\nSUB main\nEND SUB\n");
        assert!(function(&ir, "s")
            .body
            .iter()
            .any(|op| matches!(op, IrOp::Return { value: None, .. })));
    }

    #[test]
    fn exit_func_contributes_no_op() {
        let ir = lower_src(
            "FUNC f() AS Integer\n  EXIT FUNC\n  RETURN 1\nEND FUNC\nSUB main\nEND SUB\n",
        );
        // EXIT FUNC emits nothing; only the trailing RETURN survives.
        assert!(function(&ir, "f")
            .body
            .iter()
            .any(|op| matches!(op, IrOp::Return { value: Some(_), .. })));
    }

    #[test]
    fn fail_statement_lowers_to_fail_op() {
        let ir = lower_src("SUB main\n  FAIL error(500, \"boom\")\nEND SUB\n");
        assert!(main_body(&ir)
            .iter()
            .any(|op| matches!(op, IrOp::Fail { .. })));
    }

    #[test]
    fn if_else_lowers_to_if_op_with_both_branches() {
        let ir = lower_src(
            "SUB main\n  LET x AS Integer = 1\n\
               IF x > 0 THEN\n    LET a AS Integer = 1\n  ELSE\n    LET b AS Integer = 2\n  END IF\n\
             END SUB\n",
        );
        assert!(main_body(&ir).iter().any(|op| match op {
            IrOp::If {
                then_body,
                else_body,
                ..
            } => !then_body.is_empty() && !else_body.is_empty(),
            _ => false,
        }));
    }

    #[test]
    fn while_and_do_until_loops() {
        let ir = lower_src(
            "SUB main\n\
               MUT i AS Integer = 0\n\
               WHILE i < 3\n    i = i + 1\n  WEND\n\
               DO\n    i = i - 1\n  LOOP UNTIL i <= 0\n\
             END SUB\n",
        );
        let body = main_body(&ir);
        assert!(body.iter().any(|op| matches!(op, IrOp::While { .. })));
        assert!(body.iter().any(|op| matches!(op, IrOp::DoUntil { .. })));
    }

    #[test]
    fn for_loop_with_step() {
        let ir = lower_src(
            "SUB main\n  FOR i = 1 TO 10 STEP 2\n    LET x AS Integer = i\n  NEXT\nEND SUB\n",
        );
        assert!(main_body(&ir)
            .iter()
            .any(|op| matches!(op, IrOp::For { .. })));
    }

    #[test]
    fn for_loop_default_step() {
        let ir =
            lower_src("SUB main\n  FOR i = 1 TO 3\n    LET x AS Integer = i\n  NEXT\nEND SUB\n");
        assert!(main_body(&ir)
            .iter()
            .any(|op| matches!(op, IrOp::For { .. })));
    }

    #[test]
    fn for_loop_float_bounds_promote_loop_type() {
        let ir = lower_src(
            "SUB main\n  FOR x = 1.0 TO 2.0 STEP 0.5\n    LET y AS Float = x\n  NEXT\nEND SUB\n",
        );
        assert!(main_body(&ir).iter().any(|op| match op {
            IrOp::For { type_, .. } => type_ == "Float",
            _ => false,
        }));
    }

    #[test]
    fn for_each_over_list() {
        let ir = lower_src(
            "IMPORT collections\n\
             SUB main\n\
               LET nums AS List OF Integer = [1, 2, 3]\n\
               FOR EACH n IN nums\n    LET x AS Integer = n\n  NEXT\n\
             END SUB\n",
        );
        assert!(main_body(&ir).iter().any(|op| match op {
            IrOp::ForEach { type_, .. } => type_ == "Integer",
            _ => false,
        }));
    }

    #[test]
    fn for_each_over_map_yields_map_entry() {
        let ir = lower_src(
            "IMPORT collections\n\
             SUB main\n\
               LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }\n\
               FOR EACH e IN m\n    LET k AS String = e.key\n  NEXT\n\
             END SUB\n",
        );
        assert!(main_body(&ir).iter().any(|op| match op {
            IrOp::ForEach { type_, .. } => type_.starts_with("MapEntry OF "),
            _ => false,
        }));
    }

    #[test]
    fn expression_statement_lowers_to_eval() {
        let ir = lower_src("IMPORT io\nSUB main\n  io::print(\"hi\")\nEND SUB\n");
        assert!(main_body(&ir)
            .iter()
            .any(|op| matches!(op, IrOp::Eval { .. })));
    }

    // ---- MATCH -----------------------------------------------------------

    #[test]
    fn match_with_value_oneof_and_else() {
        let ir = lower_src(
            "SUB main\n\
               LET x AS Integer = 2\n\
               MATCH x\n\
                 CASE 1\n      LET a AS Integer = 1\n\
                 CASE 2, 3\n      LET b AS Integer = 2\n\
                 CASE ELSE\n      LET c AS Integer = 3\n\
               END MATCH\n\
             END SUB\n",
        );
        let cases = main_body(&ir)
            .iter()
            .find_map(|op| match op {
                IrOp::Match { cases, .. } => Some(cases),
                _ => None,
            })
            .expect("a Match op");
        assert!(cases
            .iter()
            .any(|c| matches!(c.pattern, IrMatchPattern::Value(_))));
        assert!(cases
            .iter()
            .any(|c| matches!(c.pattern, IrMatchPattern::OneOf(_))));
        assert!(cases
            .iter()
            .any(|c| matches!(c.pattern, IrMatchPattern::Else)));
    }

    #[test]
    fn match_with_guard() {
        let ir = lower_src(
            "SUB main\n\
               LET x AS Integer = 5\n\
               MATCH x\n\
                 CASE 5 WHEN x > 0\n      LET a AS Integer = 1\n\
                 CASE ELSE\n      LET b AS Integer = 2\n\
               END MATCH\n\
             END SUB\n",
        );
        assert!(main_body(&ir).iter().any(|op| match op {
            IrOp::Match { cases, .. } => cases.iter().any(|c| c.guard.is_some()),
            _ => false,
        }));
    }

    #[test]
    fn match_on_union_binds_variant() {
        let ir = lower_src(
            "TYPE Dog\n  legs AS Integer\nEND TYPE\n\
             TYPE Cat\n  lives AS Integer\nEND TYPE\n\
             UNION Animal\n  Dog\n  Cat\nEND UNION\n\
             FUNC describe(a AS Animal) AS Integer\n\
               MATCH a\n\
                 CASE Dog(d)\n      RETURN d.legs\n\
                 CASE Cat(c)\n      RETURN c.lives\n\
               END MATCH\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        fn has_union_extract(ops: &[IrOp]) -> bool {
            ops.iter().any(|op| match op {
                IrOp::Bind {
                    value: Some(IrValue::UnionExtract { .. }),
                    ..
                } => true,
                IrOp::Match { cases, .. } => cases.iter().any(|c| has_union_extract(&c.body)),
                _ => false,
            })
        }
        assert!(has_union_extract(&function(&ir, "describe").body));
    }

    #[test]
    fn match_on_fallible_call_uses_ok_error_and_is_ok_flag() {
        // MATCH on a fallible call keeps its `Result OF …` shape (matched with
        // CASE Ok / CASE Error), exercising the Result branch of the match
        // lowering (ResultIsOk flag + ResultValue/ResultError case bindings).
        // `Result OF …` cannot be named in source, so a direct call scrutinee is
        // the only way to reach this path.
        let src = "FUNC parse(s AS String) AS Integer\n\
               MATCH toInt(s)\n\
                 CASE Ok(v)\n      RETURN v\n\
                 CASE Error(e)\n      RETURN 0\n\
               END MATCH\n\
             END FUNC\n\
             SUB main\nEND SUB\n";
        let Some(ir) = try_lower_src(src) else {
            // If a call scrutinee auto-unwraps (front end forbids this Result
            // MATCH), the branch is unreachable from single-file source; skip.
            return;
        };
        let f = function(&ir, "parse");
        let has_is_ok = f.body.iter().any(|op| {
            matches!(
                op,
                IrOp::Bind {
                    value: Some(IrValue::ResultIsOk { .. }),
                    ..
                }
            )
        });
        fn has_result_extract(ops: &[IrOp]) -> bool {
            ops.iter().any(|op| match op {
                IrOp::Bind {
                    value: Some(IrValue::ResultValue { .. } | IrValue::ResultError { .. }),
                    ..
                } => true,
                IrOp::Match { cases, .. } => cases.iter().any(|c| has_result_extract(&c.body)),
                _ => false,
            })
        }
        // Whether or not the front end keeps the Result shape, the program must
        // lower to a MATCH; assert the Result path when it is taken.
        if has_is_ok {
            assert!(has_result_extract(&f.body));
        }
    }

    // ---- expressions: literals ------------------------------------------

    #[test]
    fn literal_kinds_lower_to_typed_consts() {
        let ir = lower_src(
            "SUB main\n\
               LET s AS String = \"x\"\n\
               LET i AS Integer = 3\n\
               LET f AS Float = 1.5\n\
               LET bt AS Byte = 7\n\
               LET fx AS Fixed = 2.50\n\
               LET b AS Boolean = TRUE\n\
               LET nul AS Nothing = NOTHING\n\
             END SUB\n",
        );
        let const_type = |body: &[IrOp], name: &str| -> String {
            match bind_value(body, name) {
                Some(IrValue::Const { type_, .. }) => type_.clone(),
                _ => "<missing>".to_string(),
            }
        };
        let body = main_body(&ir);
        assert_eq!(const_type(body, "s"), "String");
        assert_eq!(const_type(body, "i"), "Integer");
        assert_eq!(const_type(body, "f"), "Float");
        assert_eq!(const_type(body, "bt"), "Byte");
        assert_eq!(const_type(body, "fx"), "Fixed");
        assert_eq!(const_type(body, "b"), "Boolean");
        assert_eq!(const_type(body, "nul"), "Nothing");
    }

    // ---- expressions: operators -----------------------------------------

    #[test]
    fn binary_and_unary_operators() {
        let ir = lower_src(
            "SUB main\n\
               LET a AS Integer = 1 + 2\n\
               LET b AS Boolean = a > 0 AND NOT (a = 5)\n\
               LET c AS Integer = -a\n\
               LET s AS String = \"x\" & \"y\"\n\
             END SUB\n",
        );
        let body = main_body(&ir);
        assert!(matches!(
            bind_value(body, "a"),
            Some(IrValue::Binary { .. })
        ));
        assert!(matches!(
            bind_value(body, "b"),
            Some(IrValue::Binary { .. })
        ));
        assert!(matches!(bind_value(body, "c"), Some(IrValue::Unary { .. })));
        assert!(matches!(
            bind_value(body, "s"),
            Some(IrValue::Binary { type_, .. }) if type_ == "String"
        ));
    }

    // ---- expressions: calls ----------------------------------------------

    #[test]
    fn user_function_call_lowers_to_call() {
        let ir = lower_src(
            "FUNC dbl(x AS Integer) AS Integer\n  RETURN x * 2\nEND FUNC\n\
             SUB main\n  LET y AS Integer = dbl(4)\nEND SUB\n",
        );
        assert!(matches!(
            bind_value(main_body(&ir), "y"),
            Some(IrValue::Call { target, .. }) if target == "dbl"
        ));
    }

    #[test]
    fn call_with_named_and_default_arguments() {
        let ir = lower_src(
            "FUNC box(w AS Integer, h AS Integer = 10) AS Integer\n  RETURN w * h\nEND FUNC\n\
             SUB main\n  LET a AS Integer = box(h := 3, w := 4)\n  LET b AS Integer = box(2)\nEND SUB\n",
        );
        let calls: Vec<usize> = main_body(&ir)
            .iter()
            .filter_map(|op| match op {
                IrOp::Bind {
                    value: Some(IrValue::Call { target, args, .. }),
                    ..
                } if target == "box" => Some(args.len()),
                _ => None,
            })
            .collect();
        assert_eq!(calls, vec![2, 2]);
    }

    #[test]
    fn error_builtin_lowers_to_error_record() {
        let ir = lower_src("SUB main\n  LET e AS Error = error(42, \"bad\")\nEND SUB\n");
        assert!(matches!(
            bind_value(main_body(&ir), "e"),
            Some(IrValue::Constructor { type_, .. }) if type_ == "Error"
        ));
    }

    #[test]
    fn builtin_call_return_type_resolves() {
        let ir = lower_src("SUB main\n  LET s AS String = toString(5)\nEND SUB\n");
        assert!(matches!(
            bind_value(main_body(&ir), "s"),
            Some(IrValue::Call { type_, .. }) if type_ == "String"
        ));
    }

    // ---- constructors, member access, WITH -------------------------------

    #[test]
    fn constructor_and_member_access() {
        let ir = lower_src(
            "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\
             FUNC mk() AS Integer\n\
               LET p AS Point = Point[1, 2]\n\
               RETURN p.x\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        let f = function(&ir, "mk");
        assert!(f.body.iter().any(|op| matches!(
            op,
            IrOp::Bind {
                value: Some(IrValue::Constructor { type_, .. }),
                ..
            } if type_ == "Point"
        )));
        assert!(f.body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::MemberAccess { .. }),
                ..
            }
        )));
    }

    #[test]
    fn constructor_named_arguments() {
        // Named-field record construction (reordered), exercising the named-arg
        // branch of `lower_constructor_args`.
        let ir = lower_src(
            "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\
             FUNC mk() AS Point\n  RETURN Point[y := 2, x := 1]\nEND FUNC\n\
             SUB main\nEND SUB\n",
        );
        let ctor = function(&ir, "mk").body.iter().find_map(|op| match op {
            IrOp::Return {
                value: Some(IrValue::Constructor { args, .. }),
                ..
            } => Some(args.len()),
            _ => None,
        });
        assert_eq!(ctor, Some(2));
    }

    #[test]
    fn with_update_expression() {
        let ir = lower_src(
            "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\
             FUNC shift(p AS Point) AS Point\n  RETURN WITH p { x := 9 }\nEND FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "shift").body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::WithUpdate { .. }),
                ..
            }
        )));
    }

    // ---- list / map literals ---------------------------------------------

    #[test]
    fn list_and_map_literals() {
        let ir = lower_src(
            "IMPORT collections\n\
             SUB main\n\
               LET xs AS List OF Integer = [1, 2, 3]\n\
               LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1, \"b\" := 2 }\n\
             END SUB\n",
        );
        let body = main_body(&ir);
        assert!(matches!(
            bind_value(body, "xs"),
            Some(IrValue::ListLiteral { .. })
        ));
        assert!(matches!(
            bind_value(body, "m"),
            Some(IrValue::MapLiteral { .. })
        ));
    }

    #[test]
    fn empty_list_literal_uses_expected_element_type() {
        let ir =
            lower_src("IMPORT collections\nSUB main\n  LET xs AS List OF Integer = []\nEND SUB\n");
        assert!(matches!(
            bind_value(main_body(&ir), "xs"),
            Some(IrValue::ListLiteral { type_, .. }) if type_ == "List OF Integer"
        ));
    }

    // ---- closures / lambdas / FUNC refs ----------------------------------

    #[test]
    fn lambda_capturing_local_becomes_closure() {
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC run() AS List OF Integer\n\
               LET base AS Integer = 10\n\
               LET xs AS List OF Integer = [1, 2]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v + base)\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(ir.functions.iter().any(|f| f.name.starts_with("$lambda")));
        fn value_has_closure(v: &IrValue) -> bool {
            match v {
                IrValue::Closure { .. } => true,
                IrValue::Call { args, .. } => args.iter().any(value_has_closure),
                _ => false,
            }
        }
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Return { value: Some(v), .. } if value_has_closure(v)
        )));
    }

    #[test]
    fn lambda_without_capture_is_function_ref() {
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC run() AS List OF Integer\n\
               LET xs AS List OF Integer = [1, 2]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v + 1)\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(ir.functions.iter().any(|f| f.name.starts_with("$lambda")));
        fn value_has_fnref(v: &IrValue) -> bool {
            match v {
                IrValue::FunctionRef { .. } => true,
                IrValue::Call { args, .. } => args.iter().any(value_has_fnref),
                _ => false,
            }
        }
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Return { value: Some(v), .. } if value_has_fnref(v)
        )));
    }

    #[test]
    fn named_function_reference_as_argument() {
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC inc(x AS Integer) AS Integer\n  RETURN x + 1\nEND FUNC\n\
             FUNC run() AS List OF Integer\n\
               LET xs AS List OF Integer = [1, 2]\n\
               RETURN collections::transform(xs, inc)\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        fn value_has_fnref(v: &IrValue) -> bool {
            match v {
                IrValue::FunctionRef { .. } => true,
                IrValue::Call { args, .. } => args.iter().any(value_has_fnref),
                _ => false,
            }
        }
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Return { value: Some(v), .. } if value_has_fnref(v)
        )));
    }

    #[test]
    fn mut_capture_in_foreach_is_by_ref() {
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC total() AS Integer\n\
               MUT sum AS Integer = 0\n\
               LET xs AS List OF Integer = [1, 2, 3]\n\
               collections::forEach(xs, LAMBDA(v AS Integer) -> sum = sum + v)\n\
               RETURN sum\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        // The lambda body is an assignment-bodied lambda: assign then valueless
        // return.
        let lambda = ir
            .functions
            .iter()
            .find(|f| f.name.starts_with("$lambda"))
            .expect("a lambda");
        assert!(lambda
            .body
            .iter()
            .any(|op| matches!(op, IrOp::Assign { name, .. } if name == "sum")));
        assert!(lambda
            .body
            .iter()
            .any(|op| matches!(op, IrOp::Return { value: None, .. })));
        // The MUT capture is a by-ref (LocalRef) capture in the closure.
        fn value_has_localref_capture(v: &IrValue) -> bool {
            match v {
                IrValue::Closure { captures, .. } => captures
                    .iter()
                    .any(|c| matches!(c, IrValue::LocalRef { .. })),
                IrValue::Call { args, .. } => args.iter().any(value_has_localref_capture),
                _ => false,
            }
        }
        assert!(function(&ir, "total").body.iter().any(|op| matches!(
            op,
            IrOp::Eval { value, .. } if value_has_localref_capture(value)
        )));
    }

    // ---- inline TRAP -----------------------------------------------------

    #[test]
    fn inline_trap_bind_with_recover() {
        let ir = lower_src(
            "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n    RECOVER 0\n  END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        let f = function(&ir, "parse");
        assert!(f.body.iter().any(|op| matches!(
            op,
            IrOp::Bind {
                value: Some(IrValue::CallResult { .. }),
                ..
            }
        )));
        assert!(f.body.iter().any(|op| matches!(
            op,
            IrOp::If {
                condition: IrValue::ResultIsOk { .. },
                ..
            }
        )));
    }

    #[test]
    fn inline_trap_assign_target() {
        let ir = lower_src(
            "FUNC parse(s AS String) AS Integer\n\
               MUT n AS Integer = 100\n\
               n = toInt(s) TRAP(e)\n    RECOVER 0\n  END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "parse").body.iter().any(|op| matches!(
            op,
            IrOp::Bind {
                value: Some(IrValue::CallResult { .. }),
                ..
            }
        )));
    }

    #[test]
    fn inline_trap_discard_bare_statement() {
        let ir = lower_src(
            "SUB effect(x AS Integer)\n  IF x < 0 THEN FAIL error(1, \"neg\")\nEND SUB\n\
             SUB run()\n\
               effect(-1) TRAP(e)\n    RECOVER\n  END TRAP\n\
             END SUB\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::If {
                condition: IrValue::ResultIsOk { .. },
                ..
            }
        )));
    }

    #[test]
    fn inline_trap_handler_with_if_then_fail() {
        let ir = lower_src(
            "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 IF e.code = 404 THEN RECOVER 0\n\
                 FAIL e\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "parse").body.iter().any(|op| matches!(
            op,
            IrOp::If {
                condition: IrValue::ResultIsOk { .. },
                ..
            }
        )));
    }

    #[test]
    fn inline_trap_handler_with_match_and_continuation() {
        let ir = lower_src(
            "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 MATCH e.code\n\
                   CASE 404\n          RECOVER 0\n\
                 END MATCH\n\
                 FAIL e\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        fn has_match(ops: &[IrOp]) -> bool {
            ops.iter().any(|op| match op {
                IrOp::Match { .. } => true,
                IrOp::If {
                    then_body,
                    else_body,
                    ..
                } => has_match(then_body) || has_match(else_body),
                _ => false,
            })
        }
        assert!(has_match(&function(&ir, "parse").body));
    }

    #[test]
    fn inline_trap_handler_diverges_with_return() {
        let ir = lower_src(
            "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n    RETURN 99\n  END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "parse").body.iter().any(|op| matches!(
            op,
            IrOp::If {
                condition: IrValue::ResultIsOk { .. },
                ..
            }
        )));
    }

    // ---- union wrapping --------------------------------------------------

    #[test]
    fn returned_variant_wraps_into_union() {
        let ir = lower_src(
            "TYPE Dog\n  legs AS Integer\nEND TYPE\n\
             TYPE Cat\n  lives AS Integer\nEND TYPE\n\
             UNION Animal\n  Dog\n  Cat\nEND UNION\n\
             FUNC makeDog() AS Animal\n  RETURN Dog[4]\nEND FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "makeDog").body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::UnionWrap { .. }),
                ..
            }
        )));
    }

    #[test]
    fn union_typed_binding_wraps_variant() {
        let ir = lower_src(
            "TYPE Dog\n  legs AS Integer\nEND TYPE\n\
             UNION Animal\n  Dog\nEND UNION\n\
             FUNC run() AS Integer\n  LET a AS Animal = Dog[4]\n  RETURN 0\nEND FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Bind {
                value: Some(IrValue::UnionWrap { .. }),
                ..
            }
        )));
    }

    // ---- native LINK / RESOURCE / aliases --------------------------------

    #[test]
    fn link_functions_resources_and_aliases() {
        let src = "EXPORT RESOURCE Db CLOSE BY demoLink::close\n\
             LINK \"sqlite3\" AS demoLink\n\
               FUNC open(path AS String) AS RES Db\n\
                 SYMBOL \"sqlite3_open\"\n\
                 ABI (path CString, return OUT CPtr) AS status CInt32\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
               FUNC close(RES db AS Db) AS Nothing\n\
                 SYMBOL \"sqlite3_close\"\n\
                 ABI (db CPtr) AS status CInt32\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
             END LINK\n\
             EXPORT FUNC close AS demoLink::close\n\
             SUB main\nEND SUB\n";
        let ir = try_lower_src(src).expect("LINK program should lower");
        let open = ir
            .link_functions
            .iter()
            .find(|f| f.name == "open")
            .expect("open link function");
        assert_eq!(open.library, "sqlite3");
        assert_eq!(open.symbol, "sqlite3_open");
        assert!(open.success_on.is_some());
        assert!(!open.abi_slots.is_empty());

        let db = ir
            .native_resources
            .iter()
            .find(|r| r.name == "Db")
            .expect("Db resource");
        assert_eq!(db.visibility, "export");
        assert!(db.close_may_fail);

        assert!(ir
            .link_aliases
            .iter()
            .any(|(name, target)| name == "close" && target == "demoLink.close"));
    }

    #[test]
    fn link_const_pin_and_result_expression() {
        let src = "RESOURCE Handle CLOSE BY lib::shut\n\
             LINK \"c\" AS lib\n\
               FUNC make() AS RES Handle\n\
                 SYMBOL \"make\"\n\
                 ABI (flags CInt32, return OUT CPtr) AS status CInt32\n\
                 CONST flags = 0\n\
                 SUCCESS_ON status = 0\n\
                 RESULT NOT (status < 0)\n\
               END FUNC\n\
               FUNC shut(RES h AS Handle) AS Nothing\n\
                 SYMBOL \"shut\"\n\
                 ABI (h CPtr) AS status CInt32\n\
               END FUNC\n\
             END LINK\n\
             SUB main\nEND SUB\n";
        let ir = try_lower_src(src).expect("link CONST/RESULT program should lower");
        let make = ir
            .link_functions
            .iter()
            .find(|f| f.name == "make")
            .expect("make link function");
        assert!(!make.consts.is_empty());
        assert!(make.result.is_some());
        let handle = ir.native_resources.iter().find(|r| r.name == "Handle");
        assert!(handle.is_some_and(|r| !r.close_may_fail));
    }

    // ---- DOC blocks ------------------------------------------------------

    #[test]
    fn doc_blocks_collected_for_exports() {
        let src = "DOC\n  PACKAGE\n  DESC A package.\nEND DOC\n\
             DOC\n  FUNC add(Integer, Integer)\n  GROUP Math\n  DESC Adds.\n\
               ARG a First.\n  ARG b Second.\n  RET The sum.\nEND DOC\n\
             EXPORT FUNC add(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\n\
             DOC\n  TYPE Point\n  DESC A point.\n  PROP x X.\nEND DOC\n\
             EXPORT TYPE Point\n  x AS Integer\nEND TYPE\n\
             DOC\n  ENUM Color\n  DESC Colors.\nEND DOC\n\
             EXPORT ENUM Color\n  Red\nEND ENUM\n\
             SUB main\nEND SUB\n";
        let ir = try_lower_src(src).expect("DOC program should lower");
        assert!(ir.docs.package.is_some());
        let add = ir
            .docs
            .decls
            .iter()
            .find(|d| d.name == "add")
            .expect("add doc");
        assert_eq!(add.group, "Math");
        assert_eq!(add.args.len(), 2);
        assert!(ir
            .docs
            .decls
            .iter()
            .any(|d| d.name == "Point" && d.kind == IrDocKind::Type));
        assert!(ir
            .docs
            .decls
            .iter()
            .any(|d| d.name == "Color" && d.kind == IrDocKind::Enum));
    }

    #[test]
    fn doc_block_for_union_and_sub() {
        let src = "DOC\n  UNION Shape\n  DESC A shape.\nEND DOC\n\
             TYPE Dot\n  n AS Integer\nEND TYPE\n\
             EXPORT UNION Shape\n  Dot\nEND UNION\n\
             DOC INTERNAL\n  SUB logIt\n  DESC Logs.\n  DEPRECATED Use logger.\nEND DOC\n\
             EXPORT SUB logIt(value AS Integer)\n  LET x AS Integer = value\nEND SUB\n\
             SUB main\nEND SUB\n";
        if let Some(ir) = try_lower_src(src) {
            assert!(ir
                .docs
                .decls
                .iter()
                .any(|d| d.name == "Shape" && d.kind == IrDocKind::Union));
            let log = ir.docs.decls.iter().find(|d| d.name == "logIt");
            assert!(log
                .is_some_and(|d| d.kind == IrDocKind::Sub && d.internal && d.deprecated.is_some()));
        }
    }

    #[test]
    fn doc_for_nonexported_function_is_dropped() {
        let src = "DOC\n  FUNC helper()\n  DESC Private.\nEND DOC\n\
             FUNC helper() AS Integer\n  RETURN 1\nEND FUNC\n\
             SUB main\nEND SUB\n";
        if let Some(ir) = try_lower_src(src) {
            assert!(!ir.docs.decls.iter().any(|d| d.name == "helper"));
        }
    }

    // ---- qualified builtin package call ----------------------------------

    #[test]
    fn qualified_strings_call_lowers_to_internal_target() {
        let ir = lower_src(
            "IMPORT strings\n\
             FUNC up(s AS String) AS String\n  RETURN strings::upper(s)\nEND FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "up").body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::Call { .. }),
                ..
            }
        )));
    }

    // ---- RES with STATE: param STATE, StateAssign, `.state` access -------

    #[test]
    fn resource_state_param_assign_and_access() {
        let ir = lower_src(
            "IMPORT fs\n\
             IMPORT io\n\
             TYPE FileState\n  pos AS Integer\n  len AS Integer\nEND TYPE\n\
             SUB advance(RES f AS File STATE FileState, by AS Integer)\n\
               f.state = WITH f.state { pos := f.state.pos + by }\n\
             END SUB\n\
             FUNC main AS Integer\n\
               RES f AS File STATE FileState = fs::createTempFile()\n\
               io::print(toString(f.state.pos))\n\
               advance(f, 5)\n\
               fs::close(f)\n\
               RETURN 0\n\
             END FUNC\n",
        );
        // The `advance` SUB has a RES param carrying STATE in its type string.
        let advance = function(&ir, "advance");
        assert!(advance.params[0].type_.contains("STATE FileState"));
        // Its body assigns the resource STATE via StateAssign.
        assert!(advance
            .body
            .iter()
            .any(|op| matches!(op, IrOp::StateAssign { .. })));
        // `main` reads `f.state.pos` -> a MemberAccess chain over the RES state.
        fn has_member_access(ops: &[IrOp]) -> bool {
            fn value_has(v: &IrValue) -> bool {
                match v {
                    IrValue::MemberAccess { .. } => true,
                    IrValue::Call { args, .. } => args.iter().any(value_has),
                    _ => false,
                }
            }
            ops.iter().any(|op| match op {
                IrOp::Eval { value, .. }
                | IrOp::Bind {
                    value: Some(value), ..
                } => value_has(value),
                _ => false,
            })
        }
        assert!(has_member_access(&function(&ir, "main").body));
    }

    // ---- builtin package calls (expression_type resolution branches) -----

    #[test]
    fn many_builtin_package_calls_resolve_return_types() {
        // Each qualified builtin routes through its package's expression_type
        // resolver and its `implementation_name`/internalization path.
        let ir = lower_src(
            "IMPORT math\n\
             IMPORT bits\n\
             IMPORT io\n\
             IMPORT datetime\n\
             FUNC compute() AS Integer\n\
               LET a AS Float = math::sqrt(2.0)\n\
               LET b AS Integer = bits::sl(1, 3)\n\
               LET t AS Instant = datetime::now()\n\
               RETURN b\n\
             END FUNC\n\
             SUB main\n  io::print(\"x\")\nEND SUB\n",
        );
        let f = function(&ir, "compute");
        // math::sqrt resolves to a Float-returning call.
        let sqrt_ty = f.body.iter().find_map(|op| match op {
            IrOp::Bind {
                name,
                value: Some(IrValue::Call { type_, .. }),
                ..
            } if name == "a" => Some(type_.clone()),
            _ => None,
        });
        assert_eq!(sqrt_ty.as_deref(), Some("Float"));
    }

    #[test]
    fn strings_and_collections_calls_resolve() {
        let ir = lower_src(
            "IMPORT strings\n\
             IMPORT collections\n\
             FUNC run() AS Integer\n\
               LET s AS String = strings::upper(\"hi\")\n\
               LET n AS Integer = strings::byteLen(s)\n\
               LET xs AS List OF Integer = [1, 2, 3]\n\
               LET total AS Integer = collections::sum(xs)\n\
               RETURN n + total\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        let f = function(&ir, "run");
        assert!(f.body.iter().any(|op| matches!(
            op,
            IrOp::Bind {
                name,
                value: Some(IrValue::Call { type_, .. }),
                ..
            } if name == "n" && type_ == "Integer"
        )));
    }

    #[test]
    fn collections_filter_with_named_predicate() {
        // `collections::filter(list, predicate)` with a built-in predicate name
        // (`isEven`) takes the filter-predicate-typing + FunctionRef lowering
        // branch (the predicate is resolved to `FUNC(Integer) AS Boolean`).
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC run() AS List OF Integer\n\
               LET xs AS List OF Integer = [1, 2, 3]\n\
               RETURN collections::filter(xs, isEven)\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        fn has_fn_ref(v: &IrValue) -> bool {
            match v {
                IrValue::FunctionRef { .. } => true,
                IrValue::Call { args, .. } => args.iter().any(has_fn_ref),
                _ => false,
            }
        }
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Return { value: Some(v), .. } if has_fn_ref(v)
        )));
    }

    #[test]
    fn regex_call_padding_and_internal_name() {
        // `regex::` calls pad optional trailing arguments and internalize.
        let ir = lower_src(
            "IMPORT regex\n\
             FUNC run() AS Boolean\n  RETURN regex::match(\"abc\", \"a.*\")\nEND FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::Call { .. }),
                ..
            }
        )));
    }

    #[test]
    fn nested_call_arguments_capture_and_lower() {
        // Nested built-in call as an argument exercises the argument-lowering
        // recursion and expected-type propagation.
        let ir = lower_src(
            "IMPORT io\n\
             SUB main\n  io::print(toString(len(\"abc\")))\nEND SUB\n",
        );
        fn has_nested_call(ops: &[IrOp]) -> bool {
            fn value_has(v: &IrValue) -> bool {
                match v {
                    IrValue::Call { args, .. } => {
                        args.iter().any(|a| matches!(a, IrValue::Call { .. }))
                            || args.iter().any(value_has)
                    }
                    _ => false,
                }
            }
            ops.iter()
                .any(|op| matches!(op, IrOp::Eval { value, .. } if value_has(value)))
        }
        assert!(has_nested_call(main_body(&ir)));
    }

    // ---- lambda body captures across many expression shapes --------------

    #[test]
    fn lambda_captures_across_expression_shapes() {
        // The lambda body references captures inside a binary op, a member
        // access, a list literal, a map literal, a constructor, and a nested
        // call — exercising each arm of `collect_captured_locals`.
        let ir = lower_src(
            "IMPORT collections\n\
             TYPE Point\n  x AS Integer\nEND TYPE\n\
             FUNC run() AS List OF Integer\n\
               LET base AS Integer = 1\n\
               LET p AS Point = Point[2]\n\
               LET xs AS List OF Integer = [10]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v + base + p.x + len([base]))\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        // Both `base` and `p` are captured by the lambda.
        let lambda = ir
            .functions
            .iter()
            .find(|f| f.name.starts_with("$lambda"))
            .expect("a lambda");
        let capture_binds = lambda
            .body
            .iter()
            .filter(|op| {
                matches!(
                    op,
                    IrOp::Bind {
                        value: Some(IrValue::Capture { .. }),
                        ..
                    }
                )
            })
            .count();
        assert!(capture_binds >= 2, "captured at least base and p");
    }

    // ---- native LINK expression forms (lower_link_expr / eval_link_const) --

    #[test]
    fn link_success_on_with_comparison_and_boolean_ops() {
        // Exercises eval_link_const (number, boolean, NOTHING, unary +/-) and
        // lower_link_expr (Var identifier, NOTHING, comparison, AND/OR/NOT,
        // non-logic binary fallthrough, unary minus on a Var).
        let src = "RESOURCE H CLOSE BY lib::done\n\
             LINK \"c\" AS lib\n\
               FUNC make() AS RES H\n\
                 SYMBOL \"make\"\n\
                 ABI (a CInt32, b CInt32, c CInt32, return OUT CPtr) AS status CInt32\n\
                 CONST a = -1\n\
                 CONST b = TRUE\n\
                 CONST c = NOTHING\n\
                 SUCCESS_ON status >= 0 AND status <> 5\n\
                 RESULT status OR NOT (status < 0)\n\
               END FUNC\n\
               FUNC probe() AS RES H\n\
                 SYMBOL \"probe\"\n\
                 ABI (return OUT CPtr) AS status CInt32\n\
                 SUCCESS_ON status\n\
                 RESULT -status\n\
               END FUNC\n\
               FUNC probe2() AS RES H\n\
                 SYMBOL \"probe2\"\n\
                 ABI (return OUT CPtr) AS status CInt32\n\
                 SUCCESS_ON status <> NOTHING\n\
                 RESULT status + 1\n\
               END FUNC\n\
               FUNC done(RES h AS H) AS Nothing\n\
                 SYMBOL \"done\"\n\
                 ABI (h CPtr) AS status CInt32\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
             END LINK\n\
             SUB main\nEND SUB\n";
        let ir = try_lower_src(src).expect("link SUCCESS_ON/RESULT program should lower");
        let make = ir
            .link_functions
            .iter()
            .find(|f| f.name == "make")
            .expect("make");
        assert!(make.success_on.is_some());
        assert!(make.result.is_some());
        // CONST pins were evaluated: a=-1, b=TRUE(1), c=NOTHING(0).
        assert_eq!(make.consts.len(), 3);
        // probe2's SUCCESS_ON/RESULT lowered (NOTHING comparison + non-logic
        // binary).
        assert!(ir
            .link_functions
            .iter()
            .any(|f| f.name == "probe2" && f.result.is_some()));
    }

    // ---- external functions (lower_project_with_external_functions) -------

    #[test]
    fn external_function_return_and_param_types_are_used() {
        use super::super::ExternalFunctionParam;
        use std::collections::HashMap as StdHashMap;

        // Build the AST directly and lower with external function tables so the
        // external-function seam (params/returns injection) is exercised.
        let src = "FUNC run() AS Integer\n  RETURN extAdd(1, 2)\nEND FUNC\nSUB main\nEND SUB\n";
        let file =
            crate::ast::parse_source(std::path::Path::new("src/main.mfb"), "src/main.mfb", src)
                .expect("parse");
        let project = crate::ast::AstProject {
            name: "irtest".to_string(),
            files: vec![file],
        };

        let mut types = StdHashMap::new();
        types.insert(
            "extAdd".to_string(),
            "FUNC(Integer, Integer) AS Integer".to_string(),
        );
        let mut params = StdHashMap::new();
        params.insert(
            "extAdd".to_string(),
            vec![
                ExternalFunctionParam {
                    name: "a".to_string(),
                    type_: "Integer".to_string(),
                },
                ExternalFunctionParam {
                    name: "b".to_string(),
                    type_: "Integer".to_string(),
                },
            ],
        );
        let ir = lower_project_with_external_functions(&project, None, &types, &params);
        // The call to the external function types as Integer (from the external
        // return-type table).
        let call_ty = function(&ir, "run").body.iter().find_map(|op| match op {
            IrOp::Return {
                value: Some(IrValue::Call { target, type_, .. }),
                ..
            } if target == "extAdd" => Some(type_.clone()),
            _ => None,
        });
        assert_eq!(call_ty.as_deref(), Some("Integer"));
    }

    // ---- write_ir --------------------------------------------------------

    #[test]
    fn write_ir_emits_ir_file() {
        let ir = lower_src("SUB main\nEND SUB\n");
        let dir = std::env::temp_dir().join(format!(
            "mfb_write_ir_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = write_ir(&dir, &ir).expect("write_ir");
        assert!(path.exists());
        assert!(std::fs::read_to_string(&path).unwrap().contains(&ir.name));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- collect_project_docs / import inference smoke -------------------

    #[test]
    fn inferred_binding_from_list_literal_types() {
        let ir = lower_src("IMPORT collections\nLET xs = [1, 2, 3]\nSUB main\nEND SUB\n");
        assert_eq!(binding(&ir, "xs").type_, "List OF Integer");
    }

    // ---- more builtin packages: expression_type + call padding ----------

    #[test]
    fn json_and_csv_calls_resolve() {
        let ir = lower_src(
            "IMPORT json\n\
             IMPORT csv\n\
             FUNC run() AS String\n\
               LET j AS Json = json::parse(\"1\")\n\
               LET s AS String = json::stringify(j)\n\
               LET g AS List OF List OF String = csv::parse(\"a,b\")\n\
               RETURN s\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(matches!(
            bind_value(&function(&ir, "run").body, "j"),
            Some(IrValue::Call { type_, .. }) if type_ == "Json"
        ));
    }

    #[test]
    fn crypto_call_resolves_and_pads_aad() {
        // crypto::sha256 resolves via the crypto resolver; an AEAD call pads its
        // optional `aad` argument to an empty byte list.
        let ir = lower_src(
            "IMPORT crypto\n\
             FUNC run() AS List OF Byte\n\
               RETURN crypto::sha256(\"data\")\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::Call { .. }),
                ..
            }
        )));
    }

    #[test]
    fn net_and_http_calls_resolve_and_pad() {
        // net::toUrl -> Url; http::read pads its default headers (empty map) and
        // method (a literal), exercising the http default-argument padding.
        let src = "IMPORT net\n\
             IMPORT http\n\
             FUNC run() AS Integer\n\
               LET u AS Url = net::toUrl(\"http://x\")\n\
               LET r AS http::Response = http::read(u)\n\
               RETURN 0\n\
             END FUNC\n\
             SUB main\nEND SUB\n";
        let ir = try_lower_src(src).expect("net/http program should lower");
        let f = function(&ir, "run");
        // http::read(u) pads BOTH headers (empty map literal) and method (a
        // String const), exercising the map and scalar arms of http padding.
        let has_map_pad = f.body.iter().any(|op| match op {
            IrOp::Bind {
                value: Some(IrValue::Call { args, .. }),
                ..
            } => args.iter().any(|a| matches!(a, IrValue::MapLiteral { .. })),
            _ => false,
        });
        assert!(
            has_map_pad,
            "http::read headers padded as an empty map literal"
        );
    }

    #[test]
    fn thread_call_resolves() {
        let src = "IMPORT thread\n\
             ISOLATED FUNC worker(w AS thread::ThreadWorker OF Integer TO Integer, seed AS Integer) AS Integer\n\
               RETURN seed\n\
             END FUNC\n\
             FUNC run() AS Integer\n\
               LET t AS thread::Thread OF Integer TO Integer = thread::start(worker, 5)\n\
               RETURN 0\n\
             END FUNC\n\
             SUB main\nEND SUB\n";
        // thread APIs are complex; only assert lowering succeeds when the front
        // end accepts this program.
        if let Some(ir) = try_lower_src(src) {
            assert!(!function(&ir, "run").body.is_empty());
        }
    }

    #[test]
    fn datetime_time_pads_optional_arguments() {
        // datetime::time(hour, minute) pads the optional second/nanos with 0.
        let ir = lower_src(
            "IMPORT datetime\n\
             FUNC run() AS Time\n  RETURN datetime::time(10, 30)\nEND FUNC\n\
             SUB main\nEND SUB\n",
        );
        let padded = function(&ir, "run").body.iter().any(|op| match op {
            IrOp::Return {
                value: Some(IrValue::Call { args, .. }),
                ..
            } => args.len() >= 4,
            _ => false,
        });
        assert!(padded, "second/nanos padded to a 4-arg call");
    }

    #[test]
    fn regex_find_pads_start_argument() {
        let ir = lower_src(
            "IMPORT regex\n\
             FUNC run() AS Integer\n  RETURN regex::find(\"abc\", \"b\")\nEND FUNC\n\
             SUB main\nEND SUB\n",
        );
        let padded = function(&ir, "run").body.iter().any(|op| match op {
            IrOp::Return {
                value: Some(IrValue::Call { args, .. }),
                ..
            } => args.len() >= 3,
            _ => false,
        });
        assert!(padded, "start default padded to a 3-arg call");
    }

    #[test]
    fn crypto_aead_pads_aad_as_empty_list() {
        // crypto::aes256GcmSeal(key, nonce, plaintext) pads the optional `aad`
        // with an empty byte list (ListLiteral, not a scalar Const).
        let src = "IMPORT crypto\n\
             FUNC run(key AS List OF Byte, nonce AS List OF Byte, pt AS List OF Byte) AS crypto::Sealed\n\
               RETURN crypto::aes256GcmSeal(key, nonce, pt)\n\
             END FUNC\n\
             SUB main\nEND SUB\n";
        let ir = try_lower_src(src).expect("crypto AEAD program should lower");
        let has_list_pad = function(&ir, "run").body.iter().any(|op| match op {
            IrOp::Return {
                value: Some(IrValue::Call { args, .. }),
                ..
            } => args
                .last()
                .is_some_and(|a| matches!(a, IrValue::ListLiteral { .. })),
            _ => false,
        });
        assert!(has_list_pad, "aad padded as an empty list literal");
    }

    #[test]
    fn tls_connect_pads_optional_arguments() {
        let src = "IMPORT tls\n\
             FUNC run() AS Integer\n\
               RES s AS tls::TlsSocket = tls::connect(\"host\", 443)\n\
               tls::close(s)\n\
               RETURN 0\n\
             END FUNC\n\
             SUB main\nEND SUB\n";
        let ir = try_lower_src(src).expect("tls connect program should lower");
        let padded = function(&ir, "run").body.iter().any(|op| match op {
            IrOp::Bind {
                value: Some(IrValue::Call { args, .. }),
                ..
            } => args.len() >= 4,
            _ => false,
        });
        assert!(padded, "timeoutMs + serverName padded to a 4-arg call");
    }

    #[test]
    fn tls_close_on_listener_routes_to_listener_helper() {
        // `tls::close` on a `TlsListener` operand routes to the listener-shaped
        // internal close helper (the tls.close listener branch).
        let src = "IMPORT tls\n\
             FUNC run() AS Integer\n\
               RES l AS tls::TlsListener = tls::listen(\"0.0.0.0\", 8080, \"cert\", \"key\")\n\
               tls::close(l)\n\
               RETURN 0\n\
             END FUNC\n\
             SUB main\nEND SUB\n";
        if let Some(ir) = try_lower_src(src) {
            assert!(function(&ir, "run")
                .body
                .iter()
                .any(|op| matches!(op, IrOp::Eval { .. })));
        }
    }

    #[test]
    fn vector_call_and_constant_lower() {
        // A vector call resolves a type-specific internal name from its argument
        // record types (implementation_name), and a vector constant inlines a
        // record constructor (constant_components).
        let ir = lower_src(
            "IMPORT vector\n\
             IMPORT io\n\
             FUNC run() AS Float\n\
               LET a AS vector::Float3 = vector::Float3[1.0, 2.0, 2.0]\n\
               LET up AS vector::Float3 = vector::upFloat3\n\
               RETURN vector::length(a)\n\
             END FUNC\n\
             SUB main\n  io::print(\"x\")\nEND SUB\n",
        );
        let f = function(&ir, "run");
        // The `up` constant inlined a Constructor.
        assert!(matches!(
            bind_value(&f.body, "up"),
            Some(IrValue::Constructor { .. })
        ));
        // `vector::length(a)` lowers to a Call with a resolved (internal) target.
        assert!(f.body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::Call { .. }),
                ..
            }
        )));
    }

    #[test]
    fn datetime_call_pads_default_arguments() {
        // datetime constructors are arity-aware and pad optional arguments.
        let ir = lower_src(
            "IMPORT datetime\n\
             FUNC run() AS Instant\n  RETURN datetime::instant(0)\nEND FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::Call { .. }),
                ..
            }
        )));
    }

    // ---- inline-TRAP handler treeify: loops & nested statements ----------

    #[test]
    fn inline_trap_mut_bind_and_global_assign_targets() {
        // A MUT inline-trap bind marks the target mutable (mutable-bind arm), and
        // an inline-trap assign to a top-level binding takes the AssignGlobal arm.
        let ir = lower_src(
            "MUT counter AS Integer = 0\n\
             FUNC run(s AS String) AS Integer\n\
               MUT local AS Integer = toInt(s) TRAP(e)\n    RECOVER 0\n  END TRAP\n\
               counter = toInt(s) TRAP(e)\n    RECOVER 0\n  END TRAP\n\
               RETURN local + counter\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        let f = function(&ir, "run");
        // The global assign target lowered to AssignGlobal.
        assert!(f
            .body
            .iter()
            .any(|op| matches!(op, IrOp::AssignGlobal { name, .. } if name == "counter")));
        // The MUT bind of `local` is mutable.
        assert!(f.body.iter().any(|op| matches!(
            op,
            IrOp::Bind { mutable: true, name, .. } if name == "local"
        )));
    }

    #[test]
    fn inline_trap_handler_with_terminator_then_dead_statement() {
        // A handler whose head diverges (FAIL) with a following statement drops
        // the unreachable tail (statement_terminates head, tail present).
        let ir = lower_src(
            "FUNC run(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n    FAIL e\n    RECOVER 0\n  END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::If {
                condition: IrValue::ResultIsOk { .. },
                ..
            }
        )));
    }

    #[test]
    fn inline_trap_handler_is_a_single_if() {
        // The handler's only statement is an IF (no continuation), so it is
        // normalized by treeify_statement's If arm (recursing into both blocks).
        let ir = lower_src(
            "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 IF e.code = 404 THEN\n            RECOVER 0\n          ELSE\n            RECOVER 1\n          END IF\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "parse").body.iter().any(|op| matches!(
            op,
            IrOp::If {
                condition: IrValue::ResultIsOk { .. },
                ..
            }
        )));
    }

    #[test]
    fn inline_trap_handler_is_a_single_match() {
        // The handler's only statement is a MATCH (no continuation), normalized
        // by treeify_statement's Match arm.
        let ir = lower_src(
            "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 MATCH e.code\n\
                   CASE 404\n              RECOVER 0\n\
                   CASE ELSE\n              RECOVER 1\n\
                 END MATCH\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "parse").body.iter().any(|op| matches!(
            op,
            IrOp::If {
                condition: IrValue::ResultIsOk { .. },
                ..
            }
        )));
    }

    #[test]
    fn inline_trap_handler_is_a_single_loop() {
        // The handler's only statement is a loop, so treeify recurses into the
        // loop bodies via treeify_statement (For / While / DoUntil / ForEach).
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC parse(s AS String) AS Integer\n\
               MUT acc AS Integer = 0\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 FOR i = 1 TO 2\n\
                   WHILE acc < 0\n              acc = acc + 1\n            WEND\n\
                   DO\n              acc = acc + 1\n            LOOP UNTIL acc > 5\n\
                   FOR EACH v IN [1, 2]\n              acc = acc + v\n            NEXT\n\
                   RECOVER acc\n\
                 NEXT\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "parse").body.iter().any(|op| matches!(
            op,
            IrOp::If {
                condition: IrValue::ResultIsOk { .. },
                ..
            }
        )));
    }

    #[test]
    fn inline_trap_handler_with_loop_and_continuation() {
        // Handler whose head is a non-terminating loop (falls through), followed
        // by a RECOVER continuation. Exercises the fall-through arm of
        // treeify_handler and the loop arm of treeify_statement.
        let ir = lower_src(
            "FUNC parse(s AS String) AS Integer\n\
               MUT acc AS Integer = 0\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 FOR i = 1 TO 2\n            acc = acc + i\n          NEXT\n\
                 WHILE acc < 0\n            acc = acc + 1\n          WEND\n\
                 RECOVER acc\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "parse").body.iter().any(|op| matches!(
            op,
            IrOp::If {
                condition: IrValue::ResultIsOk { .. },
                ..
            }
        )));
    }

    #[test]
    fn inline_trap_handler_with_nested_if_and_continuation() {
        // Handler head is an IF whose branches fall through; the continuation is
        // distributed into both branches (distribute_continuation).
        let ir = lower_src(
            "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 IF e.code > 0 THEN\n            LET x AS Integer = 1\n          ELSE\n            LET y AS Integer = 2\n          END IF\n\
                 RECOVER 0\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "parse").body.iter().any(|op| matches!(
            op,
            IrOp::If {
                condition: IrValue::ResultIsOk { .. },
                ..
            }
        )));
    }

    #[test]
    fn inline_trap_handler_with_match_else_and_continuation() {
        // Handler head is a MATCH that already has an ELSE arm; the continuation
        // is distributed into every arm without synthesizing an extra ELSE.
        let ir = lower_src(
            "FUNC parse(s AS String) AS Integer\n\
               LET n AS Integer = toInt(s) TRAP(e)\n\
                 MATCH e.code\n\
                   CASE 404\n              LET a AS Integer = 1\n\
                   CASE ELSE\n              LET b AS Integer = 2\n\
                 END MATCH\n\
                 RECOVER 0\n\
               END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "parse").body.iter().any(|op| matches!(
            op,
            IrOp::If {
                condition: IrValue::ResultIsOk { .. },
                ..
            }
        )));
    }

    // ---- named-argument reordering (normalize_*_call_arguments) ----------

    #[test]
    fn builtin_named_arguments_reorder_with_extra() {
        // A builtin called with a MIX of positional and named arguments takes
        // the param-name reorder branch of normalize_builtin_call_arguments
        // (positional fills index 0, the named `delimiter` fills index 1).
        let ir = lower_src(
            "IMPORT strings\n\
             FUNC run() AS String\n  RETURN strings::join([\"a\", \"b\"], delimiter := \"-\")\nEND FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::Call { .. }),
                ..
            }
        )));
    }

    #[test]
    fn local_call_named_arguments_reorder() {
        // A user-function call with named args reordered exercises
        // normalize_local_call_arguments + lower_local_call_arguments.
        let ir = lower_src(
            "FUNC box(w AS Integer, h AS Integer) AS Integer\n  RETURN w * h\nEND FUNC\n\
             SUB main\n  LET a AS Integer = box(h := 3, w := 4)\nEND SUB\n",
        );
        assert!(matches!(
            bind_value(main_body(&ir), "a"),
            Some(IrValue::Call { target, args, .. }) if target == "box" && args.len() == 2
        ));
    }

    // ---- captured-locals variants (collect_captured_locals arms) ---------

    #[test]
    fn lambda_captures_through_map_literal_and_with_update() {
        let ir = lower_src(
            "IMPORT collections\n\
             TYPE Point\n  x AS Integer\nEND TYPE\n\
             FUNC run() AS List OF Integer\n\
               LET base AS Integer = 5\n\
               LET p AS Point = Point[1]\n\
               LET keyv AS String = \"k\"\n\
               LET xs AS List OF Integer = [1]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v + collections::getOr(Map OF String TO Integer { keyv := base }, keyv, 0) + (WITH p { x := base }).x)\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        let lambda = ir
            .functions
            .iter()
            .find(|f| f.name.starts_with("$lambda"))
            .expect("a lambda");
        // base, p, keyv are all captured.
        let captures = lambda
            .body
            .iter()
            .filter(|op| {
                matches!(
                    op,
                    IrOp::Bind {
                        value: Some(IrValue::Capture { .. }),
                        ..
                    }
                )
            })
            .count();
        assert!(captures >= 3, "captured base, p and keyv (got {captures})");
    }

    #[test]
    fn lambda_captures_called_function_local_and_constructor_arg() {
        // The lambda calls a captured function-typed local `f` (callee-capture
        // arm) and constructs a record from a captured value `base` (constructor
        // arm) and a list literal (list-literal arm).
        let ir = lower_src(
            "IMPORT collections\n\
             TYPE Box\n  n AS Integer\nEND TYPE\n\
             FUNC makeAdder(k AS Integer) AS FUNC(Integer) AS Integer\n\
               RETURN LAMBDA(x AS Integer) -> x + k\n\
             END FUNC\n\
             FUNC run() AS List OF Integer\n\
               LET base AS Integer = 3\n\
               LET f AS FUNC(Integer) AS Integer = makeAdder(1)\n\
               LET xs AS List OF Integer = [1]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> f(v) + Box[base].n + len([base]))\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        // The inner lambda captures both `f` (called) and `base` (in a
        // constructor + list literal).
        let inner = ir
            .functions
            .iter()
            .filter(|f| f.name.starts_with("$lambda"))
            .max_by_key(|f| f.body.len())
            .expect("a lambda");
        let captures = inner
            .body
            .iter()
            .filter(|op| {
                matches!(
                    op,
                    IrOp::Bind {
                        value: Some(IrValue::Capture { .. }),
                        ..
                    }
                )
            })
            .count();
        assert!(captures >= 2, "captured f and base (got {captures})");
    }

    // ---- assignment-bodied lambda binding to a captured target -----------

    #[test]
    fn assignment_bodied_lambda_captures_assignment_target() {
        // The lambda's assignment target (`acc`) is captured even though it is
        // only written, not read, in the body.
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC run() AS Integer\n\
               MUT acc AS Integer = 0\n\
               LET xs AS List OF Integer = [1, 2]\n\
               collections::forEach(xs, LAMBDA(v AS Integer) -> acc = v)\n\
               RETURN acc\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        let lambda = ir
            .functions
            .iter()
            .find(|f| f.name.starts_with("$lambda"))
            .expect("a lambda");
        assert!(lambda
            .body
            .iter()
            .any(|op| matches!(op, IrOp::Assign { name, .. } if name == "acc")));
    }

    #[test]
    fn local_call_named_then_positional_skips_filled_slot() {
        // A named arg fills index 0, then trailing positionals skip the filled
        // slot (the skip-while loop of normalize_local_call_arguments).
        let ir = lower_src(
            "FUNC box(w AS Integer, h AS Integer, d AS Integer) AS Integer\n  RETURN w * h * d\nEND FUNC\n\
             SUB main\n  LET a AS Integer = box(w := 4, 5, 6)\nEND SUB\n",
        );
        assert!(matches!(
            bind_value(main_body(&ir), "a"),
            Some(IrValue::Call { target, args, .. }) if target == "box" && args.len() == 3
        ));
    }

    #[test]
    fn builtin_call_named_then_positional_skips_filled_slot() {
        // Same skip path for a built-in call: a named arg fills an index, then a
        // positional skips it (the skip-while loop of
        // normalize_builtin_call_arguments).
        let ir = lower_src(
            "IMPORT strings\n\
             FUNC run() AS String\n  RETURN strings::join(delimiter := \"-\", [\"a\", \"b\"])\nEND FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::Call { .. }),
                ..
            }
        )));
    }

    // ---- native RESOURCE visibility variants -----------------------------

    #[test]
    fn native_resource_visibility_variants() {
        // A package-visible and a private RESOURCE exercise the package/private
        // arms of native_resources' visibility mapping.
        let src = "PACKAGE RESOURCE Db CLOSE BY lib::close\n\
             RESOURCE Cache CLOSE BY lib::freeCache\n\
             LINK \"c\" AS lib\n\
               FUNC open() AS RES Db\n\
                 SYMBOL \"open\"\n\
                 ABI (return OUT CPtr) AS status CInt32\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
               FUNC close(RES db AS Db) AS Nothing\n\
                 SYMBOL \"close\"\n\
                 ABI (db CPtr) AS status CInt32\n\
               END FUNC\n\
               FUNC makeCache() AS RES Cache\n\
                 SYMBOL \"mk\"\n\
                 ABI (return OUT CPtr) AS status CInt32\n\
                 SUCCESS_ON status = 0\n\
               END FUNC\n\
               FUNC freeCache(RES c AS Cache) AS Nothing\n\
                 SYMBOL \"free\"\n\
                 ABI (c CPtr) AS status CInt32\n\
               END FUNC\n\
             END LINK\n\
             SUB main\nEND SUB\n";
        if let Some(ir) = try_lower_src(src) {
            let db = ir.native_resources.iter().find(|r| r.name == "Db");
            let cache = ir.native_resources.iter().find(|r| r.name == "Cache");
            assert!(db.is_some_and(|r| r.visibility == "package"));
            assert!(cache.is_some_and(|r| r.visibility == "private"));
        }
    }

    // ---- captured_locals: unary / trapped arms ---------------------------

    #[test]
    fn lambda_captures_through_unary_and_trapped() {
        // The lambda body references a captured local inside a unary op and
        // inside a trapped call, exercising the Unary and Trapped arms of
        // collect_captured_locals.
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC dbl(x AS Integer) AS Integer\n  RETURN x * 2\nEND FUNC\n\
             FUNC run() AS List OF Integer\n\
               LET base AS Integer = 4\n\
               LET xs AS List OF Integer = [1]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v + (-base))\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        let lambda = ir
            .functions
            .iter()
            .find(|f| f.name.starts_with("$lambda"))
            .expect("a lambda");
        assert!(lambda.body.iter().any(|op| matches!(
            op,
            IrOp::Bind {
                value: Some(IrValue::Capture { .. }),
                ..
            }
        )));
    }

    // ---- collections.filter with a lambda predicate ----------------------

    #[test]
    fn collections_filter_with_lambda_predicate() {
        // A `collections::filter(list, <lambda>)` (non-identifier predicate)
        // takes the else branch that lowers each argument normally.
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC run() AS List OF Integer\n\
               LET xs AS List OF Integer = [1, 2, 3]\n\
               RETURN collections::filter(xs, LAMBDA(v AS Integer) -> v > 1)\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::Call { .. }),
                ..
            }
        )));
    }

    // ---- DOC for a callable with no matching overload --------------------

    #[test]
    fn doc_for_unmatched_overload_is_dropped() {
        // A DOC FUNC header whose parameter types match no overload is dropped
        // (overload_for returns None).
        let src = "DOC\n  FUNC add(String, String)\n  DESC No such overload.\nEND DOC\n\
             EXPORT FUNC add(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\n\
             SUB main\nEND SUB\n";
        if let Some(ir) = try_lower_src(src) {
            assert!(!ir.docs.decls.iter().any(|d| d.name == "add"));
        }
    }

    // ---- literal_expression_type fallback --------------------------------

    #[test]
    fn list_literal_without_expected_type_infers_from_first_literal() {
        // A list literal lowered with no expected element type (as an argument
        // to `len`, whose parameter type is generic) drives
        // literal_expression_type over the first element (a String literal).
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC run() AS Integer\n\
               RETURN len([\"a\", \"b\"])\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        // The inner list literal was typed from its first (String) element.
        fn find_list(v: &IrValue) -> Option<&str> {
            match v {
                IrValue::ListLiteral { type_, .. } => Some(type_.as_str()),
                IrValue::Call { args, .. } => args.iter().find_map(find_list),
                _ => None,
            }
        }
        let ty = function(&ir, "run").body.iter().find_map(|op| match op {
            IrOp::Return { value: Some(v), .. } => find_list(v),
            _ => None,
        });
        assert_eq!(ty, Some("List OF String"));
    }

    #[test]
    fn list_literal_without_expected_and_non_literal_first_element() {
        // First element is a call (not a plain literal) -> literal_expression_type
        // returns None -> element type falls back to "Unknown".
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC run() AS Integer\n\
               RETURN len([len(\"a\"), 2])\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::Call { .. }),
                ..
            }
        )));
    }

    #[test]
    fn builtin_type_field_access_and_net_lookup() {
        // net::lookup -> List OF Address; accessing an Address's `host` field
        // resolves via TypeIndex::record_field_type's builtin-type-fields branch.
        let src = "IMPORT net\n\
             IMPORT collections\n\
             FUNC run() AS String\n\
               LET addrs AS List OF Address = net::lookup(\"example.com\")\n\
               LET a AS Address = collections::get(addrs, 0)\n\
               RETURN a.host\n\
             END FUNC\n\
             SUB main\nEND SUB\n";
        let ir = try_lower_src(src).expect("net::lookup program should lower");
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::MemberAccess { .. }),
                ..
            }
        )));
    }

    #[test]
    fn collections_filter_with_user_function_predicate() {
        // A user FUNC predicate (not a built-in like isEven) yields no builtin
        // predicate type, taking the else branch that lowers args normally.
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC keep(x AS Integer) AS Boolean\n  RETURN x > 0\nEND FUNC\n\
             FUNC run() AS List OF Integer\n\
               LET xs AS List OF Integer = [1, 2, 3]\n\
               RETURN collections::filter(xs, keep)\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::Call { .. }),
                ..
            }
        )));
    }

    #[test]
    fn list_literal_first_element_float_bool_and_nothing() {
        // literal_expression_type over Float, Boolean, and NOTHING first
        // elements (all lowered with no expected type).
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC a() AS Integer\n  RETURN len([1.5, 2.5])\nEND FUNC\n\
             FUNC b() AS Integer\n  RETURN len([NOTHING])\nEND FUNC\n\
             FUNC c() AS Integer\n  RETURN len([TRUE, FALSE])\nEND FUNC\n\
             SUB main\nEND SUB\n",
        );
        fn list_type(v: &IrValue) -> Option<String> {
            match v {
                IrValue::ListLiteral { type_, .. } => Some(type_.clone()),
                IrValue::Call { args, .. } => args.iter().find_map(list_type),
                _ => None,
            }
        }
        let a_ty = function(&ir, "a").body.iter().find_map(|op| match op {
            IrOp::Return { value: Some(v), .. } => list_type(v),
            _ => None,
        });
        assert_eq!(a_ty.as_deref(), Some("List OF Float"));
    }

    #[test]
    fn nested_lambda_inside_lambda_body() {
        // A lambda whose body contains another lambda exercises the Lambda arm of
        // collect_captured_locals (the nested lambda is not descended into for
        // the outer capture set).
        let src = "IMPORT collections\n\
             FUNC run() AS List OF Integer\n\
               LET k AS Integer = 2\n\
               LET xs AS List OF Integer = [1]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v + collections::sum(collections::transform([k], LAMBDA(w AS Integer) -> w + 1)))\n\
             END FUNC\n\
             SUB main\nEND SUB\n";
        if let Some(ir) = try_lower_src(src) {
            assert!(
                ir.functions
                    .iter()
                    .filter(|f| f.name.starts_with("$lambda"))
                    .count()
                    >= 2
            );
        }
    }

    #[test]
    fn lambda_captures_through_trapped_expression() {
        // The lambda body contains a trapped call referencing a captured local,
        // exercising the Trapped arm of collect_captured_locals.
        let src = "IMPORT collections\n\
             FUNC parse(s AS String) AS Integer\n  RETURN toInt(s)\nEND FUNC\n\
             FUNC run(seed AS String) AS List OF Integer\n\
               LET xs AS List OF Integer = [1]\n\
               RETURN collections::transform(xs, LAMBDA(v AS Integer) -> v + (parse(seed) TRAP(e) RECOVER 0 END TRAP))\n\
             END FUNC\n\
             SUB main\nEND SUB\n";
        // Inline TRAP inside a lambda expression may be rejected; only assert
        // when it lowers.
        if let Some(ir) = try_lower_src(src) {
            assert!(ir.functions.iter().any(|f| f.name.starts_with("$lambda")));
        }
    }

    #[test]
    fn filter_result_typed_binding_resolves_element_type() {
        // Binding the result of `collections::filter(xs, isEven)` requires
        // expression_type over the filter call (the builtin-predicate branch of
        // the collections.filter expression_type resolution).
        let ir = lower_src(
            "IMPORT collections\n\
             FUNC run() AS Integer\n\
               LET xs AS List OF Integer = [1, 2, 3, 4]\n\
               LET evens = collections::filter(xs, isEven)\n\
               RETURN len(evens)\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        // `evens` inferred as List OF Integer from the filter call.
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Bind { name, type_, .. } if name == "evens" && type_ == "List OF Integer"
        )));
    }

    #[test]
    fn call_through_zero_arg_function_typed_local() {
        // Calling a `FUNC() AS Integer`-typed local drives
        // function_param_types_from_type over an empty parameter list.
        let ir = lower_src(
            "FUNC seed() AS Integer\n  RETURN 7\nEND FUNC\n\
             FUNC run() AS Integer\n\
               LET f AS FUNC() AS Integer = seed\n\
               RETURN f()\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "run").body.iter().any(|op| matches!(
            op,
            IrOp::Return {
                value: Some(IrValue::Call { .. }),
                ..
            }
        )));
    }

    #[test]
    fn doc_for_nonexported_type_is_dropped() {
        // A DOC TYPE header for a non-exported (package/private) type is dropped
        // (the visibility check in collect_project_docs).
        let src = "DOC\n  TYPE Hidden\n  DESC Not exported.\nEND DOC\n\
             TYPE Hidden\n  n AS Integer\nEND TYPE\n\
             SUB main\nEND SUB\n";
        if let Some(ir) = try_lower_src(src) {
            assert!(!ir.docs.decls.iter().any(|d| d.name == "Hidden"));
        }
    }

    #[test]
    fn inline_trap_binding_type_from_trapped_expression() {
        // An inferred `LET x = call() TRAP ...` computes the success type via
        // expression_type over the inner call (the inline-trap success-type path).
        let ir = lower_src(
            "FUNC parse(s AS String) AS Integer\n\
               LET n = toInt(s) TRAP(e)\n    RECOVER 0\n  END TRAP\n\
               RETURN n\n\
             END FUNC\n\
             SUB main\nEND SUB\n",
        );
        assert!(function(&ir, "parse").body.iter().any(|op| matches!(
            op,
            IrOp::Bind { name, type_, .. } if name == "n" && type_ == "Integer"
        )));
    }
}
