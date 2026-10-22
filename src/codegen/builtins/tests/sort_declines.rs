//! The copying `sort`/`sortBy` fast path (`collection/assign/builder_inplace_sort.rs`,
//! bug-767 A) serves a call only when it can type it: the value a list it can copy,
//! the key function's return type ordered. Anything else answers `Ok(None)` having
//! emitted NOTHING, so the call keeps the `.mfb` body — a fast path that emitted half
//! a sort before declining would leave dead code, or worse, a half-sorted copy. And
//! the word-merge gate the fast path and the scratch prescan share accepts only a
//! two-argument `sortBy` monomorph.

use crate::codegen::collection::assign::builder_inplace_sort::sort_by_word_path_accepts;
use crate::codegen::engine::builder::{CodeBuilder, GlobalValue};
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{harness, model};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

fn integers() -> NirValue {
    NirValue::Global {
        name: "xs".to_string(),
        type_: ParameterType::list_of(ParameterType::Integer),
    }
}

/// A function value returning `returns`.
fn key_fn(returns: ParameterType) -> NirValue {
    NirValue::FunctionRef {
        name: "key".to_string(),
        type_: ParameterType::func(vec![ParameterType::Integer], returns),
    }
}

/// Run `body` on a builder that HAS the self-update scratch (so a decline is the
/// value's, not the frame's) and a global `xs AS List OF Integer`.
fn with_builder<R>(body: impl FnOnce(&mut CodeBuilder<'_>) -> R) -> R {
    let model = model(Vec::new());
    let mut harness = harness(&model);
    harness.globals.insert(
        "xs".to_string(),
        GlobalValue {
            type_: ParameterType::list_of(ParameterType::Integer),
            offset: 64,
        },
    );
    let platform = TestPlatform;
    let mut builder = harness.builder("sort", &platform);
    builder.self_update_scratch = Some(builder.allocate_stack_object("su_scratch", 8));
    body(&mut builder)
}

#[test]
fn the_copying_sort_declines_what_it_cannot_type_and_emits_nothing() {
    with_builder(|builder| {
        let before = builder.instructions.len();
        let not_a_list = NirValue::Const {
            type_: ParameterType::Integer,
            value: "1".to_string(),
        };
        assert!(
            builder
                .lower_copying_sort(&[not_a_list.clone()])
                .expect("a decline is not an error")
                .is_none(),
            "sort of a value that is not a list"
        );
        assert!(
            builder
                .lower_copying_sort_by(&[integers(), not_a_list])
                .expect("a decline is not an error")
                .is_none(),
            "sortBy whose key is not a function"
        );
        assert!(
            builder
                .lower_copying_sort_by(&[integers(), key_fn(ParameterType::Boolean)])
                .expect("a decline is not an error")
                .is_none(),
            "sortBy whose keys have no `<`"
        );
        assert_eq!(
            builder.instructions.len(),
            before,
            "a decline emits nothing"
        );
    });
}

#[test]
fn the_word_merge_takes_only_a_two_argument_sort_by_monomorph() {
    assert!(!sort_by_word_path_accepts(
        "#collections_sort$Integer",
        &[integers()]
    ));
    assert!(!sort_by_word_path_accepts(
        "#collections_sortBy$Integer$Integer",
        &[integers()]
    ));
}
