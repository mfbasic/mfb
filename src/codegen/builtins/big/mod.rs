//! The built-in `big` package (plan-127): arbitrary-precision signed integers.
//!
//! `big::Int` is a package-owned, exported, copyable value record — the `net::Url`
//! shape, not a `RES` handle — holding a little-endian magnitude and a sign flag.
//! Every member lowers natively through `Body::abi_function`; the shared emitters
//! every member reads and builds a `big::Int` through live in `gen_big.rs`, so the
//! canonical form (no trailing zero bytes, never negative zero) is established in
//! exactly one place.

use crate::codegen::registry::{
    EnumVariant, RecordProp, Registry, RegistryEnum, RegistryPackage, RegistryRecord,
};
use crate::types::ParameterType;

/// The `Int` record's bare member id — the `RegistryRecord` name.
pub(crate) const INT_TYPE: &str = "Int";

/// The `Endian` enum's bare member id.
pub(crate) const ENDIAN_TYPE: &str = "Endian";

const MODULE_INTRO: &str = r#"Signed integers of any size, with arithmetic that never overflows"#;
const MODULE_DESC: &str = r#"The `big` package provides `big::Int`, a signed integer with no fixed size. Where an
`Integer` is 64 bits wide and its arithmetic raises on overflow, a `big::Int` grows to
hold whatever value it is given. `big` is a built-in package: `IMPORT big` needs no
manifest dependency.

A `big::Int` is an ordinary value. Assigning it or passing it to a function makes an
independent copy, and there is nothing to open or close. A `big::Int` declared with
`MUT` and no initializer holds zero.

**Operators do not apply.** `+`, `-`, `*`, `/`, `=`, `<>`, `<` and `>` are rejected at
compile time on a `big::Int`. For the same reason a `big::Int` cannot be a `Map` key
or a `Set` element.

**Not for secrets.** Nothing in `big` runs in constant time: how long a call takes, and
which bytes it reads, depend on the values involved. Use `crypto::` for anything
cryptographic."#;

/// Register the `big` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("big", MODULE_INTRO, MODULE_DESC);

    // Field ORDER is contract, not documentation: every `big` lowering reads and
    // builds an `Int` at the slots these declarations fix — slot 0 (`magnitude`) holds
    // the block-relative offset of the inlined `List OF Byte`, slot 1 (`negative`) the
    // flag inline (`gen_big.rs`, plan-127-A §4.2). Reordering these silently turns
    // every member into a misread.
    pkg.add_record(RegistryRecord {
        name: INT_TYPE,
        export: true,
        description: "A signed integer of any size. The value is `magnitude` read as an unsigned little-endian number, negated when `negative` is `TRUE`. Every `big` member returns it in canonical form: `magnitude` has no trailing zero bytes, is empty exactly when the value is zero, and `negative` is `FALSE` for zero. A value built by hand that breaks those rules still reads as the number it spells, so trailing zero bytes and a negative zero are accepted everywhere.",
        props: vec![
            RecordProp {
                name: "magnitude",
                ty: ParameterType::list_of(ParameterType::Byte),
                description: "The absolute value, least significant byte first. Empty for zero.",
            },
            RecordProp {
                name: "negative",
                ty: ParameterType::Boolean,
                description: "`TRUE` when the value is below zero. `FALSE` for zero.",
            },
        ],
    });

    // Variant ORDER fixes the discriminants (`Little` = 0, `Big` = 1), which the
    // `fromBytes`/`toBytes` lowerings compare against directly.
    pkg.add_enum(RegistryEnum {
        name: ENDIAN_TYPE,
        export: true,
        variants: vec![
            EnumVariant {
                name: "Little",
                description: "Least significant byte first — the order `big::Int`'s `magnitude` uses.",
                advisory: None,
            },
            EnumVariant {
                name: "Big",
                description: "Most significant byte first — the order most wire formats and key encodings use.",
                advisory: None,
            },
        ],
    });

    r.add_package(pkg);
}

// Man/spec citation anchor: `BIG`. The `big/*` man pages and the stdlib spec chapter
// ground their package-level and value-type facts here with `[[…/big/mod.rs:BIG]]`.

#[cfg(test)]
mod tests {
    use crate::codegen::registry::registry;
    use crate::types::ParameterType;

    #[test]
    fn big_registered_on_the_clean_room_registry() {
        let pkg = registry().resolve_package("big").expect("big package");
        let source = pkg.get_mfb();
        assert!(source.contains("EXPORT TYPE Int"), "{source}");
        assert!(source.contains("EXPORT ENUM Endian"), "{source}");
        assert!(source.contains("Little"), "{source}");
        assert!(source.contains("Big"), "{source}");
    }

    #[test]
    fn int_and_endian_are_builtin_types() {
        assert!(registry().is_builtin_type("Int"));
        assert!(registry().is_builtin_type("Endian"));
        assert_eq!(
            registry().qualified_builtin_type("big.Int"),
            Some("big.Int".to_string())
        );
        assert_eq!(
            registry().qualified_builtin_type("big.Endian"),
            Some("big.Endian".to_string())
        );
    }

    /// Field order is the layout every lowering addresses (plan-127-A §4.2): slot 0 is
    /// the magnitude, slot 1 the sign. A reorder must fail here, not as a misread.
    #[test]
    fn int_field_order_is_magnitude_then_negative() {
        let layout = crate::codegen::registry::builtin_record_layouts()
            .iter()
            .find(|(type_, _)| type_.name() == "big.Int")
            .map(|(_, fields)| fields.clone())
            .expect("big.Int has a builtin record layout");
        assert_eq!(
            layout,
            vec![
                (
                    "magnitude".to_string(),
                    ParameterType::list_of(ParameterType::Byte)
                ),
                ("negative".to_string(), ParameterType::Boolean),
            ]
        );
    }

    /// The magnitude is inlined into the record's own block, so a whole-record copy
    /// is one byte copy and the slot holds a block-relative offset — the reading every
    /// `gen_big.rs` emitter is written against.
    #[test]
    fn int_magnitude_field_is_inlined() {
        let model = crate::codegen::engine::builder::TypeModel::builtin_records();
        assert!(crate::codegen::collection::layout::record_field_is_inlined(
            model,
            &ParameterType::list_of(ParameterType::Byte)
        ));
        assert!(!crate::codegen::collection::layout::record_field_is_inlined(
            model,
            &ParameterType::Boolean
        ));
    }
}
