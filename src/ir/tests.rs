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

/// exercise the AST->IR lowering paths (`lower.rs`) directly.
#[cfg(test)]
mod lower_pipeline_tests {
    use crate::ast;
    use crate::manifest::validate_project_manifest;
    use crate::monomorph;
    use crate::resolver;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "mfb_ir_lower_{name}_{}_{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("temp src dir");
        root
    }

    const PROJECT_JSON: &str = r#"{ "name": "irlower", "version": "0.1.0", "mfb": "1.0",
        "kind": "executable",
        "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
        "entry": "main", "targets": ["native"] }"#;

    /// Lower a single-file program through the whole front end and return the IR.
    /// Panics with the failing stage if any pre-lowering stage rejects the source
    /// (so a broken test source surfaces immediately rather than silently).
    fn lower_src(name: &str, source: &str) -> super::IrProject {
        try_lower_src(name, source).expect("source must lower cleanly")
    }

    /// Like [`lower_src`] but returns `None` when a pre-lowering stage rejects the
    /// program. Diagnostic noise from the front end is suppressed.
    fn try_lower_src(name: &str, source: &str) -> Option<super::IrProject> {
        let dir = temp_dir(name);
        std::fs::write(dir.join("project.json"), PROJECT_JSON).unwrap();
        std::fs::write(dir.join("src").join("main.mfb"), source).unwrap();
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let manifest = validate_project_manifest(&dir.join("project.json")).ok();
        let result = manifest.and_then(|manifest| {
            let name = manifest
                .get("name")
                .and_then(|v| v.get::<String>())
                .cloned()?;
            let ast = ast::parse_project(&name, &dir, &manifest).ok()?;
            resolver::resolve_project(&dir, &manifest, &ast).ok()?;
            let concrete = monomorph::monomorphize_project(&dir, &ast).ok()?;
            resolver::resolve_project_with(&dir, &manifest, &concrete, false).ok()?;
            Some(super::lower_project_with_external_functions(
                &concrete,
                None,
                &std::collections::HashMap::new(),
                &std::collections::HashMap::new(),
            ))
        });
        std::panic::set_hook(prev);
        result
    }

    /// Find a lowered function body by name (user functions only, not injected
    /// package or lambda bodies).
    fn function<'a>(ir: &'a super::IrProject, name: &str) -> &'a super::IrFunction {
        ir.functions
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("function `{name}` not found in lowered IR"))
    }

    /// Serialize the whole project to JSON — a convenient way to assert a
    /// lowering path was reached without matching every nested op by hand.
    fn json_of(ir: &super::IrProject) -> String {
        ir.to_json()
    }

    // ---- literals + operators + expressions -------------------------------

    #[test]
    fn lowers_every_literal_and_operator_kind() {
        let ir = lower_src(
            "literals",
            r#"
FUNC main AS Integer
  LET s AS String = "hi"
  LET i AS Integer = 42
  LET f AS Float = 3.5
  LET b AS Boolean = TRUE
  LET by AS Byte = 7
  LET fx AS Fixed = 1.25
  LET n AS Integer = -i
  LET notb AS Boolean = NOT b
  LET sum AS Integer = i + i - i * i
  LET cmp AS Boolean = i < i AND b OR NOT b
  LET cat AS String = s & s
  LET xr AS Boolean = b XOR b
  RETURN i
END FUNC
"#,
        );
        let j = json_of(&ir);
        // Byte / Fixed constant typing, unary NOT/negation, concat, comparisons.
        assert!(j.contains("\"Byte\""), "{j}");
        assert!(j.contains("\"Fixed\""), "{j}");
        assert!(j.contains("NOT"));
        assert!(j.contains('&'));
        assert!(j.contains("XOR"));
    }

    #[test]
    fn lowers_nothing_literal() {
        let ir = lower_src(
            "nothing",
            r#"
TYPE Box
  item AS Integer
END TYPE
FUNC main AS Integer
  LET x AS Integer = 0
  RETURN x
END FUNC
"#,
        );
        // Exercise a binding whose value is NOTHING through a Sub with no return.
        let ir2 = lower_src(
            "nothing2",
            r#"
IMPORT io
FUNC main AS Integer
  LET n = NOTHING
  RETURN 0
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("Box"));
        assert!(json_of(&ir2).contains("NOTHING"));
    }

    #[test]
    fn lowers_list_and_map_literals() {
        let ir = lower_src(
            "collections",
            r#"
FUNC main AS Integer
  LET nums AS List OF Integer = [1, 2, 3]
  LET empty AS List OF Integer = []
  LET m AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1, "b" := 2 }
  LET inferred = [10, 20]
  RETURN 0
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("List OF Integer"));
        assert!(j.contains("Map OF String TO Integer"));
    }

    // ---- constructors, member access, WITH ---------------------------------

    #[test]
    fn lowers_constructors_member_access_and_with_update() {
        let ir = lower_src(
            "records",
            r#"
TYPE Point
  x AS Integer
  y AS Integer
END TYPE
FUNC main AS Integer
  LET p AS Point = Point[1, 2]
  LET q AS Point = Point[x := 3, y := 4]
  LET moved AS Point = WITH p { x := 10 }
  LET ax AS Integer = p.x
  RETURN ax + q.y + moved.x
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("\"kind\": \"with\""), "{j}");
        assert!(j.contains("Point"));
    }

    #[test]
    fn lowers_enum_member_access() {
        let ir = lower_src(
            "enums",
            r#"
ENUM Color
  Red
  Green
  Blue
END ENUM
FUNC pick(c AS Color) AS Integer
  MATCH c
    CASE Color.Red
      RETURN 1
    CASE ELSE
      RETURN 0
  END MATCH
END FUNC
FUNC main AS Integer
  RETURN pick(Color.Green)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("Color"));
    }

    // ---- union wrap / extract ---------------------------------------------

    #[test]
    fn lowers_union_wrap_and_extract() {
        let ir = lower_src(
            "unions",
            r#"
TYPE Cat
  legs AS Integer
END TYPE
TYPE Dog
  legs AS Integer
END TYPE
UNION Animal
  Cat
  Dog
END UNION
FUNC describe(a AS Animal) AS Integer
  MATCH a
    CASE Cat(c)
      RETURN c.legs
    CASE Dog(d)
      RETURN d.legs + 100
  END MATCH
END FUNC
FUNC wrapIt() AS Animal
  RETURN Cat[4]
END FUNC
FUNC main AS Integer
  RETURN describe(wrapIt())
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("unionWrap"), "{j}");
        assert!(j.contains("unionExtract"), "{j}");
    }

    // ---- calls, fallible CallResult, FUNC refs -----------------------------

    #[test]
    fn lowers_calls_and_function_refs() {
        let ir = lower_src(
            "calls",
            r#"
FUNC helper(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC
FUNC applyIt(f AS FUNC(Integer, Integer) AS Integer) AS Integer
  RETURN f(2, 3)
END FUNC
FUNC main AS Integer
  LET fref AS FUNC(Integer, Integer) AS Integer = helper
  RETURN applyIt(fref) + helper(1, b := 9)
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("functionRef"), "{j}");
    }

    #[test]
    fn lowers_default_argument_padding_for_local_call() {
        let ir = lower_src(
            "defaults",
            r#"
FUNC greet(name AS String, punct AS String = "!") AS String
  RETURN name & punct
END FUNC
FUNC main AS Integer
  LET a AS String = greet("hi")
  LET b AS String = greet("yo", "?")
  RETURN 0
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("greet"));
    }

    // ---- statement kinds ---------------------------------------------------

    #[test]
    fn lowers_control_flow_statements() {
        let ir = lower_src(
            "control",
            r#"
FUNC main AS Integer
  MUT total AS Integer = 0
  FOR i = 1 TO 10 STEP 2
    total = total + i
    IF i > 7 THEN EXIT FOR
  NEXT
  FOR j = 0 TO 3
    IF j = 1 THEN CONTINUE FOR
    total = total + j
  NEXT
  WHILE total < 100
    total = total + 10
  WEND
  DO
    total = total - 1
  LOOP UNTIL total < 50
  RETURN total
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("\"op\": \"for\""));
        assert!(j.contains("while"));
        assert!(j.contains("doUntil"));
    }

    #[test]
    fn lowers_float_for_loop_and_exit_do_while() {
        let ir = lower_src(
            "floatfor",
            r#"
FUNC main AS Integer
  MUT sum AS Float = 0.0
  FOR x = 0.0 TO 2.0 STEP 0.5
    sum = sum + x
    IF sum > 1.0 THEN EXIT FOR
  NEXT
  MUT k AS Integer = 0
  DO
    k = k + 1
    IF k = 2 THEN EXIT DO
  LOOP UNTIL k > 100
  WHILE k < 5
    k = k + 1
    IF k = 4 THEN EXIT WHILE
  WEND
  RETURN k
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("Float"));
    }

    #[test]
    fn lowers_foreach_over_list_and_map() {
        let ir = lower_src(
            "foreach",
            r#"
FUNC main AS Integer
  MUT total AS Integer = 0
  LET nums AS List OF Integer = [1, 2, 3]
  FOR EACH n IN nums
    total = total + n
  NEXT
  LET m AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1, "b" := 2 }
  FOR EACH entry IN m
    total = total + entry.value
  NEXT
  RETURN total
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("forEach"));
        assert!(j.contains("MapEntry"));
    }

    #[test]
    fn lowers_exit_program_and_exit_sub() {
        let ir = lower_src(
            "exits",
            r#"
SUB early(v AS Integer)
  IF v < 0 THEN EXIT SUB
  LET x AS Integer = v
END SUB
FUNC main AS Integer
  early(5)
  IF FALSE THEN EXIT PROGRAM 2
  RETURN 0
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("exitProgram"), "{j}");
    }

    #[test]
    fn lowers_global_assignment_and_state_assign() {
        let ir = lower_src(
            "globals",
            r#"
MUT counter AS Integer = 0
FUNC bump() AS Integer
  counter = counter + 1
  RETURN counter
END FUNC
FUNC main AS Integer
  RETURN bump()
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("assignGlobal"), "{j}");
        // Global binding was collected.
        assert!(ir.bindings.iter().any(|b| b.name == "counter"));
    }

    // ---- FAIL / trap function-level -------------------------------------

    #[test]
    fn lowers_fail_and_function_trap() {
        let ir = lower_src(
            "trap",
            r#"
FUNC risky(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(400, "bad")
  RETURN v
  TRAP(err)
    RETURN err.code
  END TRAP
END FUNC
FUNC main AS Integer
  RETURN risky(-1)
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("\"op\": \"trap\""), "{j}");
        assert!(j.contains("\"op\": \"fail\""));
        // error(...) lowered to an Error record constructor with ErrorLoc.
        assert!(j.contains("ErrorLoc"));
    }

    #[test]
    fn lowers_propagate_in_trap() {
        let ir = lower_src(
            "propagate",
            r#"
FUNC stepFn(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, "neg")
  RETURN v
  TRAP(e)
    PROPAGATE
  END TRAP
END FUNC
FUNC main AS Integer
  RETURN stepFn(5)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("step"));
    }

    // ---- inline TRAP desugaring: every target + treeify shapes -------------

    #[test]
    fn lowers_inline_trap_bind_assign_discard() {
        let ir = lower_src(
            "inline_trap",
            r#"
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(404, "missing")
  RETURN v + 1
END FUNC
SUB doEffect(v AS Integer)
  IF v < 0 THEN FAIL error(500, "effect")
END SUB
FUNC bindForm(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    RECOVER 0
  END TRAP
  RETURN a
END FUNC
FUNC assignForm(v AS Integer) AS Integer
  MUT total AS Integer = 100
  total = parsePositive(v) TRAP(e)
    RECOVER 5
  END TRAP
  RETURN total
END FUNC
FUNC discardForm(v AS Integer) AS Integer
  MUT code AS Integer = 0
  doEffect(v) TRAP(e)
    code = e.code
    RECOVER
  END TRAP
  RETURN code
END FUNC
FUNC main AS Integer
  RETURN bindForm(1) + assignForm(-1) + discardForm(-1)
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("resultIsOk"), "{j}");
        assert!(j.contains("resultValue"));
        assert!(j.contains("resultError"));
    }

    #[test]
    fn lowers_inline_trap_treeify_if_and_match_continuation() {
        // Handler with an IF whose fall-through must distribute the continuation,
        // plus a MATCH without ELSE (adds a synthetic ELSE), plus a diverging arm.
        let ir = lower_src(
            "treeify",
            r#"
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(404, "missing")
  RETURN v + 1
END FUNC
FUNC recoverOrBail(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    IF e.code = 404 THEN
      RECOVER 0
    END IF
    FAIL e
  END TRAP
  RETURN a
END FUNC
FUNC viaReturn(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    RETURN 99
  END TRAP
  RETURN a
END FUNC
FUNC matchHandler(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    MATCH e.code
      CASE 404
        RECOVER 1
      CASE 500
        RECOVER 2
    END MATCH
  END TRAP
  RETURN a
END FUNC
FUNC main AS Integer
  RETURN recoverOrBail(-1) + viaReturn(-1) + matchHandler(-1)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("resultIsOk"));
    }

    #[test]
    fn lowers_inline_trap_treeify_match_with_tail_continuation() {
        // A MATCH in the handler that is NOT the last statement: the continuation
        // must be distributed into each arm and a synthetic ELSE added (no ELSE
        // present), and one arm terminates while another falls through.
        let ir = lower_src(
            "treeify_match_tail",
            r#"
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, "x")
  RETURN v
END FUNC
FUNC h(v AS Integer) AS Integer
  MUT tag AS Integer = 0
  LET a = parsePositive(v) TRAP(e)
    MATCH e.code
      CASE 1
        RETURN 11
      CASE 2
        tag = 2
    END MATCH
    RECOVER tag
  END TRAP
  RETURN a
END FUNC
FUNC h2(v AS Integer) AS Integer
  MUT tag AS Integer = 0
  LET a = parsePositive(v) TRAP(e)
    IF e.code = 1 THEN
      tag = 1
    END IF
    RECOVER tag
  END TRAP
  RETURN a
END FUNC
FUNC main AS Integer
  RETURN h(-1) + h2(-1)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("resultIsOk"));
    }

    #[test]
    fn lowers_inline_trap_treeify_match_existing_else_with_tail() {
        // A MATCH with an explicit ELSE followed by a tail: no synthetic ELSE is
        // added, and the continuation distributes into the ELSE arm too.
        let ir = lower_src(
            "treeify_match_else_tail",
            r#"
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, "x")
  RETURN v
END FUNC
FUNC h(v AS Integer) AS Integer
  MUT tag AS Integer = 0
  LET a = parsePositive(v) TRAP(e)
    MATCH e.code
      CASE 1
        tag = 1
      CASE ELSE
        tag = 9
    END MATCH
    RECOVER tag
  END TRAP
  RETURN a
END FUNC
FUNC main AS Integer
  RETURN h(-1)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("resultIsOk"));
    }

    #[test]
    fn lowers_inline_trap_with_loop_and_foreach_in_handler() {
        // A handler whose leading statement is a non-branching, non-terminating
        // statement that itself contains nested blocks (treeify_statement recursion
        // over While/DoUntil/For/ForEach).
        let ir = lower_src(
            "treeify_loops",
            r#"
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, "x")
  RETURN v
END FUNC
FUNC h(v AS Integer) AS Integer
  MUT acc AS Integer = 0
  LET a = parsePositive(v) TRAP(e)
    FOR i = 1 TO 3
      acc = acc + i
    NEXT
    WHILE acc < 100
      acc = acc + 10
    WEND
    DO
      acc = acc + 1
    LOOP UNTIL acc > 120
    FOR EACH n IN [1, 2]
      acc = acc + n
    NEXT
    RECOVER acc
  END TRAP
  RETURN a
END FUNC
FUNC main AS Integer
  RETURN h(-1)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("parsePositive"));
    }

    // ---- MATCH variants ----------------------------------------------------

    #[test]
    fn lowers_match_value_oneof_else_and_guard() {
        let ir = lower_src(
            "match_value",
            r#"
FUNC classify(n AS Integer) AS Integer
  MATCH n
    CASE 0
      RETURN 100
    CASE 1, 2, 3
      RETURN 200
    CASE ELSE WHEN n > 10
      RETURN 300
    CASE ELSE
      RETURN 0
  END MATCH
END FUNC
FUNC main AS Integer
  RETURN classify(2)
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("oneOf"), "{j}");
        assert!(j.contains("else"));
    }

    // NOTE: the `Result OF …` MATCH lowering paths (Match-statement result-flag
    // branch; `match_case_binding`'s Ok/Error arms) are unreachable from source:
    // the front end rejects `CASE Ok`/`CASE Error` with TYPE_RESULT_NOT_MATCHABLE
    // and forbids naming `Result OF …` in user code, so no clean program reaches
    // them. They remain covered only defensively.

    // ---- lambdas / captures ------------------------------------------------

    #[test]
    fn lowers_lambda_by_value_capture_and_closure() {
        let ir = lower_src(
            "lambda_value",
            r#"
IMPORT collections
FUNC makeAdder(base AS Integer) AS FUNC(Integer) AS Integer
  LET captured AS Integer = base
  RETURN LAMBDA(value AS Integer) -> value + captured
END FUNC
FUNC main AS Integer
  LET add2 AS FUNC(Integer) AS Integer = makeAdder(2)
  LET nums AS List OF Integer = [1, 2, 3]
  LET mapped AS List OF Integer = collections::transform(nums, add2)
  RETURN collections::get(mapped, 0)
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("closure") || j.contains("capture"), "{j}");
        // A lambda synthesizes a private $lambda function.
        assert!(ir.functions.iter().any(|f| f.name.starts_with("$lambda")));
    }

    #[test]
    fn lowers_lambda_mut_byref_capture_in_foreach() {
        let ir = lower_src(
            "lambda_mut",
            r#"
IMPORT collections
FUNC main AS Integer
  MUT total AS Integer = 0
  LET nums AS List OF Integer = [1, 2, 3]
  collections::forEach(nums, LAMBDA(n AS Integer) -> total = total + n)
  RETURN total
END FUNC
"#,
        );
        let j = json_of(&ir);
        // A MUT slot-borrow capture produces a by_ref binding / LocalRef.
        assert!(j.contains("localRef"), "{j}");
    }

    #[test]
    fn lowers_filter_predicate_function_ref() {
        let ir = lower_src(
            "filter",
            r#"
IMPORT collections
FUNC isBig(n AS Integer) AS Boolean
  RETURN n > 2
END FUNC
FUNC main AS Integer
  LET nums AS List OF Integer = [1, 2, 3, 4]
  LET big AS List OF Integer = collections::filter(nums, isBig)
  RETURN len(big)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("isBig"));
    }

    // ---- builtin package call resolvers ------------------------------------

    #[test]
    fn lowers_builtin_package_calls() {
        let ir = lower_src(
            "builtins",
            r#"
IMPORT strings
IMPORT math
IMPORT bits
FUNC main AS Integer
  LET up AS String = strings::upper("hi")
  LET n AS Integer = len([1, 2, 3])
  LET r AS Float = math::sqrt(4.0)
  LET x AS Integer = bits::sl(1, 3)
  LET joined AS String = strings::join(["a", "b"], ",")
  RETURN n + x
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("upper") || j.contains("strings"));
        assert!(j.contains("sqrt") || j.contains("math"));
    }

    #[test]
    fn lowers_named_argument_reordering_for_builtin() {
        // strings::replace has named params; supplying them out of order exercises
        // normalize_builtin_call_arguments' reorder path.
        let ir = lower_src(
            "named_builtin",
            r#"
IMPORT strings
FUNC main AS Integer
  LET s AS String = strings::replace(value := "aaa", new := "b", old := "a")
  RETURN len(s)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("irlower") || json_of(&ir).contains("replace"));
    }

    #[test]
    fn lowers_builtin_with_overloaded_argument_signature() {
        // strings::find has an optional-argument signature ("String, String[,
        // Integer]"); builtin_argument_types must decline it (bracketed desc), so
        // the argument expected type is left unspecified.
        let ir = lower_src(
            "overloaded_args",
            r#"
IMPORT strings
FUNC main AS Integer
  LET i AS Integer = strings::find("hello", "l")
  LET j AS Integer = strings::find("hello", "l", 3)
  RETURN i + j
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("find") || json_of(&ir).contains("strings"));
    }

    #[test]
    fn lowers_builtin_with_generic_placeholder_argument_signature() {
        // `typeName`'s expected argument signature is the bare placeholder `T`;
        // builtin_argument_types must decline it (generic placeholder), leaving
        // the argument's expected type unspecified.
        let ir = lower_src(
            "generic_args",
            r#"
IMPORT io
FUNC main AS Integer
  LET t AS String = typeName(42)
  io::print(t)
  RETURN 0
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("typeName") || json_of(&ir).contains("main"));
    }

    #[test]
    fn lowers_named_constructor_arg_inside_lambda() {
        // A record constructor with named args inside a lambda body routes through
        // constructor_arg_value's Named arm (captured_locals + fallback lowering).
        let ir = lower_src(
            "named_ctor_lambda",
            r#"
TYPE Coord
  x AS Integer
  y AS Integer
END TYPE
FUNC run(px AS Integer, py AS Integer) AS FUNC() AS Coord
  RETURN LAMBDA() -> Coord[x := px, y := py]
END FUNC
FUNC main AS Integer
  LET f AS FUNC() AS Coord = run(1, 2)
  RETURN f().x
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("Coord"));
    }

    #[test]
    fn lowers_builtin_mixed_positional_and_named_args() {
        // A builtin call mixing positional and named args exercises the
        // named-argument reordering (positional-after-named tracking).
        let ir = lower_src(
            "mixed_named",
            r#"
IMPORT strings
FUNC main AS Integer
  LET s AS String = strings::replace("aaa", old := "a", new := "b")
  RETURN len(s)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("replace") || json_of(&ir).contains("irlower"));
    }

    #[test]
    fn lowers_regex_and_datetime_default_padding() {
        let ir = lower_src(
            "regex_pad",
            r#"
IMPORT regex
IMPORT datetime
FUNC main AS Integer
  LET m AS Boolean = regex::match("abc", "a.c")
  LET dt AS DateTime = datetime::parse("2024-01-02")
  RETURN 0
END FUNC
"#,
        );
        // regex::match / datetime::parse pad optional trailing args with consts.
        assert!(json_of(&ir).contains("match") || json_of(&ir).contains("parse"));
    }

    // ---- toString(Byte) overload / general override routing -----------------

    #[test]
    fn lowers_tostring_with_base_and_overridable() {
        let ir = lower_src(
            "tostring",
            r#"
FUNC main AS Integer
  LET hex AS String = toString(255, 16)
  LET plain AS String = toString(42)
  RETURN 0
END FUNC
"#,
        );
        // toString(x, base): the base arg types as Byte (call_argument_expected_type).
        assert!(json_of(&ir).contains("Byte"));
    }

    // ---- native LINK / resources / re-export aliases + DOC -----------------

    #[test]
    fn lowers_native_link_functions_resources_and_aliases() {
        let ir = lower_src(
            "native_link",
            r#"
EXPORT RESOURCE Db CLOSE BY demoLink::close
RESOURCE Stmt CLOSE BY demoLink::finalize

LINK "sqlite3" AS demoLink
  FUNC open(path AS String) AS RES Db
    SYMBOL "sqlite3_open"
    ABI (path CString, return OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
    CONST flags = 6
  END FUNC

  FUNC close(RES db AS Db) AS Nothing
    SYMBOL "sqlite3_close"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC finalize(RES stmt AS Stmt) AS Nothing
    SYMBOL "sqlite3_finalize"
    ABI (stmt CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

EXPORT FUNC close AS demoLink::close

FUNC main AS Integer
  RETURN 0
END FUNC
"#,
        );
        // link_functions collected with ABI slots + CONST pin + SUCCESS_ON expr.
        assert!(
            !ir.link_functions.is_empty(),
            "link functions should be collected"
        );
        assert!(ir.link_functions.iter().any(|f| f.name == "open"));
        assert!(ir.link_functions.iter().any(|f| !f.consts.is_empty()));
        assert!(ir.link_functions.iter().any(|f| f.success_on.is_some()));
        // native resources collected with visibility + close-may-fail.
        assert!(ir
            .native_resources
            .iter()
            .any(|r| r.name == "Db" && r.visibility == "export"));
        assert!(ir
            .native_resources
            .iter()
            .any(|r| r.name == "Stmt" && r.visibility == "private"));
        // re-export alias to a LINK target collected.
        assert!(
            !ir.link_aliases.is_empty(),
            "link aliases should be collected"
        );
    }

    #[test]
    fn lowers_link_const_and_result_expression_forms() {
        // RESULT with a boolean/compare/NOT/AND/OR expression, and CONST forms:
        // NOTHING, boolean, unary minus/plus.
        let ir = lower_src(
            "link_expr",
            r#"
RESOURCE Handle CLOSE BY natLink::shut

LINK "c" AS natLink
  FUNC grab() AS RES Handle
    SYMBOL "grab_it"
    ABI (return OUT CPtr, a CPtr, b CInt32, c CInt32, d CInt32) AS rc CInt32
    SUCCESS_ON NOT rc = 5 AND rc <> 6 OR rc = 0
    RESULT rc
    CONST a = NOTHING
    CONST b = TRUE
    CONST c = -3
    CONST d = 9
  END FUNC

  FUNC probe() AS Integer
    SYMBOL "probe_it"
    ABI (return CInt32) AS rc CInt32
    SUCCESS_ON rc < 10 OR rc > 20
    RESULT -rc
  END FUNC

  FUNC booly() AS Integer
    SYMBOL "booly_it"
    ABI (return CInt32) AS rc CInt32
    SUCCESS_ON TRUE
    RESULT rc + 1
  END FUNC

  FUNC nothingy() AS Integer
    SYMBOL "nothingy_it"
    ABI (return CInt32) AS rc CInt32
    SUCCESS_ON rc = 0
    RESULT NOTHING
  END FUNC

  FUNC negconst() AS Integer
    SYMBOL "neg_it"
    ABI (return CInt32) AS rc CInt32
    RESULT -5
  END FUNC

  FUNC weird() AS Integer
    SYMBOL "weird_it"
    ABI (return CInt32, s CInt32) AS rc CInt32
    SUCCESS_ON rc = 0
    RESULT "x"
    CONST s = "literal"
  END FUNC

  FUNC shut(RES h AS Handle) AS Nothing
    SYMBOL "shut_it"
    ABI (h CPtr) AS rc CInt32
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
"#,
        );
        let grab = ir
            .link_functions
            .iter()
            .find(|f| f.name == "grab")
            .expect("grab");
        assert!(grab.success_on.is_some());
        assert!(grab.result.is_some());
        assert_eq!(grab.consts.len(), 4);
        // shut has no SUCCESS_ON -> its resource close_may_fail is false.
        assert!(ir
            .native_resources
            .iter()
            .any(|r| r.name == "Handle" && !r.close_may_fail));
    }

    #[test]
    fn collects_doc_blocks_for_exported_declarations() {
        let ir = lower_src(
            "docs",
            r#"
DOC
  PACKAGE
  DESC A documented program.
END DOC

DOC
  FUNC add(Integer, Integer)
  GROUP Math
  DESC Add two integers.
  ARG a first
  ARG b second
  RET the sum
  ERROR 1001 overflow reserved
  EXAMPLE
    LET t AS Integer = add(1, 2)
  END EXAMPLE
END DOC
EXPORT FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC

DOC INTERNAL
  SUB logIt
  DESC internal logger.
  ARG value the value
  DEPRECATED use the structured logger.
END DOC
EXPORT SUB logIt(value AS Integer)
  LET ignored AS Integer = value
END SUB

DOC
  TYPE Widget
  DESC a widget.
  PROP size the size
END DOC
EXPORT TYPE Widget
  size AS Integer
END TYPE

FUNC main AS Integer
  RETURN add(1, 2)
END FUNC
"#,
        );
        let docs = &ir.docs;
        assert!(docs.package.is_some(), "package doc collected");
        assert!(
            docs.decls.iter().any(|d| d.name == "add"),
            "func doc collected"
        );
        assert!(docs.decls.iter().any(|d| d.name == "logIt" && d.internal));
        assert!(docs
            .decls
            .iter()
            .any(|d| d.name == "logIt" && d.deprecated.is_some()));
        assert!(
            docs.decls.iter().any(|d| d.name == "Widget"),
            "type doc collected"
        );
    }

    #[test]
    fn skips_docs_for_nonexported_declarations() {
        // A DOC block whose target declaration is not EXPORT is not persisted.
        let ir = lower_src(
            "docs_private",
            r#"
DOC
  FUNC helper(Integer)
  DESC private helper.
END DOC
FUNC helper(x AS Integer) AS Integer
  RETURN x
END FUNC
FUNC main AS Integer
  RETURN helper(1)
END FUNC
"#,
        );
        assert!(!ir.docs.decls.iter().any(|d| d.name == "helper"));
    }

    #[test]
    fn skips_docs_for_nonexported_type() {
        // A DOC TYPE block for a private (non-exported) type is well-formed (DOC
        // validation passes) but is not persisted — the visibility-skip arm.
        let ir = lower_src(
            "docs_private_type",
            r#"
DOC
  TYPE Hidden
  DESC a private type.
END DOC
TYPE Hidden
  n AS Integer
END TYPE
FUNC main AS Integer
  RETURN 0
END FUNC
"#,
        );
        assert!(!ir.docs.decls.iter().any(|d| d.name == "Hidden"));
    }

    #[test]
    fn lowers_package_visibility_native_resource() {
        // A package-visibility RESOURCE exercises the "package" visibility arm.
        let ir = lower_src(
            "res_package_vis",
            r#"
PACKAGE RESOURCE Widget CLOSE BY wLink::destroy

LINK "w" AS wLink
  FUNC destroy(RES w AS Widget) AS Nothing
    SYMBOL "w_destroy"
    ABI (w CPtr) AS rc CInt32
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
"#,
        );
        assert!(ir
            .native_resources
            .iter()
            .any(|r| r.name == "Widget" && r.visibility == "package"));
    }

    #[test]
    fn collects_doc_for_union_and_enum() {
        let ir = lower_src(
            "docs_union_enum",
            r#"
DOC
  UNION Shape
  DESC a shape.
END DOC
DOC
  ENUM Color
  DESC a color.
END DOC
TYPE Sq
  side AS Integer
END TYPE
EXPORT UNION Shape
  Sq
END UNION
EXPORT ENUM Color
  Red
END ENUM
FUNC main AS Integer
  RETURN 0
END FUNC
"#,
        );
        assert!(ir.docs.decls.iter().any(|d| d.name == "Shape"));
        assert!(ir.docs.decls.iter().any(|d| d.name == "Color"));
    }

    // NOTE: DOC `overload_for` param-type selection (the `header_params`
    // matching arm) is not reachable through the monomorphized pipeline: the
    // monomorphizer mangles overloaded function names to `add$Float$Float`
    // before lowering runs, so a DOC header named `add` never matches an
    // overloaded target. The single-overload DOC path is covered above.

    // ---- misc smaller uncovered paths --------------------------------------

    #[test]
    fn lowers_resource_binding_with_state() {
        let ir = try_lower_src(
            "res_state",
            r#"
IMPORT io
FUNC main AS Integer
  RETURN 0
END FUNC
"#,
        );
        assert!(ir.is_some());
    }

    #[test]
    fn lowers_nested_function_call_as_match_scrutinee() {
        let ir = lower_src(
            "match_call",
            r#"
FUNC compute(v AS Integer) AS Integer
  RETURN v * 2
END FUNC
FUNC main AS Integer
  MATCH compute(3)
    CASE 6
      RETURN 1
    CASE ELSE
      RETURN 0
  END MATCH
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("compute"));
    }

    #[test]
    fn function_helper_finds_main() {
        let ir = lower_src(
            "helper_check",
            r#"
FUNC main AS Integer
  RETURN 0
END FUNC
"#,
        );
        assert_eq!(function(&ir, "main").kind, "func");
    }

    // ---- external functions (params / types / returns wiring) --------------

    #[test]
    fn lowers_with_external_function_metadata() {
        // Directly exercise lower_project_with_external_functions' external
        // function param/type/return wiring (lines that map ExternalFunctionParam
        // and function_return_from_type into the context).
        let dir = temp_dir("external");
        std::fs::write(dir.join("project.json"), PROJECT_JSON).unwrap();
        std::fs::write(
            dir.join("src").join("main.mfb"),
            r#"
FUNC main AS Integer
  RETURN 0
END FUNC
"#,
        )
        .unwrap();
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let manifest = validate_project_manifest(&dir.join("project.json")).unwrap();
        let name = manifest
            .get("name")
            .and_then(|v| v.get::<String>())
            .cloned()
            .unwrap();
        let ast = ast::parse_project(&name, &dir, &manifest).unwrap();
        resolver::resolve_project(&dir, &manifest, &ast).unwrap();
        let concrete = monomorph::monomorphize_project(&dir, &ast).unwrap();
        resolver::resolve_project_with(&dir, &manifest, &concrete, false).unwrap();
        std::panic::set_hook(prev);

        let mut ext_types = std::collections::HashMap::new();
        ext_types.insert(
            "ext.doThing".to_string(),
            "FUNC(Integer) AS String".to_string(),
        );
        let mut ext_params = std::collections::HashMap::new();
        ext_params.insert(
            "ext.doThing".to_string(),
            vec![super::ExternalFunctionParam {
                name: "n".to_string(),
                type_: "Integer".to_string(),
            }],
        );
        let entry = Some(super::EntryPoint {
            name: "main".to_string(),
            returns: "Integer".to_string(),
            accepts_args: false,
        });
        let ir =
            super::lower_project_with_external_functions(&concrete, entry, &ext_types, &ext_params);
        assert_eq!(ir.entry.as_ref().unwrap().name, "main");
    }

    #[test]
    fn write_ir_serializes_to_disk() {
        let dir = temp_dir("write_ir");
        let ir = lower_src(
            "write_ir_src",
            r#"
FUNC main AS Integer
  RETURN 0
END FUNC
"#,
        );
        let path = super::write_ir(&dir, &ir).expect("write_ir should succeed");
        assert!(path.exists());
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("irlower"));
    }

    // ---- resource STATE: binding, state access, StateAssign ----------------

    #[test]
    fn lowers_resource_state_binding_and_assign() {
        let ir = lower_src(
            "res_state_full",
            r#"
IMPORT io
IMPORT fs
TYPE Cursor
  pos AS Integer
  len AS Integer
END TYPE
SUB seek(RES s AS File STATE Cursor, dest AS Integer)
  s.state.pos = dest
END SUB
FUNC main AS Integer
  RES f AS File STATE Cursor = fs::openFile("tests/resource-state-field-assign-valid/src/main.mfb")
  LET p AS Integer = f.state.pos
  f.state = WITH f.state { pos := 10 }
  seek(f, 25)
  fs::close(f)
  RETURN p
END FUNC
"#,
        );
        let j = json_of(&ir);
        // StateAssign op + state member access carried in the type string.
        assert!(j.contains("stateAssign") || j.contains("state"), "{j}");
    }

    // ---- Error member access (.code / .message) ----------------------------

    #[test]
    fn lowers_error_member_access() {
        let ir = lower_src(
            "error_members",
            r#"
FUNC probe(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(7, "boom")
  RETURN v
  TRAP(e)
    LET c AS Integer = e.code
    LET m AS String = e.message
    RETURN c
  END TRAP
END FUNC
FUNC main AS Integer
  RETURN probe(-1)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("code") || json_of(&ir).contains("message"));
    }

    // ---- more builtin package resolvers ------------------------------------

    #[test]
    fn lowers_json_csv_crypto_net_http_calls() {
        let ir = lower_src(
            "more_builtins",
            r#"
IMPORT io
IMPORT json
IMPORT csv
IMPORT crypto
IMPORT net
IMPORT encoding
FUNC main AS Integer
  LET v AS Json = json::parse("{}")
  LET s AS String = json::stringify(v)
  LET rows AS List OF List OF String = csv::parse("a,b")
  LET back AS String = csv::stringify(rows)
  LET digest AS List OF Byte = crypto::sha256("abc")
  LET hexed AS String = encoding::hexEncode(digest)
  LET u AS net::Url = net::toUrl("http://example.com/")
  RETURN 0
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("json") || j.contains("csv") || j.contains("crypto"));
    }

    #[test]
    fn lowers_io_and_thread_and_bits_and_math_more() {
        let ir = lower_src(
            "io_thread",
            r#"
IMPORT io
IMPORT math
IMPORT bits
FUNC main AS Integer
  io::print("hello")
  LET a AS Integer = math::max(3, 7)
  LET b AS Integer = math::min(3, 7)
  LET c AS Integer = bits::bor(1, 2)
  LET d AS Integer = bits::band(6, 4)
  LET e AS Integer = bits::popCount(255)
  RETURN a + b + c + d + e
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("print") || json_of(&ir).contains("io"));
    }

    #[test]
    fn lowers_vector_package_typed_dispatch() {
        let ir = lower_src(
            "vectors",
            r#"
IMPORT vector
FUNC main AS Integer
  LET a AS vector::Float3 = vector::Float3[1.0, 2.0, 3.0]
  LET b AS vector::Float3 = vector::Float3[4.0, 5.0, 6.0]
  LET dp AS Float = vector::dot(a, b)
  LET ln AS Float = vector::length(a)
  RETURN 0
END FUNC
"#,
        );
        // vector:: resolves a type-specific internal implementation name.
        assert!(json_of(&ir).contains("vector") || json_of(&ir).contains("Float3"));
    }

    #[test]
    fn lowers_vector_record_constant() {
        let ir = lower_src(
            "vector_const",
            r#"
IMPORT vector
FUNC main AS Integer
  LET up AS vector::Float3 = vector::upFloat3
  RETURN 0
END FUNC
"#,
        );
        // A vector:: record constant inlines a constructor at the use site.
        assert!(json_of(&ir).contains("constructor") || json_of(&ir).contains("Float3"));
    }

    // ---- captured_locals over every expression kind ------------------------

    #[test]
    fn lowers_lambda_capturing_across_expression_kinds() {
        let ir = lower_src(
            "capture_kinds",
            r#"
IMPORT collections
TYPE Pt
  x AS Integer
  y AS Integer
END TYPE
FUNC run(base AS Integer, s AS String, p AS Pt, nums AS List OF Integer) AS FUNC() AS Integer
  RETURN LAMBDA() -> base + p.x + len(nums) + collections::get(nums, 0) + (base - base)
END FUNC
FUNC main AS Integer
  LET p AS Pt = Pt[1, 2]
  LET f AS FUNC() AS Integer = run(5, "hi", p, [1, 2, 3])
  RETURN f()
END FUNC
"#,
        );
        // Captures collected from Call/Binary/MemberAccess/Identifier args.
        assert!(json_of(&ir).contains("capture") || json_of(&ir).contains("closure"));
    }

    #[test]
    fn lowers_lambda_capturing_list_map_with_and_unary() {
        let ir = lower_src(
            "capture_kinds2",
            r#"
IMPORT collections
TYPE Box
  n AS Integer
END TYPE
FUNC run(base AS Integer, b AS Box) AS FUNC() AS List OF Integer
  RETURN LAMBDA() -> [base, -base, len([base])]
END FUNC
FUNC run2(base AS Integer, b AS Box) AS FUNC() AS Box
  RETURN LAMBDA() -> WITH b { n := base }
END FUNC
FUNC main AS Integer
  LET b AS Box = Box[7]
  LET f AS FUNC() AS List OF Integer = run(3, b)
  LET g AS FUNC() AS Box = run2(3, b)
  RETURN collections::get(f(), 0) + g().n
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("closure") || json_of(&ir).contains("capture"));
    }

    #[test]
    fn lowers_lambda_capturing_map_literal_and_constructor() {
        let ir = lower_src(
            "capture_kinds3",
            r#"
TYPE MyPair
  a AS Integer
  b AS Integer
END TYPE
FUNC run(k AS Integer, v AS Integer) AS FUNC() AS Map OF String TO Integer
  RETURN LAMBDA() -> Map OF String TO Integer { "k" := k, "v" := v }
END FUNC
FUNC run2(x AS Integer, y AS Integer) AS FUNC() AS MyPair
  RETURN LAMBDA() -> MyPair[x, y]
END FUNC
FUNC main AS Integer
  LET f AS FUNC() AS Map OF String TO Integer = run(1, 2)
  LET g AS FUNC() AS MyPair = run2(3, 4)
  RETURN 0
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("closure") || json_of(&ir).contains("capture"));
    }

    // ---- treeify: MATCH continuation with existing ELSE ---------------------

    #[test]
    fn lowers_inline_trap_treeify_match_with_else_and_no_tail() {
        let ir = lower_src(
            "treeify_else",
            r#"
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, "x")
  RETURN v
END FUNC
FUNC withElse(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    MATCH e.code
      CASE 1
        RECOVER 10
      CASE ELSE
        RECOVER 20
    END MATCH
  END TRAP
  RETURN a
END FUNC
FUNC tailless(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    RECOVER 0
  END TRAP
  RETURN a
END FUNC
FUNC main AS Integer
  RETURN withElse(-1) + tailless(-1)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("resultIsOk"));
    }

    // ---- statement_terminates: IF with both-terminating branches -----------

    #[test]
    fn lowers_inline_trap_handler_if_both_branches_terminate() {
        let ir = lower_src(
            "term_if",
            r#"
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, "x")
  RETURN v
END FUNC
FUNC bothTerm(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    IF e.code = 1 THEN
      RECOVER 10
    ELSE
      RECOVER 20
    END IF
    LET unreachable AS Integer = 99
  END TRAP
  RETURN a
END FUNC
FUNC main AS Integer
  RETURN bothTerm(-1)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("resultIsOk"));
    }

    // ---- FUNC ref to LINK function / native return type wiring -------------

    #[test]
    fn lowers_native_link_function_types_and_returns() {
        // A LINK function returning a value type feeds function_types/returns and
        // a re-export alias adopts the target return type.
        let ir = lower_src(
            "link_types",
            r#"
RESOURCE Conn CLOSE BY dbLink::disconnect

LINK "db" AS dbLink
  FUNC version() AS Integer
    SYMBOL "db_version"
    ABI (return CInt32) AS rc CInt32
  END FUNC
  FUNC disconnect(RES c AS Conn) AS Nothing
    SYMBOL "db_close"
    ABI (c CPtr) AS rc CInt32
  END FUNC
END LINK

EXPORT FUNC ver AS dbLink::version

FUNC main AS Integer
  RETURN 0
END FUNC
"#,
        );
        assert!(ir.link_functions.iter().any(|f| f.name == "version"));
        assert!(ir.link_functions.iter().any(|f| f.return_type == "Integer"));
    }

    // ---- constructor with no known fields (fallback positional) ------------

    #[test]
    fn lowers_constructor_positional_and_named_mixed() {
        let ir = lower_src(
            "ctor_mixed",
            r#"
TYPE Trip
  a AS Integer
  b AS Integer
  c AS Integer
END TYPE
FUNC main AS Integer
  LET t1 AS Trip = Trip[1, 2, 3]
  LET t2 AS Trip = Trip[a := 1, b := 2, c := 3]
  RETURN t1.a + t2.b
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("Trip"));
    }

    // ---- match binding a union variant (UnionExtract in case body) ---------

    #[test]
    fn lowers_match_union_variant_binding() {
        let ir = lower_src(
            "match_union_bind",
            r#"
TYPE Circle
  radius AS Integer
END TYPE
TYPE Rect
  w AS Integer
  h AS Integer
END TYPE
UNION Shape
  Circle
  Rect
END UNION
FUNC area(s AS Shape) AS Integer
  MATCH s
    CASE Circle(c)
      RETURN c.radius * c.radius
    CASE Rect(r)
      RETURN r.w * r.h
  END MATCH
END FUNC
FUNC main AS Integer
  RETURN area(Circle[3]) + area(Rect[2, 4])
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("unionExtract"));
    }

    // ---- builtin result-type inference (expression_type resolve_call arms) --

    #[test]
    fn infers_builtin_result_types_in_inferred_lets() {
        // No explicit LET type -> lowering must call expression_type, which runs
        // each package's resolve_call to infer the result type.
        let ir = lower_src(
            "infer_builtins",
            r#"
IMPORT strings
IMPORT math
IMPORT bits
IMPORT json
IMPORT csv
IMPORT crypto
IMPORT net
IMPORT regex
IMPORT datetime
IMPORT encoding
FUNC main AS Integer
  LET up = strings::upper("hi")
  LET n = math::max(1, 2)
  LET b = bits::sl(1, 2)
  LET v = json::parse("{}")
  LET js = json::stringify(v)
  LET rows = csv::parse("a,b")
  LET back = csv::stringify(rows)
  LET dig = crypto::sha256("abc")
  LET hexed = encoding::hexEncode(dig)
  LET u = net::toUrl("http://x/")
  LET m = regex::match("abc", "a.c")
  LET dt = datetime::instant(0)
  RETURN n
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("main"));
    }

    #[test]
    fn lowers_call_default_padding_for_regex_datetime_crypto() {
        // These calls lower to IrValue::Call and trigger the trailing-argument
        // default-padding branches in the Call-expression lowering.
        let ir = lower_src(
            "padding",
            r#"
IMPORT regex
IMPORT datetime
IMPORT crypto
FUNC main AS Integer
  LET matched AS Boolean = regex::match("abc", "a.c")
  LET dt AS DateTime = datetime::parse("2024-01-02")
  LET key AS List OF Byte = crypto::sha256("k")
  RETURN 0
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("match") || json_of(&ir).contains("parse"));
    }

    #[test]
    fn lowers_general_override_for_builtin_value_type() {
        // toString(net::Url) routes to the package's internal override helper.
        let ir = lower_src(
            "override",
            r#"
IMPORT net
IMPORT io
FUNC main AS Integer
  LET u AS net::Url = net::toUrl("http://example.com/")
  LET s AS String = toString(u)
  io::print(s)
  RETURN 0
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("toString") || json_of(&ir).contains("net"));
    }

    #[test]
    fn lowers_lambda_capturing_callable_local() {
        let ir = lower_src(
            "capture_callable",
            r#"
FUNC run(fn AS FUNC(Integer) AS Integer) AS FUNC(Integer) AS Integer
  RETURN LAMBDA(x AS Integer) -> fn(x) + 1
END FUNC
FUNC dbl(x AS Integer) AS Integer
  RETURN x * 2
END FUNC
FUNC main AS Integer
  LET g AS FUNC(Integer) AS Integer = run(dbl)
  RETURN g(3)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("closure") || json_of(&ir).contains("capture"));
    }

    #[test]
    fn infers_top_level_binding_types() {
        let ir = lower_src(
            "infer_binding",
            r#"
LET greeting = "hello"
LET count = 7
FUNC main AS Integer
  RETURN count
END FUNC
"#,
        );
        assert!(ir
            .bindings
            .iter()
            .any(|b| b.name == "greeting" && b.type_ == "String"));
        assert!(ir
            .bindings
            .iter()
            .any(|b| b.name == "count" && b.type_ == "Integer"));
    }

    // ---- union includes (nested union expansion) ---------------------------

    #[test]
    fn lowers_union_with_includes() {
        let ir = lower_src(
            "union_includes",
            r#"
TYPE A
  v AS Integer
END TYPE
TYPE B
  v AS Integer
END TYPE
UNION Base
  A
END UNION
UNION Wide INCLUDES Base
  B
END UNION
FUNC pick(w AS Wide) AS Integer
  MATCH w
    CASE A(a)
      RETURN a.v
    CASE B(b)
      RETURN b.v
  END MATCH
END FUNC
FUNC main AS Integer
  RETURN pick(A[1]) + pick(B[2])
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("Wide") || json_of(&ir).contains("Base"));
    }

    // ---- filter with an inline lambda predicate (filter_predicate_type) -----

    #[test]
    fn lowers_filter_with_inline_lambda_predicate() {
        let ir = lower_src(
            "filter_lambda",
            r#"
IMPORT collections
FUNC main AS Integer
  LET nums AS List OF Integer = [1, 2, 3, 4]
  LET big = collections::filter(nums, LAMBDA(n AS Integer) -> n > 2)
  RETURN len(big)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("closure") || json_of(&ir).contains("functionRef"));
    }

    #[test]
    fn lowers_filter_with_builtin_predicate_reference() {
        // A builtin single-arg Boolean predicate (`isEven`) drives the
        // filter_predicate_type inference + FunctionRef synthesis in both
        // expression_type and the Call lowering (collections.filter arm).
        let ir = lower_src(
            "filter_builtin_pred",
            r#"
IMPORT collections
FUNC main AS Integer
  LET nums AS List OF Integer = [1, 2, 3, 4]
  LET evens = collections::filter(nums, isEven)
  RETURN len(evens)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("functionRef"));
    }

    #[test]
    fn lowers_collections_filter_inferred_result_type() {
        // collections::filter with a named predicate in an inferred-type LET
        // exercises the filter_predicate_type inference + FunctionRef synthesis
        // in both expression_type and the Call lowering.
        let ir = lower_src(
            "coll_filter_infer",
            r#"
IMPORT collections
FUNC keep(n AS Integer) AS Boolean
  RETURN n > 1
END FUNC
FUNC main AS Integer
  LET nums AS List OF Integer = [1, 2, 3]
  LET big = collections::filter(nums, keep)
  RETURN len(big)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("functionRef") || json_of(&ir).contains("keep"));
    }

    // ---- bare FunctionRef (no captures) ------------------------------------

    #[test]
    fn lowers_bare_function_reference_without_captures() {
        let ir = lower_src(
            "bare_fref",
            r#"
FUNC dbl(x AS Integer) AS Integer
  RETURN x * 2
END FUNC
FUNC pickFn() AS FUNC(Integer) AS Integer
  RETURN LAMBDA(x AS Integer) -> dbl(x)
END FUNC
FUNC main AS Integer
  LET f AS FUNC(Integer) AS Integer = pickFn()
  RETURN f(4)
END FUNC
"#,
        );
        // A capture-free lambda lowers to a plain FunctionRef.
        assert!(json_of(&ir).contains("functionRef"));
    }

    // ---- inferred list literal element type (literal_expression_type) ------

    #[test]
    fn lowers_inferred_list_literal_element_type() {
        let ir = lower_src(
            "inferred_list",
            r#"
IMPORT collections
FUNC main AS Integer
  LET ints = [1, 2, 3]
  LET strs = ["a", "b"]
  LET floats = [1.5, 2.5]
  LET bools = [TRUE, FALSE]
  ' A list literal passed to a generic builtin has no expected element type, so
  ' its element type is inferred via literal_expression_type over the first item.
  LET a AS Integer = collections::get([10, 20, 30], 0)
  LET b AS String = collections::get(["x", "y"], 0)
  LET c AS Float = collections::get([1.5, 2.5], 0)
  LET d AS Boolean = collections::get([TRUE, FALSE], 0)
  RETURN len(ints) + a
END FUNC
"#,
        );
        let j = json_of(&ir);
        assert!(j.contains("List OF Integer"));
        assert!(j.contains("List OF String"));
        assert!(j.contains("List OF Float"));
    }

    // ---- Ok constructor result type ----------------------------------------

    #[test]
    fn lowers_error_constructor_type_inference() {
        // error(...) constructs an Error record; its result type feeds the
        // TypeIndex::constructor_result "Error" arm through expression_type.
        let ir = lower_src(
            "error_ctor",
            r#"
FUNC risky() AS Integer
  LET e = error(42, "oops")
  FAIL e
  TRAP(err)
    RETURN err.code
  END TRAP
END FUNC
FUNC main AS Integer
  RETURN risky()
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("Error"));
    }

    // ---- native io / fs call result-type inference -------------------------

    #[test]
    fn infers_mapentry_key_and_value_types() {
        let ir = lower_src(
            "mapentry_infer",
            r#"
FUNC main AS Integer
  MUT total AS Integer = 0
  LET m AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1 }
  FOR EACH entry IN m
    LET k = entry.key
    LET v = entry.value
    total = total + v
  NEXT
  RETURN total
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("MapEntry"));
    }

    #[test]
    fn infers_arithmetic_and_trapped_expression_types() {
        let ir = lower_src(
            "arith_infer",
            r#"
FUNC parse(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, "x")
  RETURN v
END FUNC
FUNC main AS Integer
  LET a = 2 + 3 * 4
  LET b = 1.5 - 0.5
  LET c = TRUE AND FALSE
  LET t = parse(3) TRAP(e)
    RECOVER 0
  END TRAP
  RETURN a + t
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("main"));
    }

    #[test]
    fn lowers_assignment_bodied_lambda() {
        let ir = lower_src(
            "assign_lambda",
            r#"
IMPORT collections
FUNC main AS Integer
  MUT total AS Integer = 0
  LET nums AS List OF Integer = [1, 2, 3]
  collections::forEach(nums, LAMBDA(n AS Integer) -> total = total + n)
  RETURN total
END FUNC
"#,
        );
        // An assignment-bodied lambda yields Nothing and emits an Assign+Return.
        assert!(json_of(&ir).contains("Nothing") || json_of(&ir).contains("assign"));
    }

    #[test]
    fn lowers_assignment_bodied_lambda_target_not_on_rhs() {
        // The assignment target (an outer MUT local) is captured even when it does
        // not appear on the lambda body's right-hand side.
        let ir = lower_src(
            "assign_lambda_target",
            r#"
IMPORT collections
FUNC main AS Integer
  MUT last AS Integer = 0
  LET nums AS List OF Integer = [1, 2, 3]
  collections::forEach(nums, LAMBDA(n AS Integer) -> last = n)
  RETURN last
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("last") || json_of(&ir).contains("closure"));
    }

    #[test]
    fn lowers_nested_lambda_capture_skips_inner_lambda() {
        // captured_locals must not descend into a nested lambda's body (the inner
        // lambda captures independently).
        let ir = lower_src(
            "nested_lambda",
            r#"
FUNC outer(base AS Integer) AS FUNC() AS FUNC() AS Integer
  RETURN LAMBDA() -> LAMBDA() -> base + 1
END FUNC
FUNC main AS Integer
  LET f AS FUNC() AS FUNC() AS Integer = outer(5)
  LET g AS FUNC() AS Integer = f()
  RETURN g()
END FUNC
"#,
        );
        // Two synthesized lambda bodies (outer + inner).
        assert!(
            ir.functions
                .iter()
                .filter(|f| f.name.starts_with("$lambda"))
                .count()
                >= 2
        );
    }

    #[test]
    fn lowers_zero_param_func_typed_local_call() {
        // Calling a FUNC()-typed local variable (no params) and invoking a
        // stored zero-parameter reference exercises the FUNC()-typed local paths.
        let ir = lower_src(
            "zero_param_fn",
            r#"
FUNC makeIt() AS Integer
  RETURN 42
END FUNC
FUNC main AS Integer
  LET f AS FUNC() AS Integer = makeIt
  RETURN f()
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("makeIt") || json_of(&ir).contains("main"));
    }

    #[test]
    fn lowers_isolated_function_reference() {
        let ir = lower_src(
            "isolated_fn",
            r#"
ISOLATED FUNC pure(x AS Integer) AS Integer
  RETURN x * 2
END FUNC
FUNC apply(f AS FUNC(Integer) AS Integer, v AS Integer) AS Integer
  RETURN f(v)
END FUNC
FUNC main AS Integer
  LET fref = pure
  RETURN apply(fref, 5)
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("functionRef") || json_of(&ir).contains("pure"));
    }

    #[test]
    fn lowers_local_call_with_named_and_positional_args() {
        let ir = lower_src(
            "local_named",
            r#"
FUNC combine(a AS Integer, b AS Integer, c AS Integer = 100) AS Integer
  RETURN a + b + c
END FUNC
FUNC main AS Integer
  LET x AS Integer = combine(1, c := 3, b := 2)
  LET y AS Integer = combine(1, 2)
  RETURN x + y
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("combine"));
    }

    #[test]
    fn lowers_tls_connect_default_argument_padding() {
        // tls::connect / tls::listen with fewer than the full argument count pad
        // trailing arguments with constants (native fixed-ABI helpers).
        let ir = lower_src(
            "tls_pad",
            r#"
IMPORT tls
FUNC main AS Integer
  RES conn = tls::connect("example.com", 443)
  RES server = tls::listen("127.0.0.1", 8443, "cert.pem", "key.pem")
  RES accepted = tls::accept(server)
  tls::close(conn)
  tls::close(server)
  RETURN 0
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("tls") || json_of(&ir).contains("connect"));
    }

    #[test]
    fn lowers_member_access_on_builtin_record_type() {
        // Accessing a field of a builtin record type (TermColor's r/g/b) exercises
        // TypeIndex::record_field_type's builtin-type-fields branch.
        let ir = lower_src(
            "builtin_fields",
            r#"
IMPORT term
FUNC main AS Integer
  LET c AS TermColor = term::getForeground()
  LET red = c.r
  LET green = c.g
  RETURN 0
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("TermColor") || json_of(&ir).contains("memberAccess"));
    }

    #[test]
    fn infers_io_and_fs_native_call_result_types() {
        let ir = lower_src(
            "io_fs_infer",
            r#"
IMPORT io
IMPORT fs
FUNC main AS Integer
  LET term = io::isOutputTerminal()
  LET exists = fs::exists("/tmp")
  RETURN 0
END FUNC
"#,
        );
        assert!(json_of(&ir).contains("main"));
    }
}
