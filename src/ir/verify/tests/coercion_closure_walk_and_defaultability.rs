//! plan-176-G: `ir::verify::tests` — coercion and include cycles, unknown bounds and package
//! overloads, the closure capture walk, collection element borrows, and defaultability. The
//! shared builders and fixtures live in `verify/tests/mod.rs`.

use super::*;

// --- compatible / expression_compatible coercion paths ---------------------

#[test]
fn accepts_byte_literal_into_byte_param() {
    let callee = func_returns("helper", "Nothing", vec![param("b", "Byte", None)], vec![]);
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "helper".to_string(),
            args: vec![int_const("5")],
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let caller = func_returns("run", "Nothing", vec![], body);
    accept(&project(vec![callee, caller], vec![]));
}

#[test]
fn accepts_integer_literal_into_fixed_param() {
    let callee = func_returns("helper", "Nothing", vec![param("x", "Fixed", None)], vec![]);
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "helper".to_string(),
            args: vec![int_const("5")],
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let caller = func_returns("run", "Nothing", vec![], body);
    accept(&project(vec![callee, caller], vec![]));
}

#[test]
fn accepts_negated_literal_into_fixed_binding() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "x",
            "Fixed",
            Some(unary(UnaryOp::Negate, int_const("1"), "Integer")),
            true,
            false,
        )],
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn accepts_union_variant_return() {
    // Returning a variant record value where the union type is expected.
    let body = vec![ret(IrValue::Constructor {
        type_: crate::types::ParameterType::parse("Circle"),
        args: vec![],
    })];
    let f = func_returns("run", "Shape", vec![], body);
    let mut u = union("Shape", &["Circle", "Square"]);
    u.variants[0].fields = vec![];
    accept(&project(vec![f], vec![u]));
}

#[test]
fn accepts_list_compatible_recursion() {
    // Return a List OF Integer where List OF Integer is expected via a param.
    let body = vec![ret(IrValue::Local("xs".to_string()))];
    let f = func_returns(
        "run",
        "List OF Integer",
        vec![param("xs", "List OF Integer", None)],
        body,
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn accepts_map_compatible_recursion() {
    let body = vec![ret(IrValue::Local("m".to_string()))];
    let f = func_returns(
        "run",
        "Map OF String TO Integer",
        vec![param("m", "Map OF String TO Integer", None)],
        body,
    );
    accept(&project(vec![f], vec![]));
}

// --- unknown value poisoning cascade ---------------------------------------

#[test]
fn poisoned_initializer_yields_unknown_value() {
    // A binary op mismatch poisons the value; the bind then reports UNKNOWN_VALUE.
    let body = vec![bind(
        "x",
        "Integer",
        Some(binary(
            BinaryOp::Subtract,
            const_of("String", "a"),
            int_const("1"),
            "Integer",
        )),
        false,
        false,
    )];
    let f = func_returns("run", "Nothing", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(got.contains(&"TYPE_UNKNOWN_VALUE".to_string()), "{got:?}");
}

#[test]
fn poisoned_return_yields_unknown_value() {
    let body = vec![ret(binary(
        BinaryOp::Subtract,
        const_of("String", "a"),
        int_const("1"),
        "Integer",
    ))];
    let f = func_returns("run", "Integer", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(got.contains(&"TYPE_UNKNOWN_VALUE".to_string()), "{got:?}");
}

// --- map-key ownership violation -------------------------------------------

#[test]
fn rejects_map_key_thread_ownership() {
    // A Map keyed on a Thread handle: ordinary collections cannot own it.
    //
    // The key needs its OWN ` TO Out`, so the well-formed spelling carries two
    // top-level ` TO `s: the first belongs to the nested `Thread OF`, the second
    // separates the map's key from its value (`split_top_level_to`). Written
    // `Map OF Thread OF Integer TO Integer` (one ` TO `) before plan-106-B, which
    // `ParameterType::parse` correctly reads as a map with NO value type and so
    // yields an opaque `Named` — the check only ever fired because the duplicate
    // `parse_map` split naively on the FIRST ` TO `, mis-reading the key as
    // `Thread OF Integer` and the value as `Integer`.
    let f = func_returns(
        "run",
        "Nothing",
        vec![param(
            "m",
            "Map OF Thread OF Integer TO Integer TO Integer",
            None,
        )],
        vec![],
    );
    expect_rule(
        &project(vec![f], vec![]),
        "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
    );
}

#[test]
fn rejects_map_key_record_with_resource() {
    // A record field carrying a resource makes a record key an ownership
    // violation (contains_resource_or_thread over record_field_lists).
    // Craft the record with a File field (records-cannot-own is separately
    // reported, but the map-key ownership check still fires).
    let mut holder = record_typed("Holder", &[("f", "fs.File")]);
    holder.file = "src/main.mfb".to_string();
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("m", "Map OF Holder TO Integer", None)],
        vec![],
    );
    expect_rule(
        &project(vec![f], vec![holder]),
        "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
    );
}

// --- compatible recursion (list / result / map) ----------------------------

#[test]
fn rejects_nested_list_mismatch_return() {
    // RETURN a List OF List OF Integer where List OF List OF String expected.
    let body = vec![ret(IrValue::Local("xs".to_string()))];
    let f = func_returns(
        "run",
        "List OF List OF String",
        vec![param("xs", "List OF List OF Integer", None)],
        body,
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_RETURN_MISMATCH");
}

#[test]
fn rejects_value_return_from_nothing_function() {
    // bug-929: `Nothing` is a success type like any other, so `RETURN <Integer>`
    // in an `AS Nothing` function is a mismatch, not a dropped value.
    let body = vec![ret(IrValue::Local("n".to_string()))];
    let f = func_returns("run", "Nothing", vec![param("n", "Integer", None)], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_RETURN_MISMATCH");
}

#[test]
fn accepts_nothing_value_return_from_nothing_function() {
    let body = vec![ret(IrValue::Local("n".to_string()))];
    let f = func_returns("run", "Nothing", vec![param("n", "Nothing", None)], body);
    accept(&project(vec![f], vec![]));
}

#[test]
fn accepts_nested_map_return() {
    let body = vec![ret(IrValue::Local("m".to_string()))];
    let f = func_returns(
        "run",
        "Map OF String TO List OF Integer",
        vec![param("m", "Map OF String TO List OF Integer", None)],
        body,
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn accepts_result_of_return() {
    // Result OF Integer compatible recursion via compatible().
    let body = vec![ret(IrValue::Local("r".to_string()))];
    let f = func_returns(
        "run",
        "#Result OF Integer",
        vec![param("r", "#Result OF Integer", None)],
        body,
    );
    accept(&project(vec![f], vec![]));
}

// --- union include cycle / expansion ---------------------------------------

#[test]
fn union_include_cycle_is_bounded() {
    // Two unions that include each other — the expansion is cycle-guarded.
    let mut a = union("A", &["X"]);
    a.includes = vec!["B".to_string()];
    let mut b = union("B", &["Y"]);
    b.includes = vec!["A".to_string()];
    let f = func_returns("run", "Nothing", vec![], vec![]);
    // No panic / infinite loop; may or may not emit but must terminate.
    let _ = rules(&project(vec![f], vec![a, b]));
}

#[test]
fn accepts_union_with_included_union() {
    let mut outer = union("Outer", &["Local1"]);
    outer.includes = vec!["Inner".to_string()];
    let inner = union("Inner", &["A", "B"]);
    let f = func_returns("run", "Nothing", vec![], vec![]);
    accept(&project(vec![f], vec![outer, inner]));
}

// --- record field include cycle --------------------------------------------

#[test]
fn record_include_cycle_terminates() {
    // Two records including each other via `includes` — collect_record_fields
    // cycle guard.
    let mut a = record("A", &["fa"]);
    a.includes = vec!["B".to_string()];
    let mut b = record("B", &["fb"]);
    b.includes = vec!["A".to_string()];
    // Access a field present via include chain.
    let body = vec![ret(IrValue::MemberAccess {
        target: Box::new(IrValue::Local("x".to_string())),
        member: "fb".to_string(),
        type_: crate::types::ParameterType::parse("Unknown"),
    })];
    let f = func_returns("run", "Integer", vec![param("x", "A", None)], body);
    accept(&project(vec![f], vec![a, b]));
}

// --- for / foreach unknown-typed bound skip --------------------------------

#[test]
fn for_unknown_bound_is_skipped() {
    // A local typed "Unknown" as the FOR end bound is skipped, not rejected.
    let body = vec![
        bind("u", "Unknown", Some(int_const("0")), false, false),
        IrOp::For {
            name: "i".to_string(),
            type_: ParameterType::Integer,
            start: int_const("0"),
            end: IrValue::Local("u".to_string()),
            step: int_const("1"),
            body: vec![],
            loc: IrSourceLoc::default(),
        },
    ];
    let f = func_returns("run", "Nothing", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_FOR_REQUIRES_NUMERIC"),
        "{got:?}"
    );
}

#[test]
fn foreach_unknown_iterable_is_skipped() {
    let body = vec![
        bind("u", "Unknown", Some(int_const("0")), false, false),
        IrOp::ForEach {
            name: "e".to_string(),
            type_: ParameterType::Integer,
            iterable: IrValue::Local("u".to_string()),
            body: vec![],
            loc: IrSourceLoc::default(),
        },
    ];
    let f = func_returns("run", "Nothing", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_FOR_EACH_REQUIRES_COLLECTION"),
        "{got:?}"
    );
}

#[test]
fn rejects_bits_bad_args() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        // Two arguments (the member's arity), both of the wrong type: the arity
        // check precedes the type check, as in the source checker (plan-107-E).
        vec![eval_call(
            "bits.band",
            vec![const_of("String", "x"), const_of("String", "y")],
        )],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_CALL_ARGUMENT_MISMATCH");
}

#[test]
fn rejects_encoding_bad_args() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![eval_call("encoding.hexDecode", vec![int_const("1")])],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(got.iter().any(|r| r.starts_with("TYPE_CALL")), "{got:?}");
}

#[test]
fn rejects_io_bad_args() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![eval_call(
            "io.print",
            vec![int_const("1"), int_const("2"), int_const("3")],
        )],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(got.iter().any(|r| r.starts_with("TYPE_CALL")), "{got:?}");
}

#[test]
fn rejects_fs_bad_args() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![eval_call("fs.appendText", vec![int_const("1")])],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(got.iter().any(|r| r.starts_with("TYPE_CALL")), "{got:?}");
}

#[test]
fn rejects_net_bad_args() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        // plan-110-C moved the datagram surface to `udp`, so `net.bindUdp` no
        // longer exists; `net.lookup` is the equivalent one-required-argument net
        // member, and one Integer where a String belongs is the same mistake.
        vec![eval_call("net.lookup", vec![int_const("1")])],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(got.iter().any(|r| r.starts_with("TYPE_CALL")), "{got:?}");
}

#[test]
fn rejects_vector_bad_args() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![eval_call("vector.abs", vec![const_of("String", "x")])],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(got.iter().any(|r| r.starts_with("TYPE_CALL")), "{got:?}");
}

#[test]
fn unknown_package_call_is_skipped() {
    // A dotted call that resolves to no known builtin package is left alone.
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![eval_call("nonpkg.doThing", vec![int_const("1")])],
    );
    accept(&project(vec![f], vec![]));
}

// --- rich closure body: walk_captures over every value shape ---------------

#[test]
fn closure_body_captures_walked_over_all_shapes() {
    // A closure body (1 slot) whose ops embed a Capture inside each value shape,
    // all in range (index 0). Exercises walk_captures + collect_closures arms.
    let cap = || IrValue::Capture {
        index: 0,
        type_: crate::types::ParameterType::parse("Integer"),
        by_ref: false,
    };
    let body = vec![
        // Constructor(Capture)
        bind(
            "a",
            "Point",
            Some(IrValue::Constructor {
                type_: crate::types::ParameterType::parse("Point"),
                args: vec![cap(), int_const("1")],
            }),
            false,
            false,
        ),
        // Call(Capture)
        eval_call("io.print", vec![cap()]),
        // ListLiteral(Capture)
        bind(
            "l",
            "List OF Integer",
            Some(IrValue::ListLiteral {
                type_: crate::types::ParameterType::parse("List OF Integer"),
                values: vec![cap()],
            }),
            false,
            false,
        ),
        // MapLiteral(Capture)
        bind(
            "m",
            "Map OF Integer TO Integer",
            Some(IrValue::MapLiteral {
                type_: crate::types::ParameterType::parse("Map OF Integer TO Integer"),
                entries: vec![(cap(), cap())],
            }),
            false,
            false,
        ),
        // Binary(Capture, Capture)
        bind(
            "bn",
            "Integer",
            Some(IrValue::Binary {
                op: BinaryOp::Add,
                left: Box::new(cap()),
                right: Box::new(cap()),
                type_: crate::types::ParameterType::parse("Integer"),
                loc: IrSourceLoc::default(),
            }),
            false,
            false,
        ),
        // Unary(Capture)
        bind(
            "un",
            "Integer",
            Some(IrValue::Unary {
                op: UnaryOp::Negate,
                operand: Box::new(cap()),
                type_: crate::types::ParameterType::parse("Integer"),
                loc: IrSourceLoc::default(),
            }),
            false,
            false,
        ),
        // WithUpdate(target=Capture)
        bind(
            "wu",
            "Point",
            Some(IrValue::WithUpdate {
                type_: crate::types::ParameterType::parse("Point"),
                target: Box::new(IrValue::Local("a".to_string())),
                updates: vec![crate::ir::IrRecordUpdate {
                    field: "x".to_string(),
                    value: cap(),
                }],
            }),
            false,
            false,
        ),
        // MemberAccess(target=Capture)
        bind(
            "ma",
            "Integer",
            Some(IrValue::MemberAccess {
                target: Box::new(IrValue::Local("a".to_string())),
                member: "x".to_string(),
                type_: crate::types::ParameterType::parse("Integer"),
            }),
            false,
            false,
        ),
        // Closure(Capture) as a nested closure argument
        eval_call(
            "io.print",
            vec![IrValue::Closure {
                name: "inner".to_string(),
                type_: crate::types::ParameterType::parse("FUNC() AS Integer"),
                captures: vec![cap()],
            }],
        ),
        ret(cap()),
    ];
    let closure_body = func_returns("body", "Integer", vec![], body);
    let inner = func_returns(
        "inner",
        "Integer",
        vec![],
        vec![ret(IrValue::Capture {
            index: 0,
            type_: crate::types::ParameterType::parse("Integer"),
            by_ref: false,
        })],
    );
    let maker = func_returns(
        "make",
        "FUNC() AS Integer",
        vec![],
        vec![ret(IrValue::Closure {
            name: "body".to_string(),
            type_: crate::types::ParameterType::parse("FUNC() AS Integer"),
            captures: vec![int_const("1")],
        })],
    );
    // The body reads slot 0 (in range) throughout; no capture violation.
    let diags = rules(&project(
        vec![closure_body, inner, maker],
        vec![record("Point", &["x", "y"])],
    ));
    assert!(
        !diags.iter().any(|r| r.contains("capture index")),
        "unexpected capture violation: {diags:?}"
    );
}

// --- collect_local_reads / collect_closures over union/result shapes -------

#[test]
fn accepts_function_with_union_and_result_value_shapes() {
    // Exercises collect_local_reads_value and collect_closures over
    // UnionWrap/UnionExtract/ResultIsOk/ResultValue/ResultError shapes.
    let body = vec![
        bind(
            "w",
            "Shape",
            Some(IrValue::UnionWrap {
                union_type: crate::types::ParameterType::parse("Shape"),
                member_type: crate::types::ParameterType::parse("Circle"),
                value: Box::new(IrValue::Local("c".to_string())),
            }),
            false,
            false,
        ),
        bind(
            "e",
            "Circle",
            Some(IrValue::UnionExtract {
                type_: crate::types::ParameterType::parse("Circle"),
                value: Box::new(IrValue::Local("w".to_string())),
            }),
            false,
            false,
        ),
        bind(
            "ok",
            "Boolean",
            Some(IrValue::ResultIsOk {
                value: Box::new(IrValue::Local("r".to_string())),
            }),
            false,
            false,
        ),
        bind(
            "v",
            "Integer",
            Some(IrValue::ResultValue {
                type_: crate::types::ParameterType::parse("Integer"),
                value: Box::new(IrValue::Local("r".to_string())),
            }),
            false,
            false,
        ),
        bind(
            "er",
            "Error",
            Some(IrValue::ResultError {
                value: Box::new(IrValue::Local("r".to_string())),
            }),
            false,
            false,
        ),
        ret_none(),
    ];
    let mut u = union("Shape", &["Circle", "Square"]);
    u.variants[0].fields = vec![];
    let f = func_returns(
        "run",
        "Nothing",
        vec![
            param("c", "Circle", None),
            param("r", "#Result OF Integer", None),
        ],
        body,
    );
    // Circle is a variant record; register it via the union.
    accept(&project(vec![f], vec![u, record("Circle", &[])]));
}

// --- assignment via LocalRef / FunctionRef read shapes ---------------------

#[test]
fn accepts_localref_and_functionref_values() {
    let body = vec![
        bind(
            "r",
            "Integer",
            Some(IrValue::LocalRef {
                name: "x".to_string(),
                type_: crate::types::ParameterType::parse("Integer"),
            }),
            false,
            false,
        ),
        bind(
            "fr",
            "FUNC() AS Integer",
            Some(IrValue::FunctionRef {
                name: "helper".to_string(),
                type_: crate::types::ParameterType::parse("FUNC() AS Integer"),
            }),
            false,
            false,
        ),
        ret_none(),
    ];
    let helper = func_returns("helper", "Integer", vec![], vec![ret(int_const("1"))]);
    let f = func_returns("run", "Nothing", vec![param("x", "Integer", None)], body);
    accept(&project(vec![helper, f], vec![]));
}

// --- non-owning element (RES bind of collections.get) ----------------------

fn get_call(list: &str, ret_type: &str) -> IrValue {
    IrValue::Call {
        target: "collections.get".to_string(),
        args: vec![IrValue::Local(list.to_string()), int_const("0")],
        type_: crate::types::ParameterType::parse(ret_type),
        loc: IrSourceLoc::default(),
    }
}

#[test]
fn accepts_res_bind_of_a_collection_element() {
    // plan-59-E: was `rejects_res_bind_of_non_owning_element`
    // (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`, retired). A `RES` is a pointer to the
    // one resource, and an element is such a pointer like any other holder.
    // RES h = collections.get(xs, 0) where the element type is a resource.
    let body = vec![bind(
        "h",
        "fs.File",
        Some(get_call("xs", "fs.File")),
        true,
        false,
    )];
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![param("xs", "List OF RES fs.File", None)],
        body,
    );
    f.resource_owners
        .insert("h".to_string(), crate::ir::resource_escape::ResOwner::Local);
    accept(&project(vec![f], vec![]));
}

#[test]
fn accepts_return_of_a_resource_collection_element() {
    // plan-59-E: was `rejects_return_non_owning_resource_element`
    // (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`, retired). Returning the element hands
    // the pointer to the caller, whose scope becomes the outermost one touching
    // the resource.
    // RETURN collections.get(xs, 0) whose element is a resource.
    let body = vec![ret(get_call("xs", "fs.File"))];
    let f = func_returns(
        "run",
        "fs.File",
        vec![param("xs", "List OF RES fs.File", None)],
        body,
    );
    accept(&project(vec![f], vec![]));
}

// --- is_defaultable branches (MUT without value) ---------------------------

#[test]
fn mut_list_is_defaultable() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("xs", "List OF Integer", None, true, true)],
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn mut_map_is_defaultable() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("m", "Map OF String TO Integer", None, true, true)],
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn mut_set_is_defaultable() {
    // `MUT s AS Set OF Integer` with no initializer defaults to the empty set.
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("s", "Set OF Integer", None, true, true)],
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn rejects_mut_set_of_resource_ownership_and_comparable() {
    // `MUT s AS Set OF fs.File`: after bug-434 a `Set OF T` is ALWAYS defaultable
    // (empty set), so the former `TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE` no longer
    // fires. The binding is still rejected — on the two INDEPENDENT axes bug-434
    // deliberately left untouched: an ordinary collection cannot own a resource
    // (`TYPE_COLLECTION_OWNERSHIP_VIOLATION`) and a Set element must be
    // comparable (`TYPE_REQUIRES_COMPARABLE`, File is not). Verified against the
    // release binary: `MUT s AS Set OF fs.File = []` is likewise rejected on these
    // same axes, so the doc's premise that this becomes accepted was wrong.
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("s", "Set OF fs.File", None, true, true)],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter()
            .any(|r| r == "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE"),
        "Set OF T is defaultable after bug-434; the defaultability rule must not fire: {got:?}"
    );
    assert!(
        got.iter()
            .any(|r| r == "TYPE_COLLECTION_OWNERSHIP_VIOLATION"),
        "a resource in an ordinary collection is still rejected on the ownership axis: {got:?}"
    );
}

// plan-172-N: an `ENUM` defaults to its first variant, so it is defaultable, and
// so is a record whose only obstacle was an enum field.
#[test]
fn mut_enum_is_defaultable() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("m", "Mode", None, true, true)],
    );
    accept(&project(
        vec![f],
        vec![enum_type("Mode", &["Fast", "Off", "Slow"])],
    ));
}

#[test]
fn mut_record_with_an_enum_field_is_defaultable() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("o", "Outer", None, true, true)],
    );
    accept(&project(
        vec![f],
        vec![
            enum_type("Mode", &["Fast", "Off"]),
            record_typed("Inner", &[("mode", "Mode"), ("weight", "Float")]),
            record_typed(
                "Outer",
                &[("name", "String"), ("mode", "Mode"), ("inner", "Inner")],
            ),
        ],
    ));
}

// The control for the two above: the enum arm must not have widened the rule for
// its neighbours. A union still has no default, bare or as a record's field —
// including in a record that ALSO holds an enum, where a predicate that returned
// early on the first defaultable field would wrongly accept.
#[test]
fn rejects_mut_record_with_a_union_field_beside_an_enum_field() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("h", "Holder", None, true, true)],
    );
    expect_rule(
        &project(
            vec![f],
            vec![
                enum_type("Mode", &["Fast", "Off"]),
                record("Circle", &["r"]),
                record("Square", &["s"]),
                union("Shape", &["Circle", "Square"]),
                record_typed("Holder", &[("mode", "Mode"), ("shape", "Shape")]),
            ],
        ),
        "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
    );
}

#[test]
fn rejects_mut_func_not_defaultable() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("fn", "FUNC() AS Integer", None, true, true)],
    );
    expect_rule(
        &project(vec![f], vec![]),
        "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
    );
}

// Was `rejects_mut_enum_not_defaultable` (b20ab6b56, plan-12 wave 2: one case per
// `is_defaultable` branch as the predicate then stood). plan-172-N reverses that
// branch — an enum defaults to its first variant — so the same program is now
// accepted. A one-variant enum, where "first" is the only choice.
#[test]
fn mut_single_variant_enum_is_defaultable() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("c", "Color", None, true, true)],
    );
    accept(&project(vec![f], vec![enum_type("Color", &["Red"])]));
}

#[test]
fn mut_record_of_defaultable_fields_ok() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("p", "Point", None, true, true)],
    );
    accept(&project(vec![f], vec![record("Point", &["x", "y"])]));
}

#[test]
fn rejects_mut_record_with_nondefaultable_field() {
    // A record whose field type is a FUNC — not defaultable.
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("h", "Holder", None, true, true)],
    );
    expect_rule(
        &project(
            vec![f],
            vec![record_typed("Holder", &[("cb", "FUNC() AS Integer")])],
        ),
        "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
    );
}

#[test]
fn accepts_mut_list_of_union_defaultable_empty() {
    // bug-434: `List OF <union>` is always defaultable (empty list). The
    // element type's defaultability is irrelevant — an empty list materializes
    // no element.
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("xs", "List OF Choice", None, true, true)],
    );
    accept(&project(vec![f], vec![union("Choice", &["Label"])]));
}

#[test]
fn accepts_mut_map_value_union_defaultable_empty() {
    // bug-434: `Map OF K TO <union>` is always defaultable (empty map).
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("m", "Map OF String TO Choice", None, true, true)],
    );
    accept(&project(vec![f], vec![union("Choice", &["Label"])]));
}

#[test]
fn accepts_mut_record_with_list_of_union_field() {
    // bug-434: the non-defaultability of a `List OF <union>` field must NOT
    // cascade into the containing record. A record embedding such a field is
    // defaultable (the field defaults to the empty list).
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("d", "Doc", None, true, true)],
    );
    accept(&project(
        vec![f],
        vec![
            record_typed("Doc", &[("attrs", "List OF Choice")]),
            union("Choice", &["Label"]),
        ],
    ));
}

#[test]
fn rejects_state_collection_not_a_record() {
    // bug-1013 (owner ruling, 2026-10-10): a STATE type is a record TYPE, so a
    // collection is refused as one — `fs.File STATE List OF <union>` included.
    //
    // This was `accepts_state_list_of_nondefaultable` (bug-434), which pinned
    // the opposite as an "intended STATE ripple" of collections becoming always
    // defaultable. bug-434's own subject is untouched and stays covered on the
    // MUT axis by `accepts_mut_list_of_union_defaultable_empty`,
    // `accepts_mut_map_value_union_defaultable_empty` and
    // `accepts_mut_record_with_list_of_union_field` above: a collection IS still
    // defaultable. What changed is that being defaultable is no longer enough
    // to be a STATE type.
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("h", "fs.File STATE List OF Choice", None, true, false)],
    );
    f.resource_owners
        .insert("h".to_string(), crate::ir::resource_escape::ResOwner::Local);
    expect_rule(
        &project(vec![f], vec![union("Choice", &["Label"])]),
        "TYPE_STATE_INVALID",
    );
}

#[test]
fn rejects_mut_unknown_record_not_defaultable() {
    // An unknown type name (not in record_field_lists) is not defaultable.
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("w", "Widget", None, true, true)],
    );
    expect_rule(
        &project(vec![f], vec![]),
        "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
    );
}
