use super::*;

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
