use super::*;

// The single variant corpus (bug-342 C2): `variant_corpus` is a strict
// superset of every IrType/IrOp/IrValue/IrMatchPattern kind this suite
// needs (it adds For/DoUntil/StateAssign/ExitLoop/ContinueLoop/ExitProgram
// and the LINK tables on top), so both suites build from one place.
fn corpus_project() -> IrProject {
    crate::ir::variant_corpus_tests::variant_corpus()
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

/// bug-33: `Capture.index` was a `usize` written as a `u32`, so an index past
/// `u32::MAX` encoded its low 32 bits and decoded to a different slot. The
/// field is now the wire's `u32`, and the maximum value round-trips exactly.
#[test]
fn capture_index_round_trips_at_the_u32_boundary() {
    let mut project = corpus_project();
    project.functions[0].body.insert(
        0,
        IrOp::Bind {
            mutable: false,
            name: "$captured".to_string(),
            type_: ParameterType::Integer,
            value: Some(IrValue::Capture {
                index: u32::MAX,
                type_: crate::types::ParameterType::parse("Integer"),
                by_ref: false,
            }),
            explicit_type: false,
            loc: IrSourceLoc::default(),
        },
    );
    let decoded = decode_binary_repr(&encode_binary_repr(&project)).expect("decode");
    assert_eq!(project.to_json(), decoded.to_json());
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
        // bug-342 C3: on-contract kind is `func`/`sub`, not `function`.
        kind: "func".to_string(),
        isolated: false,
        params: vec![],
        returns: crate::types::ParameterType::parse("Integer"),
        body,
        file: "src/main.mfb".to_string(),
        resource_owners: HashMap::new(),
        loc: IrSourceLoc::default(),
    }
}

fn project_named(name: &str, functions: Vec<IrFunction>) -> IrProject {
    crate::ir::test_support::project_fixture(name, functions, vec![])
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
                        type_: crate::types::ParameterType::parse("Unknown"),
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
    prefix_package_symbols(&mut pkg, id, &Default::default());

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
                    type_: crate::types::ParameterType::parse("Unknown"),
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

// `prefix_package_symbols` must fold every LINK-table routing name into the
// identity prefix in lockstep with the regular functions: the wrapper-body
// routing name (`alias.func`), the `link_functions`/`link_cstructs` alias,
// and each re-export alias target.
#[test]
fn prefix_package_symbols_qualifies_link_tables() {
    let mut pkg = corpus_project();
    let name = pkg.name.clone();
    assert!(!pkg.link_functions.is_empty());
    assert!(!pkg.link_cstructs.is_empty());
    assert!(!pkg.link_aliases.is_empty());

    let id = "id123";
    prefix_package_symbols(&mut pkg, id, &Default::default());

    let want = format!("{id}.{name}.");
    assert!(
        pkg.link_functions[0].alias.starts_with(&want),
        "link function alias not prefixed: {}",
        pkg.link_functions[0].alias
    );
    assert!(
        pkg.link_cstructs[0].alias.starts_with(&want),
        "cstruct alias not prefixed: {}",
        pkg.link_cstructs[0].alias
    );
    assert!(
        pkg.link_aliases[0].1.starts_with(&want),
        "re-export alias target not prefixed: {}",
        pkg.link_aliases[0].1
    );
}

// `merge_package` de-duplicates the CSTRUCT table by `(alias, name)`: merging
// a package into a copy of itself leaves the table length unchanged (the
// `push_unique` predicate for `link_cstructs`).
#[test]
fn merge_package_dedups_link_cstructs() {
    let mut base = corpus_project();
    let incoming = corpus_project();
    let before = base.link_cstructs.len();
    assert!(before > 0);
    merge_package(&mut base, incoming);
    assert_eq!(base.link_cstructs.len(), before, "cstructs not deduped");
}

// bug-613: packages initialize dependencies-first and before the consumer,
// whatever the merge (manifest) order. `top` imports `left` and `right`, which
// both import `bottom`; `loner` depends on nothing. Merged in the worst order
// — importers before what they import — the global initializer must still
// store `bottom` first, keep `loner` in its merge position relative to the
// other roots, keep each package's declaration order, and end with the
// consumer's own bindings in theirs.
#[test]
fn package_bindings_initialize_dependencies_first() {
    use std::collections::HashSet;
    fn binding(name: &str) -> crate::ir::IrBinding {
        crate::ir::IrBinding {
            name: name.to_string(),
            visibility: "public".to_string(),
            mutable: false,
            type_: crate::types::ParameterType::Integer,
            value: None,
            loc: crate::ir::IrSourceLoc { line: 1, column: 1 },
            file: String::new(),
            explicit_type: true,
        }
    }
    fn set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }
    fn package(
        bindings: &[&str],
        exports: &[&str],
        references: &[&str],
    ) -> crate::ir::PackageInitialization {
        crate::ir::PackageInitialization {
            bindings: set(bindings),
            exports: set(exports),
            references: set(references),
        }
    }
    let packages = [
        package(&["i.top.T"], &["top.T"], &["left.l", "right.R"]),
        package(
            &["i.left.L1", "i.left.L2"],
            &["left.L1", "left.L2", "left.l"],
            &["bottom.b"],
        ),
        package(&["i.loner.X"], &["loner.X"], &["io.print"]),
        package(&["i.right.R"], &["right.R"], &["bottom.B"]),
        package(&["i.bottom.B"], &["bottom.B", "bottom.b"], &[]),
    ];
    let mut bindings: Vec<_> = [
        "own1",
        "own2",
        "i.top.T",
        "i.left.L2",
        "i.left.L1",
        "i.loner.X",
        "i.right.R",
        "i.bottom.B",
    ]
    .into_iter()
    .map(binding)
    .collect();
    crate::ir::order_bindings_dependencies_first(&mut bindings, &packages);
    let order: Vec<&str> = bindings.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(
        order,
        vec![
            "i.bottom.B",
            "i.left.L2",
            "i.left.L1",
            "i.right.R",
            "i.top.T",
            "i.loner.X",
            "own1",
            "own2",
        ]
    );
}

// bug-673: a builtin package's internalized bindings (`#JSON_DEPTH_LIMIT`)
// move ahead of the manifest packages' and the program's, keeping their own
// order; a user file's mangled PRIVATE name (`#<hash>$name`) is the program's
// and stays where it was.
#[test]
fn builtin_bindings_initialize_first() {
    fn binding(name: &str) -> crate::ir::IrBinding {
        crate::ir::IrBinding {
            name: name.to_string(),
            visibility: "public".to_string(),
            mutable: false,
            type_: crate::types::ParameterType::Integer,
            value: None,
            loc: crate::ir::IrSourceLoc { line: 1, column: 1 },
            file: String::new(),
            explicit_type: true,
        }
    }
    let private = crate::internal_name::mangle_private(
        &crate::internal_name::file_scope_hash("src/main.mfb"),
        "secret",
    );
    let mut bindings: Vec<_> = [
        "i.pkg.P",
        "own1",
        private.as_str(),
        "#JSON_DEPTH_LIMIT",
        "own2",
        "#COMPRESS_CRC32_TABLES",
    ]
    .into_iter()
    .map(binding)
    .collect();
    crate::ir::order_builtin_bindings_first(&mut bindings);
    let order: Vec<&str> = bindings.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(
        order,
        vec![
            "#JSON_DEPTH_LIMIT",
            "#COMPRESS_CRC32_TABLES",
            "i.pkg.P",
            "own1",
            private.as_str(),
            "own2",
        ]
    );
}

// A manifest package carries its own copy of each builtin it imports, so its
// builtin bindings reach the merge identity-prefixed
// (`<identity>.pkg.#JSON_DEPTH_LIMIT`). They initialize first too: the
// package's own `EXPORT LET doc AS json::Json = json::parse(…)` sorts
// before the injected `builtins/json.mfb` file's bindings in the package,
// and used to read the depth limit as zero ("nested too deeply"). A
// package's mangled PRIVATE binding (`<identity>.pkg.#<hash>$name`) is the
// package's and stays where it was.
#[test]
fn a_package_s_builtin_bindings_initialize_first() {
    fn binding(name: &str) -> crate::ir::IrBinding {
        crate::ir::IrBinding {
            name: name.to_string(),
            visibility: "public".to_string(),
            mutable: false,
            type_: crate::types::ParameterType::Integer,
            value: None,
            loc: crate::ir::IrSourceLoc { line: 1, column: 1 },
            file: String::new(),
            explicit_type: true,
        }
    }
    let private = format!(
        "1a4a95166428bc0c.pkg.{}",
        crate::internal_name::mangle_private(
            &crate::internal_name::file_scope_hash("src/lib.mfb"),
            "secret",
        )
    );
    let mut bindings: Vec<_> = [
        "1a4a95166428bc0c.pkg.DOC",
        private.as_str(),
        "1a4a95166428bc0c.pkg.#JSON_DEPTH_LIMIT",
        "own1",
        "#JSON_DEPTH_LIMIT",
    ]
    .into_iter()
    .map(binding)
    .collect();
    crate::ir::order_builtin_bindings_first(&mut bindings);
    let order: Vec<&str> = bindings.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(
        order,
        vec![
            "1a4a95166428bc0c.pkg.#JSON_DEPTH_LIMIT",
            "#JSON_DEPTH_LIMIT",
            "1a4a95166428bc0c.pkg.DOC",
            private.as_str(),
            "own1",
        ]
    );
}

// A stateful link function (`return_state_type`/`bind_state`) rides the
// optional STATE trailer, and its `BIND IN` block rides the positional
// record — both must survive the encode/decode round trip (plan-53 / plan-50-E).
#[test]
fn link_state_trailer_and_bind_in_round_trip() {
    let mut project = corpus_project();
    {
        let lf = &mut project.link_functions[0];
        lf.return_state_type = Some(crate::types::ParameterType::parse("OpenState"));
        lf.bind_state = Some("cfg".to_string());
        lf.bind_state_resource = None;
        lf.bind_in = vec![crate::ir::IrBindIn {
            slot: "cfg".to_string(),
            fields: vec![
                crate::ir::IrBindInField {
                    name: "a".to_string(),
                    param: Some("path".to_string()),
                    literal: None,
                },
                crate::ir::IrBindInField {
                    name: "b".to_string(),
                    param: None,
                    literal: Some(-7),
                },
            ],
        }];
    }
    let bytes = encode_binary_repr(&project);
    let decoded = decode_binary_repr(&bytes).expect("decode");
    assert_eq!(project.to_json(), decoded.to_json());
    let dlf = &decoded.link_functions[0];
    assert_eq!(
        dlf.return_state_type.as_ref().map(|t| t.name()).as_deref(),
        Some("OpenState")
    );
    assert_eq!(dlf.bind_state.as_deref(), Some("cfg"));
    assert_eq!(dlf.bind_in.len(), 1);
    assert_eq!(dlf.bind_in[0].fields[1].literal, Some(-7));
}

// bug-291: a `FloatBlocked` resource owner is unreachable on the wire
// (`ir::verify` rejects a blocked float pre-encode), but the encoder still
// serializes it as a plain `Local` (tag 0) so the format needs no new tag.
#[test]
fn float_blocked_resource_owner_encodes_as_local() {
    let mut project = corpus_project();
    project.functions[0].resource_owners.insert(
        "blocked".to_string(),
        crate::ir::resource_escape::ResOwner::FloatBlocked("xs".to_string()),
    );
    let bytes = encode_binary_repr(&project);
    let decoded = decode_binary_repr(&bytes).expect("decode");
    assert!(matches!(
        decoded.functions[0].resource_owners.get("blocked"),
        Some(crate::ir::resource_escape::ResOwner::Local)
    ));
}

// A per-CSTRUCT field count is an attacker-controlled u32, so it must be
// bounded independently of the struct count (decode_cstructs field cap).
#[test]
fn rejects_an_absurd_cstruct_field_count() {
    let project = crate::ir::test_support::project_fixture("plain", vec![], vec![]);
    let mut bytes = encode_binary_repr(&project);
    // Append a trailer: 0 link functions, 0 aliases, then 1 CSTRUCT whose
    // (alias, name, maps_to) are empty strings and whose field count is huge.
    bytes.extend_from_slice(&0u32.to_le_bytes()); // link functions
    bytes.extend_from_slice(&0u32.to_le_bytes()); // aliases
    bytes.extend_from_slice(&1u32.to_le_bytes()); // one CSTRUCT
    bytes.extend_from_slice(&0u32.to_le_bytes()); // alias: ""
    bytes.extend_from_slice(&0u32.to_le_bytes()); // name: ""
    bytes.extend_from_slice(&0u32.to_le_bytes()); // maps_to: ""
    bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // field count
    let err = match decode_binary_repr(&bytes) {
        Err(err) => err,
        Ok(_) => panic!("an absurd CSTRUCT field count must be rejected"),
    };
    assert!(err.contains("limit"), "{err}");
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
/// A project whose single function body exercises the JSON-emission arms the
/// `corpus_project` above does not: `For`, `DoUntil`, `ExitLoop`,
/// `ContinueLoop`, `ExitProgram`, `Fail`, `StateAssign`, and a by-value
/// `Capture`. Covers the remaining `IrOp::to_json` / `loop_kind_name` arms.
fn json_extra_ops_project() -> IrProject {
    let body = vec![
        IrOp::For {
            name: "i".to_string(),
            type_: ParameterType::Integer,
            start: IrValue::Const {
                type_: crate::types::ParameterType::parse("Integer"),
                value: "0".to_string(),
            },
            end: IrValue::Const {
                type_: crate::types::ParameterType::parse("Integer"),
                value: "10".to_string(),
            },
            step: IrValue::Const {
                type_: crate::types::ParameterType::parse("Integer"),
                value: "1".to_string(),
            },
            body: vec![
                IrOp::ExitLoop {
                    kind: LoopKind::For,
                    loc: IrSourceLoc::default(),
                },
                IrOp::ContinueLoop {
                    kind: LoopKind::For,
                    loc: IrSourceLoc::default(),
                },
            ],
            loc: IrSourceLoc::default(),
        },
        IrOp::DoUntil {
            body: vec![IrOp::ContinueLoop {
                kind: LoopKind::Do,
                loc: IrSourceLoc::default(),
            }],
            condition: IrValue::Local("b".to_string()),
            loc: IrSourceLoc::default(),
        },
        IrOp::StateAssign {
            resource: "file".to_string(),
            value: IrValue::Local("open".to_string()),
            loc: IrSourceLoc::default(),
        },
        IrOp::Eval {
            value: IrValue::Closure {
                name: "lam".to_string(),
                type_: crate::types::ParameterType::parse("() -> Integer"),
                captures: vec![IrValue::Capture {
                    index: 0,
                    type_: crate::types::ParameterType::parse("Integer"),
                    by_ref: false,
                }],
            },
            loc: IrSourceLoc::default(),
        },
        IrOp::Fail {
            error: IrValue::Local("e".to_string()),
            loc: IrSourceLoc::default(),
        },
        IrOp::ExitProgram {
            code: IrValue::Const {
                type_: crate::types::ParameterType::parse("Integer"),
                value: "1".to_string(),
            },
            loc: IrSourceLoc::default(),
        },
    ];
    project_named("extras", vec![fn_named("main", body)])
}

#[test]
fn json_covers_loop_state_and_capture_ops() {
    let json = json_extra_ops_project().to_json();
    assert!(json.contains("\"op\": \"for\""), "{json}");
    assert!(json.contains("\"op\": \"doUntil\""), "{json}");
    assert!(json.contains("\"op\": \"exitLoop\""), "{json}");
    assert!(json.contains("\"op\": \"continueLoop\""), "{json}");
    assert!(json.contains("\"op\": \"exitProgram\""), "{json}");
    assert!(json.contains("\"op\": \"fail\""), "{json}");
    assert!(json.contains("\"op\": \"stateAssign\""), "{json}");
    // by-value capture omits the `byRef` field.
    assert!(json.contains("\"kind\": \"capture\""), "{json}");
    assert!(!json.contains("\"byRef\""), "{json}");
    // Loop-kind names for `for` and `do` are emitted.
    assert!(json.contains("\"loop\": \"for\""), "{json}");
    assert!(json.contains("\"loop\": \"do\""), "{json}");
}

#[test]
fn json_binding_and_param_null_value_and_default() {
    // A binding with no value and a param with no default both emit `null`,
    // covering the `unwrap_or_else` "null" arms.
    let mut project = project_named("nulls", vec![fn_named("main", vec![])]);
    project.bindings.push(IrBinding {
        name: "g".to_string(),
        visibility: "public".to_string(),
        mutable: true,
        type_: crate::types::ParameterType::parse("Integer"),
        value: None,
        loc: IrSourceLoc::default(),
        file: String::new(),
        explicit_type: false,
    });
    project.functions[0].params.push(IrParam {
        name: "p".to_string(),
        type_: crate::types::ParameterType::parse("Integer"),
        default: None,
        loc: IrSourceLoc::default(),
    });
    let json = project.to_json();
    assert!(json.contains("\"value\": null"), "{json}");
    assert!(json.contains("\"default\": null"), "{json}");
}

#[test]
fn json_bind_with_null_value_emits_null() {
    // A `Bind` op with no value takes the `null` arm.
    let body = vec![IrOp::Bind {
        mutable: false,
        name: "x".to_string(),
        type_: ParameterType::Integer,
        value: None,
        loc: IrSourceLoc::default(),
        explicit_type: true,
    }];
    let json = project_named("bindnull", vec![fn_named("main", body)]).to_json();
    assert!(json.contains("\"op\": \"bind\""), "{json}");
    assert!(json.contains("\"value\": null"), "{json}");
}

#[test]
fn visibility_name_maps_all_variants() {
    assert_eq!(visibility_name(Visibility::Private), "private");
    assert_eq!(visibility_name(Visibility::Public), "public");
    assert_eq!(visibility_name(Visibility::Export), "export");
}
