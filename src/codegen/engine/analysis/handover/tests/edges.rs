//! Edges of the hand-over predicates the program probes in `mod.rs` do not reach:
//! a self-referential record type, a parameter a closure takes by reference or a
//! loop rebinds, and the shapes `returned_field_store_local` declines.

use super::*;
use crate::codegen::memory::arena::graph_copy::tests::{field, model, record};
use crate::target::shared::nir::NirRecordUpdate;

/// H2: a record that reaches itself is refused rather than reasoned about — the
/// cycle is only reachable through a field the record does not solely own.
#[test]
fn a_self_referential_record_is_not_a_handover_type() {
    let cyclic = model(vec![
        record("A", vec![field("xs", "List OF Integer"), field("b", "B")]),
        record("B", vec![field("a", "A")]),
    ]);
    assert!(!handover_type(&cyclic, &ParameterType::declared("A")));
    let plain = model(vec![record("A", vec![field("xs", "List OF Integer")])]);
    assert!(
        handover_type(&plain, &ParameterType::declared("A")),
        "a record of collections is one"
    );
}

/// P2: a name a closure captures BY REFERENCE (a `LocalRef`), or a loop variable,
/// is "captured or address-taken"; a name read only by value inside a closure is
/// also captured, and a name nothing touches is not.
#[test]
fn a_by_reference_capture_or_a_loop_variable_counts_as_taken() {
    let module = lower(
        "IMPORT collections\nIMPORT io\n\nFUNC main() AS Integer\n  MUT acc AS String = \"\"\n  \
         collections::forEach([\"a\", \"b\"], LAMBDA(x AS String) -> acc = acc & x)\n  \
         FOR EACH n IN [1, 2]\n    io::print(toString(n))\n  NEXT\n  io::print(acc)\n  \
         RETURN 0\nEND FUNC\n",
    );
    let main = function(&module, "main");
    assert!(
        is_captured_or_address_taken(main, "acc"),
        "a by-ref capture takes acc"
    );
    assert!(
        is_captured_or_address_taken(main, "n"),
        "a FOR EACH rebinds n"
    );
    assert!(!is_captured_or_address_taken(main, "untouched"));
}

fn with(target: NirValue, updates: Vec<(&str, NirValue)>) -> NirValue {
    NirValue::WithUpdate {
        type_: ParameterType::declared("Outer"),
        target: Box::new(target),
        updates: updates
            .into_iter()
            .map(|(field, value)| NirRecordUpdate {
                field: field.to_string(),
                value,
            })
            .collect(),
    }
}

fn local(name: &str) -> NirValue {
    NirValue::Local(name.to_string())
}

/// plan-147-F: `RETURN WITH r { … }` stores into `r`'s own block only for a `WITH`
/// over a local, with at least one update, every one a slot, a fixed-size inlined
/// record, or the LAST variable-width inlined field.
#[test]
fn a_returned_field_store_declines_what_it_cannot_store_in_place() {
    let model = model(vec![
        record("Fixed", vec![field("a", "Integer"), field("b", "Integer")]),
        record("Named", vec![field("s", "String")]),
        record(
            "Outer",
            vec![
                field("n", "Integer"),
                field("fixed", "Fixed"),
                field("named", "Named"),
                field("tail", "String"),
            ],
        ),
    ]);
    let value = || NirValue::Const {
        type_: ParameterType::Integer,
        value: "1".to_string(),
    };
    // Not over a local.
    assert_eq!(
        returned_field_store_local(&model, &with(value(), vec![("n", value())])),
        None
    );
    // No update at all.
    assert_eq!(
        returned_field_store_local(&model, &with(local("r"), Vec::new())),
        None
    );
    // A variable-width record field that is not the last inlined one.
    assert_eq!(
        returned_field_store_local(&model, &with(local("r"), vec![("named", value())])),
        None
    );
    // A slot, a fixed-size inlined record, and the last variable-width field.
    assert_eq!(
        returned_field_store_local(
            &model,
            &with(
                local("r"),
                vec![("n", value()), ("fixed", value()), ("tail", value())]
            )
        ),
        Some("r")
    );
}
