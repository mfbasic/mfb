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

mod func_from_bytes;
mod func_from_integer;
mod func_to_bytes;
mod func_to_integer;

pub(crate) mod gen_big;

/// The `Int` record's bare member id — the `RegistryRecord` name.
pub(crate) const INT_TYPE: &str = "Int";

/// The `Int` record's package-qualified type identity, the `*_TYPE` / `*_TYPE_ID`
/// split `net/mod.rs` established. A lowering that type-checks or builds an `Int`
/// names THIS; the registry row declares the bare leaf.
pub(crate) const INT_TYPE_ID: &str = "big.Int";

/// The `Endian` enum's bare member id and its package-qualified identity.
pub(crate) const ENDIAN_TYPE: &str = "Endian";
pub(crate) const ENDIAN_TYPE_ID: &str = "big.Endian";

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

**Conversions.** `big::fromInteger` and `big::toInteger` cross to and from `Integer`;
`toInteger` raises `ErrOverflow` when the value does not fit. `big::fromBytes` and
`big::toBytes` cross to and from a `List OF Byte` magnitude in either byte order,
selected by `big::Endian`.

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

    // Conversion seams (plan-127-A Phase 4).
    func_from_integer::register(&mut pkg);
    func_to_integer::register(&mut pkg);
    func_from_bytes::register(&mut pkg);
    func_to_bytes::register(&mut pkg);

    r.add_package(pkg);
}

// Man/spec citation anchor: `BIG`. The `big/*` man pages and the stdlib spec chapter
// ground their package-level and value-type facts here with `[[…/big/mod.rs:BIG]]`.

#[cfg(test)]
mod tests {
    use super::{ENDIAN_TYPE_ID, INT_TYPE_ID};
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
            registry().qualified_builtin_type(INT_TYPE_ID),
            Some(INT_TYPE_ID.to_string())
        );
        assert_eq!(
            registry().qualified_builtin_type(ENDIAN_TYPE_ID),
            Some(ENDIAN_TYPE_ID.to_string())
        );
    }

    /// Field order is the layout every lowering addresses (plan-127-A §4.2): slot 0 is
    /// the magnitude, slot 1 the sign. A reorder must fail here, not as a misread.
    #[test]
    fn int_field_order_is_magnitude_then_negative() {
        let layout = crate::codegen::registry::builtin_record_layouts()
            .iter()
            .find(|(type_, _)| type_.name() == INT_TYPE_ID)
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

    /// Every member and the exact error set of every implementation. An `abi_function`
    /// member's declared errors are its registry `errors` vector — the only list
    /// `mfb man` renders and the only one a test can pin (plan-127-A Corrections C1).
    const MEMBERS: &[(&str, &[&str])] = &[
        ("big.fromInteger", &[]),
        ("big.toInteger", &["ErrOverflow"]),
        ("big.fromBytes", &[]),
        ("big.toBytes", &[]),
    ];

    #[test]
    fn every_member_declares_exactly_its_errors_and_lowers_natively() {
        let pkg = registry().resolve_package("big").expect("big package");
        assert_eq!(pkg.functions().len(), MEMBERS.len());
        for (name, errors) in MEMBERS {
            let resolved = registry()
                .resolve_func(name)
                .unwrap_or_else(|| panic!("{name} is not registered"));
            for implementation in &resolved.function.implementations {
                assert_eq!(implementation.errors, errors.to_vec(), "{name}");
            }
            // `None` pins that the member stayed an `abi_function`: an inline-lowered
            // body would start feeding the inline-`TRAP` census instead.
            assert_eq!(
                crate::codegen::registry::native_member_declares_error(name),
                None,
                "{name}"
            );
            assert!(
                crate::codegen::registry::abi_function_lower(name).is_some(),
                "{name} has no abi_function lowering"
            );
        }
    }

    #[test]
    fn member_signatures() {
        use crate::codegen::registry::{argument_types, call_return_type_typed};
        let int = || "big.Int".to_string();
        let args = |name: &str| argument_types(name).unwrap_or_else(|| panic!("{name}"));
        let ret = |name: &str| {
            call_return_type_typed(name)
                .map(|t| t.name().into_owned())
                .unwrap_or_else(|| panic!("{name}"))
        };
        assert_eq!(args("big.fromInteger"), vec!["Integer"]);
        assert_eq!(ret("big.fromInteger"), int());
        assert_eq!(args("big.toInteger"), vec![int()]);
        assert_eq!(ret("big.toInteger"), "Integer");
        assert_eq!(
            args("big.fromBytes"),
            vec!["List OF Byte", "Boolean", "big.Endian"]
        );
        assert_eq!(ret("big.fromBytes"), int());
        assert_eq!(args("big.toBytes"), vec![int(), "big.Endian".to_string()]);
        assert_eq!(ret("big.toBytes"), "List OF Byte");
    }

    /// The `endian` default pads `big::Endian.Little` (ordinal `0`), typed as the enum
    /// so the verifier's table check sees the parameter's own type.
    #[test]
    fn endian_defaults_to_little() {
        use crate::codegen::registry::default_argument_padding;
        for (name, provided) in [("big.fromBytes", 2), ("big.toBytes", 1)] {
            let padding = default_argument_padding(name, provided, None);
            assert_eq!(padding.len(), 1, "{name}");
            assert_eq!(padding[0].0, ParameterType::named(ENDIAN_TYPE_ID), "{name}");
            assert_eq!(padding[0].1, "0", "{name}");
        }
    }

    /// Every member's native body lowers on every backend (plan-127-A Corrections C3):
    /// the emitters are exercised in process, and a per-target rejection or a
    /// finalizer panic fails here rather than first on a Linux or Windows build.
    #[test]
    fn every_member_lowers_on_every_backend() {
        let source = r#"IMPORT io
IMPORT big

SUB main()
  LET pair AS List OF Byte = [1, 2]
  LET a AS big::Int = big::fromInteger(-5)
  LET b AS big::Int = big::fromBytes(pair, FALSE)
  LET c AS big::Int = big::fromBytes(pair, TRUE, big::Endian.Big)
  LET little AS List OF Byte = big::toBytes(a)
  LET wire AS List OF Byte = big::toBytes(c, big::Endian.Big)
  io::print(toString(big::toInteger(b)))
  io::print(toString(len(little) + len(wire)))
END SUB
"#;
        for target in crate::testutil::CodeTarget::ALL {
            let code = crate::testutil::code_for_src_on(source, target);
            for (name, _) in MEMBERS {
                let member = name.trim_start_matches("big.");
                let body = code
                    .functions
                    .iter()
                    .find(|f| f.name.contains("big") && f.name.ends_with(member))
                    .unwrap_or_else(|| {
                        panic!("{}: no lowered body for {name}", target.name())
                    });
                assert!(
                    !body.instructions.is_empty(),
                    "{}: {name} lowered to an empty body",
                    target.name()
                );
            }
        }
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
