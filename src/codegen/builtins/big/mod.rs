//! The built-in `big` package (plan-127): arbitrary-precision signed integers.
//!
//! `big::Int` is a package-owned, exported, copyable value record — the `net::Url`
//! shape, not a `RES` handle — holding a little-endian magnitude and a sign flag.
//! Every member lowers natively through `Body::abi_function`; the shared emitters
//! every member reads and builds a `big::Int` through live in `gen_big/`, so the
//! canonical form (no trailing zero bytes, never negative zero) is established in
//! exactly one place.

use crate::codegen::registry::{
    EnumVariant, RecordProp, Registry, RegistryEnum, RegistryPackage, RegistryRecord,
};
use crate::types::ParameterType;

mod func_abs;
mod func_add;
mod func_bit_length;
mod func_compare;
mod func_div_mod;
mod func_divide;
mod func_equals;
mod func_factorial;
mod func_from_bytes;
mod func_from_integer;
mod func_gcd;
mod func_is_zero;
mod func_mod_pow;
mod func_multiply;
mod func_negate;
mod func_parse;
mod func_pow;
mod func_product;
mod func_remainder;
mod func_shift_left;
mod func_shift_right;
mod func_sign;
mod func_subtract;
mod func_sum;
mod func_test_bit;
mod func_to_bytes;
mod func_to_integer;
mod func_to_radix_string;
mod func_to_string;

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

/// The `DivResult` record's bare member id and its package-qualified identity.
pub(crate) const DIV_RESULT_TYPE: &str = "DivResult";
pub(crate) const DIV_RESULT_TYPE_ID: &str = "big.DivResult";

const MODULE_INTRO: &str = r#"Signed integers of any size, with arithmetic that never overflows"#;
const MODULE_DESC: &str = r#"The `big` package provides `big::Int`, a signed integer with no fixed size. Where an
`Integer` is 64 bits wide and its arithmetic raises on overflow, a `big::Int` grows to
hold whatever value it is given. `big` is a built-in package: `IMPORT big` needs no
manifest dependency.

A `big::Int` is an ordinary value. Assigning it or passing it to a function makes an
independent copy, and there is nothing to open or close. A `big::Int` declared with
`MUT` and no initializer holds zero.

**Operators do not apply.** `+`, `-`, `*`, `/`, `=`, `<>`, `<` and `>` are rejected at
compile time on a `big::Int`. Compare two values with `big::compare` or
`big::equals`, and test for zero with `big::isZero`. For the same reason a `big::Int`
cannot be a `Map` key or a `Set` element.

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
    // flag inline (`gen_big/`, plan-127-A §4.2). Reordering these silently turns
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

    // Field ORDER is contract: `emit_build_div_result` builds this record through the
    // record marshaller with the quotient in slot 0 and the remainder in slot 1, and
    // both inline into the one block (plan-127-C §4.3).
    pkg.add_record(RegistryRecord {
        name: DIV_RESULT_TYPE,
        export: true,
        description: "The quotient and remainder of one `big::divMod`. `quotient` is truncated toward zero and `remainder` takes the sign of the dividend, so the dividend equals divisor times `quotient` plus `remainder`, with `remainder` smaller in absolute value than the divisor.",
        props: vec![
            RecordProp {
                name: "quotient",
                ty: ParameterType::named(INT_TYPE),
                description: "The quotient, truncated toward zero.",
            },
            RecordProp {
                name: "remainder",
                ty: ParameterType::named(INT_TYPE),
                description: "The remainder: the sign of the dividend, and smaller in absolute value than the divisor.",
            },
        ],
    });

    // Conversion seams (plan-127-A Phase 4).
    func_from_integer::register(&mut pkg);
    func_to_integer::register(&mut pkg);
    func_from_bytes::register(&mut pkg);
    func_to_bytes::register(&mut pkg);
    // Comparison and sign (plan-127-A Phase 5).
    func_compare::register(&mut pkg);
    func_equals::register(&mut pkg);
    func_is_zero::register(&mut pkg);
    func_sign::register(&mut pkg);
    func_abs::register(&mut pkg);
    func_negate::register(&mut pkg);
    // Additive arithmetic (plan-127-B Phase 1).
    func_add::register(&mut pkg);
    func_subtract::register(&mut pkg);
    // Multiplication and the aggregates (plan-127-B Phase 2).
    func_multiply::register(&mut pkg);
    func_sum::register(&mut pkg);
    func_product::register(&mut pkg);
    // Bit operations (plan-127-B Phase 3).
    func_bit_length::register(&mut pkg);
    func_shift_left::register(&mut pkg);
    func_shift_right::register(&mut pkg);
    func_test_bit::register(&mut pkg);
    // Text (plan-127-B Phase 4).
    func_parse::register(&mut pkg);
    func_to_string::register(&mut pkg);
    func_to_radix_string::register(&mut pkg);
    // Division (plan-127-C Phase 2).
    func_divide::register(&mut pkg);
    func_remainder::register(&mut pkg);
    func_div_mod::register(&mut pkg);
    // Built on division (plan-127-C Phase 3).
    func_pow::register(&mut pkg);
    func_gcd::register(&mut pkg);
    func_factorial::register(&mut pkg);
    func_mod_pow::register(&mut pkg);

    r.add_package(pkg);
}

// Man/spec citation anchor: `BIG`. The `big/*` man pages and the stdlib spec chapter
// ground their package-level and value-type facts here with `[[…/big/mod.rs:BIG]]`.

#[cfg(test)]
mod tests;
