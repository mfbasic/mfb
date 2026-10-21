//! `flatten_concat_spine` (`engine/value/builder_values/mod.rs`): the left spine of a
//! `&` chain, in source order.

use crate::codegen::engine::value::builder_values::flatten_concat_spine;
use crate::operators::BinaryOp;
use crate::target::shared::nir::{NirSourceLoc, NirValue};
use crate::types::ParameterType;

fn text(value: &str) -> NirValue {
    NirValue::Const {
        type_: ParameterType::String,
        value: value.to_string(),
    }
}

fn concat(left: NirValue, right: NirValue) -> NirValue {
    NirValue::Binary {
        op: BinaryOp::Concat,
        left: Box::new(left),
        right: Box::new(right),
        loc: NirSourceLoc::default(),
    }
}

fn spine(value: &NirValue) -> Vec<String> {
    let mut parts = Vec::new();
    flatten_concat_spine(value, &mut parts);
    parts
        .into_iter()
        .map(|part| match part {
            NirValue::Const { value, .. } => value.clone(),
            _ => "?".to_string(),
        })
        .collect()
}

/// `a & b & c` parses left-leaning, so the spine is `a`, `b` and the
/// caller appends `c`.
#[test]
fn a_left_leaning_chain_flattens_in_source_order() {
    let chain = concat(concat(text("a"), text("b")), text("c"));
    let NirValue::Binary { left, .. } = &chain else {
        panic!("expected a concat")
    };
    assert_eq!(spine(left), vec!["a", "b"]);
}

/// A four-operand chain keeps going.
#[test]
fn a_longer_chain_keeps_its_order() {
    let chain = concat(concat(concat(text("a"), text("b")), text("c")), text("d"));
    let NirValue::Binary { left, .. } = &chain else {
        panic!("expected a concat")
    };
    assert_eq!(spine(left), vec!["a", "b", "c"]);
}

/// The spine stops at a non-`&` value: a parenthesized right subchain
/// keeps its own grouping and is fused as its own chain.
#[test]
fn the_spine_stops_at_a_non_concat() {
    let inner = concat(text("b"), text("c"));
    assert_eq!(spine(&inner), vec!["b", "c"]);
    let other = NirValue::Binary {
        op: BinaryOp::Add,
        left: Box::new(text("a")),
        right: Box::new(text("b")),
        loc: NirSourceLoc::default(),
    };
    assert_eq!(spine(&other), vec!["?"]);
}

/// A lone operand is a one-element spine, which the caller's arity check
/// then sends down the pairwise path.
#[test]
fn a_lone_operand_is_its_own_spine() {
    assert_eq!(spine(&text("a")), vec!["a"]);
}
