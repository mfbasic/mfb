//! The borrow-get window predicates (`borrow_get/predicates.rs`) over the value
//! and statement shapes no lowering suite puts in front of them: a negated index,
//! a loop or `TRAP` variable or a `STATE` store re-binding a watched name, and the
//! single-expression update's rewrite through every kind of value an update can
//! nest a field read in.

use super::*;
use crate::operators::{BinaryOp, UnaryOp};
use crate::target::shared::nir::{NirRecordUpdate, NirSourceLoc};
use crate::target::shared::runtime::RuntimeHelper;

/// The index `-j` is as pure as `j`: re-evaluated at the write-back it names the
/// slot it named at the `get`, so `get(xs, -j)` … `set(xs, -j, WITH p { … })` is
/// still one element update. A write-back to `-(j + 1)` is another slot,
/// so it is not.
#[test]
fn a_negated_index_names_the_same_slot_and_a_different_negation_does_not() {
    let program = |write_index: &str| {
        format!(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             LET j AS Integer = 0 - i\n    LET p AS P = collections::get(xs, -j)\n    \
             xs = collections::set(xs, {write_index}, WITH p {{ age := p.age + 1 }})\n  NEXT\n  \
             RETURN len(xs)\nEND FUNC\n"
        )
    };
    let same = classify(&program("-j"));
    assert!(same.elements.contains_key("p"), "-j / -j is one slot");
    let other = classify(&program("-(j + 1)"));
    assert!(!other.elements.contains_key("p"), "-j / -(j + 1) is not");
}

fn integer(value: &str) -> NirValue {
    NirValue::Const {
        type_: ParameterType::Integer,
        value: value.to_string(),
    }
}

fn local(name: &str) -> NirValue {
    NirValue::Local(name.to_string())
}

/// Each op that (re)binds a name counts as a write of it: a `Bind`, an `Assign`,
/// a `FOR` / `FOR EACH` / `TRAP` variable, and a `STATE` store through it. A name
/// the op does not bind is not written.
#[test]
fn every_binding_op_writes_the_name_it_binds() {
    let integer_type = ParameterType::Integer;
    let ops: Vec<(&str, NirOp)> = vec![
        (
            "a FOR variable",
            NirOp::For {
                name: "w".to_string(),
                type_: integer_type.clone(),
                start: integer("0"),
                end: integer("1"),
                step: integer("1"),
                body: Vec::new(),
                loc: NirSourceLoc::default(),
            },
        ),
        (
            "a FOR EACH variable",
            NirOp::ForEach {
                name: "w".to_string(),
                type_: integer_type.clone(),
                iterable: local("xs"),
                body: Vec::new(),
            },
        ),
        (
            "a TRAP variable",
            NirOp::Trap {
                name: "w".to_string(),
                body: Vec::new(),
            },
        ),
        (
            "a STATE store",
            NirOp::StateAssign {
                resource: "w".to_string(),
                value: integer("1"),
            },
        ),
        (
            "an assignment",
            NirOp::Assign {
                name: "w".to_string(),
                value: integer("1"),
            },
        ),
    ];
    let watched: HashSet<String> = HashSet::from(["w".to_string()]);
    let other: HashSet<String> = HashSet::from(["v".to_string()]);
    for (what, op) in ops {
        let ops = std::slice::from_ref(&op);
        assert!(writes_any(ops, &watched), "{what} writes w");
        assert!(!writes_any(ops, &other), "{what} writes nothing named v");
    }
}

fn element_get() -> NirValue {
    NirValue::Call {
        target: "collections.get".to_string(),
        args: vec![local("xs"), local("i")],
        loc: NirSourceLoc::default(),
    }
}

fn field_of_element() -> NirValue {
    NirValue::MemberAccess {
        target: Box::new(element_get()),
        member: "age".to_string(),
    }
}

fn boxed(value: NirValue) -> Box<NirValue> {
    Box::new(value)
}

/// One wrapper per kind of value that can hold another, each holding `inner`.
fn wrappers(inner: fn() -> NirValue) -> Vec<(&'static str, NirValue)> {
    let p = ParameterType::declared("P");
    vec![
        (
            "a call",
            NirValue::Call {
                target: "f".to_string(),
                args: vec![inner()],
                loc: NirSourceLoc::default(),
            },
        ),
        (
            "a trapped call",
            NirValue::CallResult {
                target: "f".to_string(),
                args: vec![inner()],
                loc: NirSourceLoc::default(),
            },
        ),
        (
            "a runtime call",
            NirValue::RuntimeCall {
                helper: RuntimeHelper::General,
                target: "toString".to_string(),
                args: vec![inner()],
                loc: NirSourceLoc::default(),
            },
        ),
        (
            "a closure capture",
            NirValue::Closure {
                name: "lambda".to_string(),
                type_: ParameterType::Integer,
                captures: vec![inner()],
            },
        ),
        (
            "a constructor",
            NirValue::Constructor {
                type_: p.clone(),
                args: vec![inner()],
            },
        ),
        (
            "a list literal",
            NirValue::ListLiteral {
                type_: ParameterType::Integer,
                values: vec![inner()],
            },
        ),
        (
            "a set literal",
            NirValue::SetLiteral {
                type_: ParameterType::Integer,
                values: vec![inner()],
            },
        ),
        (
            "a map literal value",
            NirValue::MapLiteral {
                type_: ParameterType::Integer,
                entries: vec![(integer("1"), inner())],
            },
        ),
        (
            "a union wrap",
            NirValue::UnionWrap {
                union_type: p.clone(),
                member_type: ParameterType::Integer,
                value: boxed(inner()),
            },
        ),
        (
            "a union payload",
            NirValue::UnionExtract {
                type_: ParameterType::Integer,
                value: boxed(inner()),
            },
        ),
        (
            "a Result test",
            NirValue::ResultIsOk {
                value: boxed(inner()),
            },
        ),
        (
            "a Result value",
            NirValue::ResultValue {
                value: boxed(inner()),
            },
        ),
        (
            "a Result error",
            NirValue::ResultError {
                value: boxed(inner()),
            },
        ),
        (
            "a checked operator",
            NirValue::Checked {
                type_: ParameterType::Integer,
                value: boxed(inner()),
                observe: false,
            },
        ),
        (
            "a nested WITH",
            NirValue::WithUpdate {
                type_: p.clone(),
                target: boxed(local("other")),
                updates: vec![NirRecordUpdate {
                    field: "age".to_string(),
                    value: inner(),
                }],
            },
        ),
        (
            "a binary operator",
            NirValue::Binary {
                op: BinaryOp::Add,
                left: boxed(integer("1")),
                right: boxed(inner()),
                loc: NirSourceLoc::default(),
            },
        ),
        (
            "a unary operator",
            NirValue::Unary {
                op: UnaryOp::Negate,
                operand: boxed(inner()),
                loc: NirSourceLoc::default(),
            },
        ),
    ]
}

fn update_with(value: NirValue) -> NirValue {
    NirValue::WithUpdate {
        type_: ParameterType::declared("P"),
        target: boxed(element_get()),
        updates: vec![NirRecordUpdate {
            field: "age".to_string(),
            value,
        }],
    }
}

/// `(element field reads, element gets left)` in `value`.
fn census(value: &NirValue) -> (usize, usize) {
    struct Count {
        reads: usize,
        gets: usize,
    }
    impl NirVisitor for Count {
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::MemberAccess { target, .. } if matches!(target.as_ref(), NirValue::Local(name) if name == "e") =>
                {
                    self.reads += 1;
                }
                NirValue::Call { target, .. } if target == "collections.get" => self.gets += 1,
                _ => {}
            }
            walk_value(self, value);
        }
    }
    let mut count = Count { reads: 0, gets: 0 };
    count.visit_value(value);
    (count.reads, count.gets)
}

/// The single-expression update `xs = set(xs, i, WITH get(xs, i) { age := … })`
/// becomes an update of the element-bound binding `e`: every field read of
/// `get(xs, i)` anywhere in an update value, however deeply nested, is rewritten
/// to a read of `e`. A missed nesting would leave a second, unbound `get` of the
/// slot being rewritten.
#[test]
fn a_field_read_of_the_element_is_rebound_through_every_value_kind() {
    let container = ContainerRef::Local("xs".to_string());
    for (what, value) in wrappers(field_of_element) {
        let rewritten =
            bind_single_expression_element(&update_with(value), &container, &local("i"), "e")
                .unwrap_or_else(|| panic!("{what}: a field read of the element was refused"));
        assert_eq!(census(&rewritten), (1, 0), "{what}");
    }
}

/// The element itself handed anywhere but a field read could keep, free or hand
/// over the alias, so the rewrite refuses it wherever it is nested.
#[test]
fn the_element_passed_whole_is_refused_through_every_value_kind() {
    let container = ContainerRef::Local("xs".to_string());
    for (what, value) in wrappers(element_get) {
        assert!(
            bind_single_expression_element(&update_with(value), &container, &local("i"), "e")
                .is_none(),
            "{what}: the element passed whole was rebound"
        );
    }
}

/// The rewrite takes only a `WITH`, and refuses one whose update values read the
/// container other than by a lookup (`f(xs)` could move or mutate the block the
/// element points into).
#[test]
fn the_rewrite_refuses_a_non_with_and_a_non_lookup_read_of_the_container() {
    let container = ContainerRef::Local("xs".to_string());
    assert!(bind_single_expression_element(&element_get(), &container, &local("i"), "e").is_none());
    let reads_container = NirValue::Call {
        target: "f".to_string(),
        args: vec![local("xs")],
        loc: NirSourceLoc::default(),
    };
    assert!(bind_single_expression_element(
        &update_with(reads_container),
        &container,
        &local("i"),
        "e"
    )
    .is_none());
}

/// A `FOR EACH` over the container is a lookup; one over another list is not a
/// read of the container at all, but its body still is checked.
#[test]
fn a_for_each_reads_the_container_only_through_its_iterable_or_body() {
    let container = ContainerRef::Local("xs".to_string());
    let for_each = |iterable: &str, body: NirValue| NirOp::ForEach {
        name: "v".to_string(),
        type_: ParameterType::Integer,
        iterable: local(iterable),
        body: vec![NirOp::Eval { value: body }],
    };
    let len_of_xs = NirValue::Call {
        target: "len".to_string(),
        args: vec![local("xs")],
        loc: NirSourceLoc::default(),
    };
    let untouched =
        |op: NirOp| container_untouched(std::slice::from_ref(&op), &container, &|_, _| false);
    assert!(untouched(for_each("xs", local("v"))));
    assert!(untouched(for_each("ys", len_of_xs)));
    assert!(!untouched(for_each("ys", local("xs"))));
}
