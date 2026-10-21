//! No `abi_inline` collection lowering aborts on an element type it was not written for.
//!
//! `abi_inline.rs` poisons an argument's WHOLE type. A collection lowering checks one
//! level further down — `math::abs` on a `List OF Integer | Fixed | Float`, `toSet` on
//! a fixed-width element, `zip` and `chunks` on a width it can address — and each of
//! those inner checks ends in a refusal naming the element type. A source program
//! never reaches one (the type checker has already accepted the call), so each refusal
//! is the only thing between a corrupted or future type and a lowering that indexes
//! memory by the wrong width.
//!
//! The sweep poisons the ELEMENT (and key, and value) of every collection-typed
//! parameter of every `abi_inline` implementation in the registry, with a type nothing
//! declares and with the scalars a lowering is most likely to mistake for its own, and
//! asserts the lowering ANSWERS — a result or a refusal — instead of aborting. A body
//! that accepts the poisoned type is allowed (a length, a copy, a drop does not care
//! what the element is); one that aborts has an unchecked assumption about it.

use crate::codegen::engine::builder::ValueResult;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::codegen::engine::util::vreg_frame::Vregs;
use crate::codegen::registry::{registry, Body, Implementation};
use crate::types::ParameterType;

/// `ty` with every element, key and value type of its outermost collection replaced by
/// `leaf`, or `None` when it is not a collection.
fn poisoned(ty: &ParameterType, leaf: &ParameterType) -> Option<ParameterType> {
    match ty {
        ParameterType::ListOf(_) => Some(ParameterType::list_of(leaf.clone())),
        ParameterType::SetOf(_) => Some(ParameterType::set_of(leaf.clone())),
        ParameterType::MapOf(_, _) => Some(ParameterType::map_of(leaf.clone(), leaf.clone())),
        ParameterType::Res(inner) => poisoned(inner, leaf),
        _ => None,
    }
}

fn arguments(implementation: &Implementation, at: usize, leaf: &ParameterType) -> Vec<ValueResult> {
    let mut vregs = Vregs::new();
    implementation
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| ValueResult {
            type_: if index == at {
                poisoned(&param.ty, leaf).unwrap_or_else(|| param.ty.clone())
            } else {
                param.ty.clone()
            },
            location: Operand::from(vregs.next()),
            text: param.name.to_string(),
            origin: None,
        })
        .collect()
}

#[test]
fn no_collection_lowering_aborts_on_an_unexpected_element_type() {
    let platform = TestPlatform;
    let leaves = [
        ParameterType::named("NoSuchType"),
        ParameterType::String,
        ParameterType::Boolean,
        ParameterType::Money,
    ];
    let mut aborted = Vec::new();
    let mut probed = 0;
    for package in registry().packages() {
        for function in package.functions() {
            for (index, implementation) in function.implementations().iter().enumerate() {
                let Body::AbiInline(lower) = implementation.body else {
                    continue;
                };
                for at in 0..implementation.params.len() {
                    if poisoned(&implementation.params[at].ty, &leaves[0]).is_none() {
                        continue;
                    }
                    for leaf in &leaves {
                        probed += 1;
                        let wrong = arguments(implementation, at, leaf);
                        let outcome = crate::testutil::silence_panics(|| {
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                let harness = BuilderHarness::default();
                                let mut builder =
                                    harness.builder("_mfb_abi_inline_probe", &platform);
                                let ctx = harness.abi_ctx(&platform);
                                let _ = lower(&mut builder, &wrong, &ctx);
                            }))
                        });
                        if outcome.is_err() {
                            aborted.push(format!(
                                "{}::{}#{index} argument {at} as a collection of {leaf}",
                                package.import_name(),
                                function.name
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        probed >= 300,
        "the sweep reached only {probed} collection parameters; the registry walk has broken"
    );
    assert!(
        aborted.is_empty(),
        "{} abi_inline lowering(s) abort on a collection element type they were not \
         written for, instead of refusing it:\n  {}",
        aborted.len(),
        aborted.join("\n  ")
    );
}
