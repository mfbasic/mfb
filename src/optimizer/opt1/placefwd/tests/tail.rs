//! Place copy forwarding at the window contents and owner shapes the cases in
//! `mod.rs` do not build: values whose types the call gate must look through
//! (a Set or Result of scalars, a recursive record, an undeclared nominal), a
//! runtime-helper call, a slot reference to the binding, `MATCH` patterns and
//! guards that read it, and a place rooted in a capture or in the binding
//! itself.

use super::*;

fn capture(type_: ParameterType) -> NirValue {
    NirValue::Capture {
        index: 0,
        type_,
        by_ref: false,
    }
}

/// The global set shape with `extra` evaluated between the copy and its read.
fn with_window_op(extra: NirValue) -> Vec<NirOp> {
    let mut body = global_set_shape(local("v"));
    body.insert(1, eval(extra));
    body
}

const FORWARDED_TAIL: [&str; 2] = [
    "LET v = collections.get(global g.xs, 0)",
    "global g = WITH global g { xs := collections.set(global g.xs, 0, v) }",
];

/// A value of a type that cannot hold a function value — a Set or a Result
/// of scalars — and a runtime-helper call run no user code, so the window
/// stays open.
#[test]
fn values_that_cannot_reach_user_code_keep_the_window_open() {
    for extra in [
        capture(ParameterType::set_of(ParameterType::Integer)),
        capture(ParameterType::result_of(ParameterType::Integer)),
        NirValue::RuntimeCall {
            helper: crate::target::shared::runtime::RuntimeHelper::Abi,
            target: "abi.f".to_string(),
            args: vec![],
            loc: NirSourceLoc::default(),
        },
    ] {
        let after = body_after_with_helper(with_window_op(extra));
        assert!(after[0].starts_with("eval "), "{after:?}");
        assert_eq!(after[1..], FORWARDED_TAIL);
    }
}

/// A recursive record holding no function value is answered without looping;
/// an undeclared nominal might hold one, so it closes the window.
#[test]
fn recursive_and_undeclared_nominals_are_answered() {
    let node = record(
        "Node",
        vec![
            ("value", ParameterType::Integer),
            ("kids", ParameterType::list_of(ParameterType::named("Node"))),
        ],
    );
    let mut module = module_of(vec![
        function(with_window_op(capture(ParameterType::named("Node")))),
        helper(),
    ]);
    module.types.push(node);
    run(&mut module, 1);
    let after = render_ops(&module.functions[0].body);
    assert_eq!(
        after[1..],
        FORWARDED_TAIL,
        "a recursive scalar record is safe"
    );

    assert_kept_with_helper(with_window_op(capture(ParameterType::named("Mystery"))));
}

/// A slot reference to the binding is a use the forward cannot repoint.
#[test]
fn a_slot_reference_to_the_binding_keeps_the_copy() {
    assert_kept_with_helper(with_window_op(NirValue::LocalRef {
        name: "cur".to_string(),
        type_: list_int(),
    }));
}

/// A `MATCH` reading the binding in a pattern and a guard is rewritten there
/// too.
#[test]
fn match_patterns_and_guards_reading_the_binding_are_forwarded() {
    let mut body = global_set_shape(local("v"));
    body.insert(
        1,
        NirOp::Match {
            value: local("cur"),
            cases: vec![
                crate::target::shared::nir::NirMatchCase {
                    pattern: NirMatchPattern::Value(local("cur")),
                    guard: Some(local("cur")),
                    body: vec![],
                },
                crate::target::shared::nir::NirMatchCase {
                    pattern: NirMatchPattern::OneOf(vec![local("cur")]),
                    guard: None,
                    body: vec![],
                },
            ],
        },
    );
    let mut module = module_of(vec![function(body), helper()]);
    run(&mut module, 1);
    let body = &module.functions[0].body;
    let NirOp::Match { value, cases } = &body[0] else {
        panic!("the MATCH leads the forwarded body: {:?}", render_ops(body));
    };
    assert_eq!(render(value), "global g.xs");
    let NirMatchPattern::Value(first) = &cases[0].pattern else {
        panic!("value pattern");
    };
    assert_eq!(render(first), "global g.xs");
    assert_eq!(
        render(cases[0].guard.as_ref().expect("guard")),
        "global g.xs"
    );
    let NirMatchPattern::OneOf(values) = &cases[1].pattern else {
        panic!("one-of pattern");
    };
    assert_eq!(render(&values[0]), "global g.xs");
}

/// A place rooted in a capture, or in the binding's own name, has no owner the
/// row can prove a window for.
#[test]
fn a_place_without_a_provable_owner_keeps_the_copy() {
    let mut from_capture = global_set_shape(local("v"));
    from_capture[0] = bind("cur", list_int(), member(capture(rec_type()), "xs"));
    assert_kept_with_helper(from_capture);

    let mut from_itself = global_set_shape(local("v"));
    from_itself[0] = bind("cur", list_int(), member(local("cur"), "xs"));
    assert_kept_with_helper(from_itself);
}
