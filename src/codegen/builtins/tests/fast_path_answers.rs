//! A `Body::Mfb` fast path answers a call it was not written for — it never aborts.
//!
//! `registry_bodies.rs` proves a fast path declines ANOTHER member's target. The other
//! way a fast path is handed something it cannot serve is its OWN target with the
//! wrong shape: the wrong argument count, an element type outside the widths it can
//! address (`toSet` serves the fixed-width elements, `zip` and `chunks` the widths
//! their data pointer arithmetic supports), or an argument whose type disagrees with
//! the instantiation named in the target. A program never produces any of these — the
//! monomorph target is built from the very arguments — so the guards that answer them
//! (`Ok(None)` to fall back to the member's own body, or an error naming the
//! mismatch) are unreachable, and the alternative to them is a lowering that reads a
//! list at the wrong stride.
//!
//! For every fast path in the registry, built from its member's own monomorph name
//! (`#package_member$Type…`), this offers each of those shapes and asserts the answer
//! is a decline, a refusal or a lowering — never an abort — and that a decline leaves
//! the caller's instruction stream untouched.

use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::codegen::registry::{registry, Body};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

fn constant(type_: ParameterType) -> NirValue {
    NirValue::Const {
        type_,
        value: "0".to_string(),
    }
}

#[test]
fn a_fast_path_answers_its_own_target_with_the_wrong_shape() {
    let platform = TestPlatform;
    let element_types = [
        "Nothing", "String", "Integer", "Byte", "Boolean", "Scalar", "Float",
    ];
    let mut probed = 0;
    let mut aborted = Vec::new();
    let mut declined_with_output = Vec::new();
    for package in registry().packages() {
        for function in package.functions() {
            for (index, implementation) in function.implementations().iter().enumerate() {
                let Body::Mfb {
                    fast_path: Some(fast),
                    ..
                } = implementation.body
                else {
                    continue;
                };
                let arity = implementation.params.len();
                for element in element_types {
                    // Both type arguments, since a two-argument instantiation
                    // (`zip`) names two.
                    let target = format!(
                        "#{}_{}${element}${element}",
                        package.import_name(),
                        function.name
                    );
                    for count in [0, 1, arity, arity + 1] {
                        for argument_type in [ParameterType::Integer, ParameterType::String] {
                            probed += 1;
                            let args: Vec<NirValue> = (0..count)
                                .map(|_| constant(argument_type.clone()))
                                .collect();
                            let label = format!(
                                "{}::{}#{index} `{target}` with {count} {argument_type} argument(s)",
                                package.import_name(),
                                function.name
                            );
                            let outcome = crate::testutil::silence_panics(|| {
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    let harness = BuilderHarness::default();
                                    let mut builder = harness.builder("_mfb_probe", &platform);
                                    let answer = fast(&mut builder, &target, &args);
                                    (matches!(answer, Ok(None)), builder.instructions.len())
                                }))
                            });
                            match outcome {
                                Err(_) => aborted.push(label),
                                Ok((true, emitted)) if emitted > 1 => {
                                    declined_with_output.push(label)
                                }
                                Ok(_) => {}
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        probed >= 500,
        "the sweep probed only {probed} calls; the registry walk has broken"
    );
    assert!(
        aborted.is_empty(),
        "{} fast-path call(s) abort on their own target with the wrong shape; a \
         decline or a refusal is the answer. First: {:?}",
        aborted.len(),
        &aborted[..aborted.len().min(40)]
    );
    assert!(
        declined_with_output.is_empty(),
        "a declining fast path must leave the caller's stream untouched: {declined_with_output:?}"
    );
}
