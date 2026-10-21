//! The built-in `money` package (plan-95 migration).
//!
//! `money` controls how `Money` **arithmetic** settles the exact-half case and
//! provides an explicit settling function. `Money` itself is a built-in scalar
//! value type (an exact base-10 fixed-point value scaled to five decimal places);
//! this package reads and writes the per-execution-context rounding mode that
//! `Money` division, `Float`/`Fixed` scaling, and the `toMoney`/`toFixed`
//! conversions round under. The mode is one of the two `Rounding` enum members
//! (`Commercial` = 0, the default; `Banker` = 1) — the discriminants are exactly
//! the stored values.
//!
//! The three rounding members lower inline (no runtime helper, no source body): each
//! is a `Body::abi_inline` self-lowering intrinsic whose call-site lowering (in its
//! `func_*.rs`) reads/writes the arena rounding-mode field. `setRounding`/
//! `getRounding` are infallible; `round` declares
//! `ErrInvalidArgument` (a `decimals` outside `0..5`) and `ErrOverflow` (settling a
//! near-maximum amount upward), a fallibility the inline-`TRAP` census now reads off
//! this registry data. The `Rounding` enum is modeled on the registry via
//! `add_enum` (rendered into the injected source, like `datetime`'s enums).
//!
//! The `Rate` record (plan-181) and its text forms — both `toRate` overloads and the
//! `toString(Rate)` override — are MFBASIC source bodies in the injected companion,
//! which therefore imports `strings`. `convert` and `scale` (plan-181-B) are
//! `Body::abi_inline` lowerings over `gen_money_math::emit_money_rescale`, which
//! read each `Rate` argument's `scaled` word directly.

use crate::codegen::registry::{
    EnumVariant, RecordProp, Registry, RegistryEnum, RegistryPackage, RegistryRecord,
};
use crate::types::ParameterType;

mod func_convert;
mod func_get_rounding;
mod func_round;
mod func_scale;
mod func_set_rounding;
mod func_to_rate;
mod func_to_rate_integer;
mod func_to_rate_string;
mod func_to_string_rate;

pub(crate) mod gen_fixed_math;
pub(crate) mod gen_money_math;

/// The `Rate` record's bare member id — the `RegistryRecord` name.
pub(crate) const RATE_TYPE: &str = "Rate";

/// A `Rate`'s `scaled` field is the rate multiplied by this constant (10^12), so
/// `scaled` 1 is the smallest rate, `0.000000000001`. It is a fixed property of the
/// type, not a parameter of any function.
pub(crate) const RATE_SCALE: i64 = 1_000_000_000_000;

const MODULE_INTRO: &str = r#"Rounding-mode control and exact rates for Money arithmetic"#;
const MODULE_DESC: &str = r#"The `money` package controls how `Money` **arithmetic** settles the half case and
provides an explicit settling function. `Money` itself is a built-in scalar type
(see `mfb man types numeric`): an exact base-10 fixed-point value scaled to five
decimal places. This package reads and writes a per-thread rounding mode used
by `money::round`, `money::convert`, `money::scale`, Integer/Byte division, Float
and Fixed scaling, conversion
between Money and Fixed, and conversion from Float or String. `money` is a built-in
package: `IMPORT money` needs no manifest dependency.

The mode is one of the `money::Rounding` enum members: `Commercial` rounds half away from
zero (the default) and `Banker` rounds half to even (banker's rounding), which
removes the small upward bias of always rounding ties away. The mode is
per-thread: a worker thread inherits the spawning thread's mode
and then changes independently. The mode is shared by calls on that thread,
not a separate setting for each imported package. It affects only Money
arithmetic — it does not change ordinary Fixed or
`Float` rounding, and it does not change how `toString(Money)` renders a value.
`toString` presentation rounding is a fixed half-away-from-zero rule independent of
the mode, so a logged or displayed amount is a pure function of its value and
precision. This decoupling enables accumulating under one mode and presenting under
another.

Scaling by Fixed rounds the exact result once: `0.00003m * 0.5F` is `0.00002m`
under either mode, and `0.00005m * 0.5F` is `0.00003m` under `Commercial` and
`0.00002m` under `Banker`.

`money::round(value, decimals)` explicitly settles an amount to `decimals` places
under the current mode ("compute at five places, book at two"). It stays a
`Money`; contrast `math::round(Money)`, which exits the dimension to the
dimensionless whole-unit `Integer` count with a fixed half-away rule.

**Exact rates.** `money::Rate` holds a rate, such as an exchange rate, as an exact
decimal with up to 12 places. Build one from text with `money::toRate("0.842")`, or
from a whole number with `money::toRate(1)`. The text is read exactly, so the rate is
the number written; a `Fixed` or `Float` rate is binary and is not. A rate must be
above zero and at most `9223372.036854775807`, and text with more than 12 decimal
places is rejected rather than rounded. `toString` on a rate prints it with trailing
zeros removed: `toString(money::toRate("0.84200"))` is `0.842`. The record's one
field, `scaled`, is the rate times 10^12; a rate built by hand with `scaled` at or
below zero is refused by each `money` function that uses it.

`money::convert(value, from, to)` moves an amount between currencies: it returns
`value × from / to`, where `from` and `to` are the two currencies' rates against a
common base. `money::scale(value, rate)` returns `value × rate`, for a tax or a fee.
Both compute the exact result and round it once, under the current mode, so no
binary rate and no intermediate rounding enters the answer; a result outside the
`Money` range raises `ErrOverflow`."#;

/// Register the `money` package on the clean-room registry.
///
/// The `Rounding` enum is modeled on the registry (`get_mfb` renders it into the
/// injected source in place of a hand-written `EXPORT ENUM`, exactly as
/// `datetime`'s enums are). Its variants' declaration order fixes the discriminants
/// (`Commercial` = 0, `Banker` = 1), which are the values `setRounding`/`getRounding`
/// store and load. Each of the three members registers itself from its `func_*.rs`.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("money", MODULE_INTRO, MODULE_DESC);

    // `strings` backs the `Rate` text forms: `toRate(String)` reads the text's bytes
    // and `toString(Rate)` pads the fraction.
    pkg.add_imports(vec!["strings"]);

    // Field ORDER and MEANING are contract: `scaled` is the only slot, and it holds
    // the rate times `RATE_SCALE` (10^12). `toRate`, `convert`, `scale` and the
    // `toString(Rate)` override all read or build it at that scale.
    pkg.add_record(RegistryRecord {
        name: RATE_TYPE,
        export: true,
        description: "An exact decimal rate, such as an exchange rate. The rate is `scaled` \
                      divided by 10^12, so it carries up to 12 decimal places with no binary \
                      rounding. Build one from text with `money::toRate`. A usable rate is \
                      above zero: a `money` function that takes a `Rate` raises \
                      `ErrInvalidArgument` for one whose `scaled` is zero or negative.",
        props: vec![RecordProp {
            name: "scaled",
            ty: ParameterType::Integer,
            description: "The rate multiplied by 10^12. `842000000000` is the rate `0.842`.",
        }],
    });

    pkg.add_enum(RegistryEnum {
        name: "Rounding",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Commercial",
                description: "Round half away from zero (the default).",
                advisory: None,
            },
            EnumVariant {
                name: "Banker",
                description: "Round half to even (banker's rounding).",
                advisory: None,
            },
        ],
    });

    func_set_rounding::register(&mut pkg);
    func_get_rounding::register(&mut pkg);
    func_round::register(&mut pkg);
    func_to_rate::register(&mut pkg);
    func_to_string_rate::register(&mut pkg);
    func_convert::register(&mut pkg);
    func_scale::register(&mut pkg);

    r.add_package(pkg);
}

// Man/spec citation anchor: `MONEY`. The `money/*` man pages and the §13 spec ground
// their package-level and value-type facts here with `[[…/money/mod.rs:MONEY]]`.

#[cfg(test)]
mod tests {
    use crate::codegen::registry::{self, registry};

    const SET_ROUNDING: &str = "money.setRounding";
    const GET_ROUNDING: &str = "money.getRounding";
    const ROUND: &str = "money.round";
    const TO_RATE: &str = "money.toRate";
    const CONVERT: &str = "money.convert";
    const SCALE: &str = "money.scale";

    #[test]
    fn money_registered_on_the_clean_room_registry() {
        let pkg = registry().resolve_package("money").expect("money package");
        // The three rounding members, `toRate` (plan-181-A), and `convert`/`scale`
        // (plan-181-B).
        assert_eq!(pkg.functions().len(), 6);
        // The `Rounding` enum is rendered into the injected companion source.
        let source = pkg.get_mfb();
        assert!(source.contains("EXPORT ENUM Rounding"));
        assert!(source.contains("Commercial"));
        assert!(source.contains("Banker"));
        // So are the `Rate` record and its source bodies.
        assert!(source.contains("EXPORT TYPE Rate"));
        assert!(source.contains("FUNC __money_toRateString(text AS String) AS Rate"));
        assert!(source.contains("FUNC __money_toRateInteger(value AS Integer) AS Rate"));
        assert!(source.contains("FUNC __money_rateToString(rate AS Rate) AS String"));
    }

    #[test]
    fn rate_is_a_builtin_record_and_to_rate_returns_it() {
        assert_eq!(
            registry().qualified_builtin_type("money.Rate"),
            Some("money.Rate".to_string())
        );
        assert_eq!(registry().owning_package(TO_RATE), Some("money"));
        assert_eq!(
            registry::call_return_type_typed(TO_RATE)
                .map(|t| t.name().into_owned())
                .as_deref(),
            Some("money.Rate")
        );
        for code in [
            "ErrInvalidFormat",
            "ErrInvalidArgument",
            "ErrOverflow",
            "ErrOutOfMemory",
        ] {
            assert!(registry().declares_error(TO_RATE, code), "{code}");
        }
    }

    #[test]
    fn convert_and_scale_lower_inline_and_take_rates() {
        for name in [CONVERT, SCALE] {
            assert_eq!(registry().owning_package(name), Some("money"), "{name}");
            assert!(
                registry::abi_inline_lower(name).is_some(),
                "{name} should have a Body::abi_inline lowering"
            );
            assert_eq!(
                registry::call_return_type_typed(name)
                    .map(|t| t.name().into_owned())
                    .as_deref(),
                Some("Money"),
                "{name}"
            );
            assert!(registry().declares_error(name, "ErrInvalidArgument"));
            assert!(registry().declares_error(name, "ErrOverflow"));
        }
        assert_eq!(
            registry::argument_types(CONVERT),
            Some(vec![
                "Money".to_string(),
                "money.Rate".to_string(),
                "money.Rate".to_string()
            ])
        );
        assert_eq!(
            registry::argument_types(SCALE),
            Some(vec!["Money".to_string(), "money.Rate".to_string()])
        );
    }

    #[test]
    fn rounding_is_a_builtin_type() {
        assert!(registry().is_builtin_type("Rounding"));
        assert!(!registry().is_builtin_type("Money"));
        assert_eq!(
            registry().qualified_builtin_type("money.Rounding"),
            Some("money.Rounding".to_string())
        );
    }

    #[test]
    fn membership_and_return_types() {
        for name in [SET_ROUNDING, GET_ROUNDING, ROUND] {
            assert_eq!(registry().owning_package(name), Some("money"), "{name}");
            assert!(
                registry::abi_inline_lower(name).is_some(),
                "{name} should have a Body::abi_inline lowering"
            );
        }
        assert_eq!(
            registry::call_return_type_typed(SET_ROUNDING)
                .map(|t| t.name().into_owned())
                .as_deref(),
            Some("Nothing")
        );
        assert_eq!(
            registry::call_return_type_typed(GET_ROUNDING)
                .map(|t| t.name().into_owned())
                .as_deref(),
            Some("money.Rounding")
        );
        assert_eq!(
            registry::call_return_type_typed(ROUND)
                .map(|t| t.name().into_owned())
                .as_deref(),
            Some("Money")
        );
    }

    #[test]
    fn only_round_is_fallible() {
        // `round` declares `ErrInvalidArgument`/`ErrOverflow`; the two mode ops are
        // total. This is the fact the inline-`TRAP` fallibility census keys on.
        assert_eq!(registry::native_member_declares_error(ROUND), Some(true));
        assert!(registry().declares_error(ROUND, "ErrInvalidArgument"));
        assert!(registry().declares_error(ROUND, "ErrOverflow"));
        assert_eq!(
            registry::native_member_declares_error(SET_ROUNDING),
            Some(false)
        );
        assert_eq!(
            registry::native_member_declares_error(GET_ROUNDING),
            Some(false)
        );
    }

    #[test]
    fn machine_argument_types() {
        assert_eq!(
            registry::argument_types(ROUND),
            Some(vec!["Money".to_string(), "Integer".to_string()])
        );
        assert_eq!(
            registry::argument_types(SET_ROUNDING),
            Some(vec!["money.Rounding".to_string()])
        );
        // getRounding takes no arguments -> an empty positional signature.
        assert_eq!(registry::argument_types(GET_ROUNDING), Some(vec![]));
    }
}
