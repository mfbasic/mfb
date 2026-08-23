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
//! The three members lower inline (no runtime helper, no source body): each is a
//! `Body::abi_inline_self` self-lowering intrinsic whose call-site lowering (in its
//! `func_*.rs`) reads/writes the arena rounding-mode field. `setRounding`/
//! `getRounding` are infallible; `round` declares
//! `ErrInvalidArgument` (a `decimals` outside `0..5`) and `ErrOverflow` (settling a
//! near-maximum amount upward), a fallibility the inline-`TRAP` census now reads off
//! this registry data. The `Rounding` enum is modeled on the registry via
//! `add_enum` (rendered into the injected source, like `datetime`'s enums); there is
//! no source companion.

use crate::codegen::registry::{EnumVariant, Registry, RegistryEnum, RegistryPackage};

mod func_get_rounding;
mod func_round;
mod func_set_rounding;

pub(crate) mod gen_fixed_math;
pub(crate) mod gen_money_math;

const MODULE_INTRO: &str = r#"Rounding-mode control for Money arithmetic"#;
const MODULE_DESC: &str = r#"The `money` package controls how `Money` **arithmetic** settles the half case and
provides an explicit settling function. `Money` itself is a built-in scalar type
(see `mfb man types numeric`): an exact base-10 fixed-point value scaled to five
decimal places. Its arithmetic (`M / k`, `M * Float`, `M * Fixed`, and the
`toMoney`/`toFixed` conversions) rounds under a per-execution-context mode that
this package reads and writes. `money` is a built-in package: `IMPORT money` needs
no manifest dependency.

The mode is one of the `Rounding` enum members: `Commercial` rounds half away from
zero (the default) and `Banker` rounds half to even (banker's rounding), which
removes the small upward bias of always rounding ties away. The mode is
per-execution-context state: a worker thread inherits the spawning thread's mode
and then diverges independently, consistent with the per-thread RNG and other
arena state. It affects only `Money` arithmetic — it does not change `Fixed` or
`Float` rounding, and it does not change how `toString(Money)` renders a value.
`toString` presentation rounding is a fixed half-away-from-zero rule independent of
the mode, so a logged or displayed amount is a pure function of its value and
precision. This decoupling enables accumulating under one mode and presenting under
another.

`money::round(value, decimals)` explicitly settles an amount to `decimals` places
under the current mode ("compute at five places, book at two"). It stays a
`Money`; contrast `math::round(Money)`, which exits the dimension to the
dimensionless whole-unit `Integer` count with a fixed half-away rule."#;

/// Register the `money` package on the clean-room registry.
///
/// The `Rounding` enum is modeled on the registry (`get_mfb` renders it into the
/// injected source in place of a hand-written `EXPORT ENUM`, exactly as
/// `datetime`'s enums are). Its variants' declaration order fixes the discriminants
/// (`Commercial` = 0, `Banker` = 1), which are the values `setRounding`/`getRounding`
/// store and load. Each of the three members registers itself from its `func_*.rs`.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("money", MODULE_INTRO, MODULE_DESC);

    pkg.add_enum(RegistryEnum {
        name: "Rounding",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Commercial",
                description: "Round half away from zero (the default).",
            },
            EnumVariant {
                name: "Banker",
                description: "Round half to even (banker's rounding).",
            },
        ],
    });

    func_set_rounding::register(&mut pkg);
    func_get_rounding::register(&mut pkg);
    func_round::register(&mut pkg);

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

    #[test]
    fn money_registered_on_the_clean_room_registry() {
        let pkg = registry().resolve_package("money").expect("money package");
        assert_eq!(pkg.functions().len(), 3);
        // The `Rounding` enum is rendered into the injected companion source.
        let source = pkg.get_mfb();
        assert!(source.contains("EXPORT ENUM Rounding"));
        assert!(source.contains("Commercial"));
        assert!(source.contains("Banker"));
    }

    #[test]
    fn rounding_is_a_builtin_type() {
        assert!(registry().is_builtin_type("Rounding"));
        assert!(!registry().is_builtin_type("Money"));
        assert_eq!(
            registry().qualified_builtin_type("money.Rounding"),
            Some("Rounding".to_string())
        );
    }

    #[test]
    fn membership_and_return_types() {
        for name in [SET_ROUNDING, GET_ROUNDING, ROUND] {
            assert_eq!(registry().owning_package(name), Some("money"), "{name}");
            assert!(
                registry::abi_inline_self_lower(name).is_some(),
                "{name} should have a Body::abi_inline_self lowering"
            );
        }
        assert_eq!(registry::call_return_type(SET_ROUNDING), Some("Nothing"));
        assert_eq!(registry::call_return_type(GET_ROUNDING), Some("Rounding"));
        assert_eq!(registry::call_return_type(ROUND), Some("Money"));
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
            Some(vec!["Rounding".to_string()])
        );
        // getRounding takes no arguments -> an empty positional signature.
        assert_eq!(registry::argument_types(GET_ROUNDING), Some(vec![]));
    }
}
