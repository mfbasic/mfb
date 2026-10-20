use super::*;
use crate::ast::Visibility;
use crate::types::ParameterType;

fn func(body: Vec<HirStatement>) -> HirFunction {
    HirFunction {
        visibility: Visibility::Private,
        isolated: false,
        name: "f".to_string(),
        template_params: Vec::new(),
        params: Vec::new(),
        returns: ParameterType::parse("Integer"),
        return_resource: false,
        return_state_type: None,
        return_stray_state: None,
        body,
        trap: None,
        line: 1,
        internal_origin: false,
        instantiated_at: Vec::new(),
    }
}

fn res(name: &str, value: HirExpression) -> HirStatement {
    HirStatement::Let {
        mutable: false,
        resource: true,
        state_type: None,
        name: name.to_string(),
        type_: ParameterType::parse("fs.File"),
        explicit_type: true,
        value: Some(value),
        line: 1,
    }
}

fn list(name: &str, value: HirExpression) -> HirStatement {
    HirStatement::Let {
        mutable: true,
        resource: false,
        state_type: None,
        name: name.to_string(),
        type_: ParameterType::parse("List OF File"),
        explicit_type: true,
        value: Some(value),
        line: 1,
    }
}

fn open() -> HirExpression {
    HirExpression::Call {
        callee: "fs.openFile".to_string(),
        arguments: vec![HirCallArg::Positional(HirExpression::String(
            "p".to_string(),
        ))],
        line: 1,
        column: 1,
    }
}

fn append(collection: &str, element: &str) -> HirExpression {
    HirExpression::Call {
        callee: "collections.append".to_string(),
        arguments: vec![
            HirCallArg::Positional(HirExpression::Identifier(collection.to_string())),
            HirCallArg::Positional(HirExpression::Identifier(element.to_string())),
        ],
        line: 1,
        column: 1,
    }
}

fn ident(name: &str) -> HirExpression {
    HirExpression::Identifier(name.to_string())
}

// -----------------------------------------------------------------------
// plan-114-C Phase 2 — the record edges.
//
// A record is a container in the same sense a collection is: a value that
// holds handle pointers and whose binding has a scope. These mirror the
// collection cases above one for one, so a divergence between the two
// container kinds shows up as a test that passes for lists and fails for
// records.
// -----------------------------------------------------------------------

/// A record binding, `MUT`/`LET` per `mutable`.
fn holder(name: &str, mutable: bool, value: HirExpression) -> HirStatement {
    HirStatement::Let {
        mutable,
        resource: false,
        state_type: None,
        name: name.to_string(),
        type_: ParameterType::declared("Holder"),
        explicit_type: true,
        value: Some(value),
        line: 1,
    }
}

/// `Holder[handle := <element>]`
fn construct(element: &str) -> HirExpression {
    HirExpression::Constructor {
        type_: ParameterType::declared("Holder"),
        arguments: vec![HirConstructorArg::Named {
            name: "handle".to_string(),
            value: ident(element),
            line: 1,
        }],
    }
}

/// `WITH <target> { handle := <element> }`
fn with_update(target: &str, element: &str) -> HirExpression {
    HirExpression::WithUpdate {
        target: Box::new(ident(target)),
        updates: vec![crate::hir::HirRecordUpdate {
            field: "handle".to_string(),
            value: ident(element),
            line: 1,
        }],
    }
}

#[test]
fn inner_resource_floats_to_an_outer_record_via_with() {
    // MUT h = Holder[...]; WHILE { RES f; h = WITH h { handle := f } }
    // The record twin of `inner_resource_floats_to_outer_collection`.
    let result = analyze_function(&func(vec![
        holder("h", true, construct("nothing")),
        HirStatement::While {
            kind: crate::ast::LoopKind::While,
            condition: HirExpression::Boolean(true),
            body: vec![
                res("f", open()),
                HirStatement::Assign {
                    name: "h".to_string(),
                    value: with_update("h", "f"),
                    line: 1,
                },
            ],
            line: 1,
        },
    ]));
    assert_eq!(result.owner("f"), ResOwner::Float("h".to_string()));
    assert!(result.floats("f"));
}

#[test]
fn same_scope_record_does_not_float() {
    // RES f; LET h = Holder[handle := f] — same scope, so ownership stays
    // local, exactly as for a list literal at the same depth.
    let result = analyze_function(&func(vec![
        res("f", open()),
        holder("h", false, construct("f")),
    ]));
    assert_eq!(result.owner("f"), ResOwner::Local);
    assert!(!result.floats("f"));
}

#[test]
fn a_constructor_nested_in_a_list_routes_to_the_list() {
    // MUT xs = []; WHILE { RES f; xs = append(xs, Holder[handle := f]) }
    // No extra scan arm is needed for this: `scan_element` falls through to
    // `scan_collection_expr`, which now has a Constructor arm, so the
    // resource reaches the LIST binding (the outermost container).
    let result = analyze_function(&func(vec![
        list("xs", HirExpression::ListLiteral(vec![])),
        HirStatement::While {
            kind: crate::ast::LoopKind::While,
            condition: HirExpression::Boolean(true),
            body: vec![
                res("f", open()),
                HirStatement::Assign {
                    name: "xs".to_string(),
                    value: HirExpression::Call {
                        callee: "collections.append".to_string(),
                        arguments: vec![
                            HirCallArg::Positional(ident("xs")),
                            HirCallArg::Positional(construct("f")),
                        ],
                        line: 1,
                        column: 1,
                    },
                    line: 1,
                },
            ],
            line: 1,
        },
    ]));
    assert_eq!(result.owner("f"), ResOwner::Float("xs".to_string()));
}

#[test]
fn a_record_with_no_resource_argument_routes_nothing() {
    // The regression guard: an ordinary record must not acquire an
    // owned-list just because the Constructor arm now exists.
    let result = analyze_function(&func(vec![
        res("f", open()),
        holder("h", false, construct("unrelated")),
    ]));
    assert_eq!(result.owner("f"), ResOwner::Local);
    assert!(!result.floats("f"));
}

#[test]
fn a_positional_constructor_argument_routes_the_same_as_a_named_one() {
    // `Holder[f]` and `Holder[handle := f]` are the same edge; the scan must
    // not see only the by-field spelling (§4.1 reads both arms).
    let result = analyze_function(&func(vec![
        list("xs", HirExpression::ListLiteral(vec![])),
        HirStatement::While {
            kind: crate::ast::LoopKind::While,
            condition: HirExpression::Boolean(true),
            body: vec![
                res("f", open()),
                HirStatement::Assign {
                    name: "xs".to_string(),
                    value: HirExpression::Call {
                        callee: "collections.append".to_string(),
                        arguments: vec![
                            HirCallArg::Positional(ident("xs")),
                            HirCallArg::Positional(HirExpression::Constructor {
                                type_: ParameterType::declared("Holder"),
                                arguments: vec![HirConstructorArg::Positional(ident("f"))],
                            }),
                        ],
                        line: 1,
                        column: 1,
                    },
                    line: 1,
                },
            ],
            line: 1,
        },
    ]));
    assert_eq!(result.owner("f"), ResOwner::Float("xs".to_string()));
}

#[test]
fn with_update_carries_the_targets_existing_contents() {
    // `h2 = WITH h1 { … }` must route h1's contents into h2 the way
    // insertion argument 0 does, or a resource already held by h1 would be
    // lost when the updated copy outlives it.
    let result = analyze_function(&func(vec![
        holder("outer", true, construct("nothing")),
        HirStatement::While {
            kind: crate::ast::LoopKind::While,
            condition: HirExpression::Boolean(true),
            body: vec![
                res("f", open()),
                holder("inner", false, construct("f")),
                HirStatement::Assign {
                    name: "outer".to_string(),
                    value: with_update("inner", "f"),
                    line: 1,
                },
            ],
            line: 1,
        },
    ]));
    // `f` reaches `outer`, the outermost container that references it.
    assert_eq!(result.owner("f"), ResOwner::Float("outer".to_string()));
}

/// bug-291's record twin: a RETURNED record declared *after* the resource it
/// carries cannot honour the float (its owned-list does not exist yet when
/// the resource is produced), so it must be reported rather than degraded.
///
/// This is the case that needs the type table. Degrading it to `Local` is a
/// silent miscompile — the callee closes the handle at its own scope while
/// the returned record still carries it, and the caller's adopted list
/// closes it a second time.

/// plan-114-C, found by running the returned-record shape rather than
/// reasoning about it: an **inferred** record binding had no `decl_type`, so
/// the bug-291 ordering gate could not see the container and degraded to
/// `ResOwner::Local`. That is the silent double close the gate exists to
/// prevent -- measured before the fix as a caller write raising
/// `7-703-0004 Resource handle is already closed`.
///
/// A record CONSTRUCTOR names its own type, so an inferred binding still has
/// a knowable one; recording it from the initializer is what makes the gate
/// fire.
#[test]
fn an_inferred_record_binding_is_still_seen_by_the_ordering_gate() {
    let mut records = HashSet::new();
    records.insert(ParameterType::declared("Holder"));
    // `RES f = …; LET h = Holder[handle := f]; RETURN h` -- note `h` carries
    // NO `AS Holder` annotation, which is the whole point.
    let inferred = HirStatement::Let {
        mutable: false,
        resource: false,
        state_type: None,
        name: "h".to_string(),
        type_: ParameterType::declared("Holder"),
        explicit_type: false,
        value: Some(construct("f")),
        line: 1,
    };
    let result = analyze_function_with(
        &func(vec![
            res("f", open()),
            inferred,
            HirStatement::Return {
                value: Some(ident("h")),
                line: 1,
            },
        ]),
        &records,
        &HashSet::new(),
    );
    assert_eq!(
        result.owner("f"),
        ResOwner::FloatBlocked("h".to_string()),
        "an inferred `LET h = Holder[…]` must still be recognised as the \
             container; degrading to Local is a double close"
    );
}

/// The other half of the same fix, and the guard against over-rejecting it:
/// a `RES` **parameter** is never blocked by ordering. The rule's hazard is
/// that the container's owned-list does not exist yet *when the resource is
/// produced* -- and a parameter is not produced here at all. The caller owns
/// and closes it.
///
/// Measured working end to end: `FUNC wrap(f AS fs::File) AS Holder`
/// returning `Holder[…, f]`, with the caller writing through the returned
/// record's handle after `wrap` returns.
#[test]
fn a_res_parameter_is_never_blocked_by_return_order() {
    let mut records = HashSet::new();
    records.insert(ParameterType::declared("Holder"));
    let mut f = func(vec![
        HirStatement::Let {
            mutable: false,
            resource: false,
            state_type: None,
            name: "h".to_string(),
            type_: ParameterType::declared("Holder"),
            explicit_type: false,
            value: Some(construct("p")),
            line: 1,
        },
        HirStatement::Return {
            value: Some(ident("h")),
            line: 1,
        },
    ]);
    f.params = vec![crate::hir::HirParam {
        name: "p".to_string(),
        type_: ParameterType::parse("fs.File"),
        resource: true,
        state_type: None,
        stray_state: None,
        default: None,
        line: 1,
    }];
    let result = analyze_function_with(&f, &records, &HashSet::new());
    assert_ne!(
        result.owner("p"),
        ResOwner::FloatBlocked("h".to_string()),
        "a resource parameter is owned by the caller; returning a record that \
             carries it transfers nothing and must not be refused"
    );
}

#[test]
fn a_returned_record_declared_after_its_resource_is_blocked() {
    let body = vec![
        res("f", open()),
        holder("h", false, construct("f")),
        HirStatement::Return {
            value: Some(ident("h")),
            line: 1,
        },
    ];
    let mut records = HashSet::new();
    records.insert(ParameterType::declared("Holder"));

    let blocked = analyze_function_with(&func(body.clone()), &records, &HashSet::new());
    assert_eq!(
        blocked.owner("f"),
        ResOwner::FloatBlocked("h".to_string()),
        "a returned record declared after its resource must be reported, \
             not degraded to Local"
    );

    // And the reason the table is needed at all: with no record-type
    // knowledge the very same program degrades to `Local`, which is the
    // bug-291 double close. This half is what makes the test prove the
    // threading, not just the gate.
    let unaware = analyze_function_with(&func(body), &HashSet::new(), &HashSet::new());
    assert_eq!(
        unaware.owner("f"),
        ResOwner::Local,
        "without the record table the gate cannot see the container — this \
             is why ir::lower must pass TypeIndex::res_field_record_types"
    );
}

/// The guard on the guard: a returned record with NO `RES` field must not be
/// blocked. Reporting an ordering problem for a container that cannot own a
/// resource would be advice that does not fix the program — the same reason
/// a bare `List OF File` is skipped.
#[test]
fn a_returned_record_without_a_res_field_is_not_blocked() {
    let result = analyze_function_with(
        &func(vec![
            res("f", open()),
            holder("h", false, construct("unrelated")),
            HirStatement::Return {
                value: Some(ident("h")),
                line: 1,
            },
        ]),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert_eq!(result.owner("f"), ResOwner::Local);
}

/// A record declared BEFORE its resource and returned floats normally — the
/// ordering gate must not fire just because a record is involved.
#[test]
fn a_returned_record_declared_before_its_resource_floats() {
    let mut records = HashSet::new();
    records.insert(ParameterType::declared("Holder"));
    let result = analyze_function_with(
        &func(vec![
            holder("h", true, construct("nothing")),
            res("f", open()),
            HirStatement::Assign {
                name: "h".to_string(),
                value: with_update("h", "f"),
                line: 1,
            },
            HirStatement::Return {
                value: Some(ident("h")),
                line: 1,
            },
        ]),
        &records,
        &HashSet::new(),
    );
    assert_eq!(result.owner("f"), ResOwner::Float("h".to_string()));
}

#[test]
fn same_scope_collection_does_not_float() {
    // RES f; LET xs = [f] — f and xs share a scope, so ownership stays local.
    let result = analyze_function(&func(vec![
        res("f", open()),
        list("xs", HirExpression::ListLiteral(vec![ident("f")])),
    ]));
    assert_eq!(result.owner("f"), ResOwner::Local);
    assert!(!result.floats("f"));
}

#[test]
fn inner_resource_floats_to_outer_collection() {
    // MUT xs = []; WHILE { RES f; xs = append(xs, f) } — f floats to xs.
    let result = analyze_function(&func(vec![
        list("xs", HirExpression::ListLiteral(vec![])),
        HirStatement::While {
            kind: crate::ast::LoopKind::While,
            condition: HirExpression::Boolean(true),
            body: vec![
                res("f", open()),
                HirStatement::Assign {
                    name: "xs".to_string(),
                    value: append("xs", "f"),
                    line: 1,
                },
            ],
            line: 1,
        },
    ]));
    assert_eq!(result.owner("f"), ResOwner::Float("xs".to_string()));
    assert!(result.floats("f"));
}

fn call(callee: &str, arguments: Vec<HirExpression>) -> HirExpression {
    HirExpression::Call {
        callee: callee.to_string(),
        arguments: arguments.into_iter().map(HirCallArg::Positional).collect(),
        line: 1,
        column: 1,
    }
}

/// `MUT xs = []; WHILE { RES f; xs = <value> }` — the owner `f` gets.
fn owner_after_inner_assign(value: HirExpression) -> ResOwner {
    analyze_function(&func(vec![
        list("xs", HirExpression::ListLiteral(vec![])),
        HirStatement::While {
            kind: crate::ast::LoopKind::While,
            condition: HirExpression::Boolean(true),
            body: vec![
                res("f", open()),
                HirStatement::Assign {
                    name: "xs".to_string(),
                    value,
                    line: 1,
                },
            ],
            line: 1,
        },
    ]))
    .owner("f")
}

/// bug-933: a re-packaging member around the insertion still routes the
/// handle into the outer list, so it floats there instead of closing early.
#[test]
fn a_resource_floats_through_a_repackaging_member() {
    let float = ResOwner::Float("xs".to_string());
    let take = call(
        "collections.take",
        vec![append("xs", "f"), HirExpression::Number("10".to_string())],
    );
    assert_eq!(owner_after_inner_assign(take), float);
    let drop = call(
        "collections.drop",
        vec![append("xs", "f"), HirExpression::Number("0".to_string())],
    );
    assert_eq!(owner_after_inner_assign(drop), float);
    let flatten = call(
        "collections.flatten",
        vec![HirExpression::ListLiteral(vec![
            ident("xs"),
            HirExpression::ListLiteral(vec![ident("f")]),
        ])],
    );
    assert_eq!(owner_after_inner_assign(flatten), float);
}

/// bug-933's arm admits only members whose result can carry an argument's
/// values. A scalar result (`findIndex`) or a bare element (`get`) is no
/// container, so assigning one to an outer binding floats nothing to it.
/// An element read placed INTO a container does carry its argument's handle.
#[test]
fn a_scalar_or_element_result_is_not_a_float_target() {
    use CollectionsCarry::{Container, Element, Nothing};
    assert_eq!(collections_carry("collections.take"), Container);
    assert_eq!(
        collections_carry("#collections_take$RES$fs$File"),
        Container
    );
    assert_eq!(collections_carry("collections.flatten"), Container);
    assert_eq!(collections_carry("collections.merge"), Container);
    assert_eq!(collections_carry("collections.values"), Container);
    assert_eq!(collections_carry("collections.findIndex"), Nothing);
    assert_eq!(collections_carry("collections.get"), Element);
    assert_eq!(collections_carry("strings.mid"), Nothing);
    let result = analyze_function(&func(vec![
        HirStatement::Let {
            mutable: true,
            resource: false,
            state_type: None,
            name: "n".to_string(),
            type_: ParameterType::parse("Integer"),
            explicit_type: true,
            value: Some(HirExpression::Number("0".to_string())),
            line: 1,
        },
        HirStatement::While {
            kind: crate::ast::LoopKind::While,
            condition: HirExpression::Boolean(true),
            body: vec![
                res("f", open()),
                list("ys", HirExpression::ListLiteral(vec![ident("f")])),
                HirStatement::Assign {
                    name: "n".to_string(),
                    value: call("collections.findIndex", vec![ident("ys"), ident("isOpen")]),
                    line: 1,
                },
            ],
            line: 1,
        },
    ]));
    assert_eq!(result.owner("f"), ResOwner::Local);
}

#[test]
fn an_element_read_put_into_a_container_floats_its_source() {
    let get = call(
        "collections.get",
        vec![append("xs", "f"), HirExpression::Number("0".to_string())],
    );
    assert_eq!(
        owner_after_inner_assign(HirExpression::ListLiteral(vec![get])),
        ResOwner::Float("xs".to_string())
    );
}

#[test]
fn resource_in_returned_collection_floats_to_it() {
    // MUT xs = []; RES f; xs = append(xs, f); RETURN xs — f floats to xs even
    // at the same scope depth, because xs is declared first and is returned;
    // the `RETURN` transfers xs's owned-list to the caller (§15.6).
    let result = analyze_function(&func(vec![
        list("xs", HirExpression::ListLiteral(vec![])),
        res("f", open()),
        HirStatement::Assign {
            name: "xs".to_string(),
            value: append("xs", "f"),
            line: 1,
        },
        HirStatement::Return {
            value: Some(ident("xs")),
            line: 1,
        },
    ]));
    assert_eq!(result.owner("f"), ResOwner::Float("xs".to_string()));
    assert!(result.floats("f"));
}

#[test]
fn bare_resource_return_does_not_float() {
    // RES f; RETURN f — a direct resource return is an ordinary move, not a
    // collection escape.
    let result = analyze_function(&func(vec![
        res("f", open()),
        HirStatement::Return {
            value: Some(ident("f")),
            line: 1,
        },
    ]));
    assert_eq!(result.owner("f"), ResOwner::Local);
}

/// bug-643: a `RES` binding of a BORROWED element holds no close obligation, so
/// appending it to an outer collection must not float it there — the
/// collection's owned-list would close a handle another scope owns. It stays a
/// `RES` name (resolved `Local`), so every ownership rule still sees it.
#[test]
fn a_borrowed_element_never_floats() {
    let get = HirExpression::Call {
        callee: "collections.get".to_string(),
        arguments: vec![
            HirCallArg::Positional(ident("files")),
            HirCallArg::Positional(HirExpression::Number("0".to_string())),
        ],
        line: 1,
        column: 1,
    };
    let result = analyze_function(&func(vec![
        list("out", HirExpression::ListLiteral(vec![])),
        HirStatement::While {
            kind: crate::ast::LoopKind::While,
            condition: HirExpression::Boolean(true),
            body: vec![
                res("one", get),
                HirStatement::Assign {
                    name: "out".to_string(),
                    value: append("out", "one"),
                    line: 1,
                },
                // The control: a produced handle in the same position floats.
                res("fresh", open()),
                HirStatement::Assign {
                    name: "out".to_string(),
                    value: append("out", "fresh"),
                    line: 1,
                },
            ],
            line: 1,
        },
    ]));
    assert_eq!(result.owner("one"), ResOwner::Local);
    assert_eq!(result.owner("fresh"), ResOwner::Float("out".to_string()));
}

#[test]
fn float_follows_collection_copy_chain() {
    // Outer ys; inner { RES f; xs = [f]; ys = xs } — f reaches ys (outermost).
    let result = analyze_function(&func(vec![
        list("ys", HirExpression::ListLiteral(vec![])),
        HirStatement::While {
            kind: crate::ast::LoopKind::While,
            condition: HirExpression::Boolean(true),
            body: vec![
                res("f", open()),
                list("xs", HirExpression::ListLiteral(vec![ident("f")])),
                HirStatement::Assign {
                    name: "ys".to_string(),
                    value: ident("xs"),
                    line: 1,
                },
            ],
            line: 1,
        },
    ]));
    // ys is the outermost referencing collection.
    assert_eq!(result.owner("f"), ResOwner::Float("ys".to_string()));
}

// -----------------------------------------------------------------------
// bug-843 — a closure is a container: capture = `collections::append`.
// -----------------------------------------------------------------------

/// `LAMBDA(s) -> <body>`
fn lambda(body: HirExpression) -> HirExpression {
    HirExpression::Lambda {
        params: vec![crate::hir::HirParam {
            name: "s".to_string(),
            type_: ParameterType::String,
            resource: false,
            state_type: None,
            stray_state: None,
            default: None,
            line: 1,
        }],
        body: Box::new(body),
        assign_target: None,
    }
}

/// `use(<name>, s)` — a body that reads a captured name.
fn reads(name: &str) -> HirExpression {
    HirExpression::Call {
        callee: "use".to_string(),
        arguments: vec![
            HirCallArg::Positional(ident(name)),
            HirCallArg::Positional(ident("s")),
        ],
        line: 1,
        column: 1,
    }
}

fn closure_binding(name: &str, value: HirExpression, explicit: bool) -> HirStatement {
    HirStatement::Let {
        mutable: true,
        resource: false,
        state_type: None,
        name: name.to_string(),
        type_: ParameterType::Func(
            vec![ParameterType::String],
            Box::new(ParameterType::Integer),
            false,
        ),
        explicit_type: explicit,
        value: Some(value),
        line: 1,
    }
}

/// The `List` twin is `inner_resource_floats_to_outer_collection`: a resource
/// captured by a closure stored in an outer binding floats to that binding.
#[test]
fn a_resource_captured_into_an_outer_closure_floats_to_it() {
    let result = analyze_function(&func(vec![
        closure_binding("g", lambda(HirExpression::Number("0".into())), true),
        HirStatement::While {
            kind: crate::ast::LoopKind::While,
            condition: HirExpression::Boolean(true),
            body: vec![
                res("f", open()),
                HirStatement::Assign {
                    name: "g".to_string(),
                    value: lambda(reads("f")),
                    line: 1,
                },
            ],
            line: 1,
        },
    ]));
    assert_eq!(result.owner("f"), ResOwner::Float("g".to_string()));
}

/// A lambda's own parameter shadows an outer name: it is not a capture.
#[test]
fn a_lambda_parameter_is_not_a_capture() {
    let result = analyze_function(&func(vec![
        closure_binding("g", lambda(HirExpression::Number("0".into())), true),
        HirStatement::While {
            kind: crate::ast::LoopKind::While,
            condition: HirExpression::Boolean(true),
            body: vec![
                res("s", open()),
                HirStatement::Assign {
                    name: "g".to_string(),
                    value: lambda(reads("unrelated")),
                    line: 1,
                },
            ],
            line: 1,
        },
    ]));
    assert_eq!(result.owner("s"), ResOwner::Local);
}

/// The bug's repro, whose `List` twin is `LET h = H[f]; RETURN [h]`: a
/// returned closure carrying a record declared after its resource is
/// refused exactly as the record would be (`TYPE_RESOURCE_RETURN_ORDER`).
#[test]
fn a_returned_closure_over_a_later_record_is_blocked() {
    let mut records = HashSet::new();
    records.insert(ParameterType::declared("Holder"));
    let result = analyze_function_with(
        &func(vec![
            res("f", open()),
            holder("h", false, construct("f")),
            HirStatement::Return {
                value: Some(lambda(reads("h"))),
                line: 1,
            },
        ]),
        &records,
        &HashSet::new(),
    );
    assert_eq!(result.owner("f"), ResOwner::FloatBlocked("h".to_string()));
}

/// The twin of `LET xs AS List OF RES File = [f]; RETURN xs`: an explicitly
/// typed closure binding is a container that can own the resource.
#[test]
fn a_returned_closure_binding_declared_after_its_resource_is_blocked() {
    let result = analyze_function(&func(vec![
        res("f", open()),
        closure_binding("g", lambda(reads("f")), true),
        HirStatement::Return {
            value: Some(ident("g")),
            line: 1,
        },
    ]));
    assert_eq!(result.owner("f"), ResOwner::FloatBlocked("g".to_string()));
}

// -----------------------------------------------------------------------
// bug-806 — a module-level binding is the outermost scope.
// -----------------------------------------------------------------------

fn assign(name: &str, value: HirExpression) -> HirStatement {
    HirStatement::Assign {
        name: name.to_string(),
        value,
        line: 1,
    }
}

fn globals(names: &[&str]) -> HashSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

/// `saved = append(saved, f)` in a function body: `saved` is no local of the
/// function, so it is placed at `MODULE_DEPTH` and `f` floats to it — the
/// storing function's own scope must not close it.
#[test]
fn a_resource_stored_into_a_global_floats_to_the_global() {
    let result = analyze_function_with(
        &func(vec![
            res("f", open()),
            assign("saved", append("saved", "f")),
        ]),
        &HashSet::new(),
        &globals(&["saved"]),
    );
    assert_eq!(result.owner("f"), ResOwner::Float("saved".to_string()));
}

/// A global outranks an outer local collection that also holds the resource:
/// the outermost scope wins.
#[test]
fn a_global_outranks_an_outer_local_collection() {
    let result = analyze_function_with(
        &func(vec![
            list("xs", HirExpression::ListLiteral(Vec::new())),
            HirStatement::If {
                condition: HirExpression::Boolean(true),
                then_body: vec![
                    res("f", open()),
                    assign("xs", append("xs", "f")),
                    assign("saved", ident("xs")),
                ],
                else_body: Vec::new(),
                line: 1,
            },
        ]),
        &HashSet::new(),
        &globals(&["saved"]),
    );
    assert_eq!(result.owner("f"), ResOwner::Float("saved".to_string()));
}

/// A resource parameter's owner is in a caller's frame: storing it into a global
/// is raised to the module at run time, at the store, not by this analysis.
#[test]
fn a_parameter_stored_into_a_global_does_not_float_statically() {
    let mut function = func(vec![assign("saved", append("saved", "p"))]);
    function.params.push(crate::hir::HirParam {
        name: "p".to_string(),
        type_: ParameterType::parse("fs.File"),
        resource: true,
        state_type: None,
        stray_state: None,
        default: None,
        line: 1,
    });
    let result = analyze_function_with(&function, &HashSet::new(), &globals(&["saved"]));
    assert_eq!(result.owner("p"), ResOwner::Local);
}

/// Without the module's global set the old placement stands — the same
/// statement in a function the lowering says assigns no global.
#[test]
fn an_assignment_to_an_unknown_name_is_not_a_global() {
    let result = analyze_function_with(
        &func(vec![
            res("f", open()),
            assign("saved", append("saved", "f")),
        ]),
        &HashSet::new(),
        &HashSet::new(),
    );
    assert_eq!(result.owner("f"), ResOwner::Local);
}
