//! The built-in `testing` assertion package — the clean-room home of the built-in
//! test framework's assertion builtins (`expectEqual`, `expectTrap`, …).
//!
//! Unlike every other migrated package, `testing`'s members are **unqualified
//! global** builtins: they are written as bare names (`expectEqual(a, b)`), never
//! `testing::expectEqual`, and there is no writable `IMPORT testing`. The package is
//! registered under the real name `"testing"` only so it has a home in the registry;
//! because the registry's qualified query surface (`resolve_func` / `owning_package`
//! / `arity` / `rewrite_target`) all require a `.` (`split_once('.')`), a bare
//! `expectEqual` is inert to those, so the real package name costs nothing at call
//! sites. The membership predicate that dispatches these calls,
//! [`crate::builtins::testing::is_testing_call`], queries this package by bare name
//! via [`RegistryPackage::function`].
//!
//! An empty-name (`""`) package is deliberately NOT used: two `""` packages (this
//! one plus the future `general` migration) would collide because `resolve_package`
//! has no duplicate-name guard. Instead the package carries the additive
//! [`RegistryPackage::mark_unqualified_global`] flag, which makes `mfb man2 --all`
//! skip its documentation page (there is no `testing::expect` spelling to advertise).
//!
//! Every assertion is a **compiler-lowered front-end desugar**: the recognized call
//! is rewritten to a block of MFBASIC statements by
//! [`crate::testing::desugar`] at AST→IR lowering (that pass stays in the front end;
//! it must not depend on `codegen`). So each member carries no codegen realization —
//! [`Body::Intrinsic`] — and types as `Nothing`. The two generic families
//! (`expectEqual`/`expectNEqual`) and the two trap families
//! (`expectTrap`/`expectNTrap`) take a generic operand (`Var("T")`); the eight typed
//! families pin both operands to their concrete `Float`/`Integer`/`Fixed`/`String`.
//! `expectTrap`'s trailing `code` is [`DefaultValue::Optional`] — it widens arity to
//! `(1, 2)` but is never default-padded (the desugar selects the trap-with-code body
//! by argument count, never by injecting a literal).

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, Registry, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;
const INTRO: &str = r#"Assertion builtins for the built-in test framework."#;

const DESC: &str = r#"The `testing` package provides the assertion builtins used inside a `TESTING`
block's `TCASE` bodies — `expectEqual`/`expectNEqual`, the typed
`expectFloat`/`expectInteger`/`expectFixed`/`expectString` (and their `expectN*`
inequality twins), and the `expectTrap`/`expectNTrap` trap assertions.

These are **unqualified global** builtins: they are written as bare names
(`expectEqual(actual, expected)`), never `testing::expectEqual`, and there is no
`IMPORT testing`. Each assertion returns `Nothing`; on failure it aborts the case
with the reserved internal error code and reports the mismatch. Every assertion is
compiler-lowered — recognized in the front end and desugared to comparison
statements — so there is no runtime helper."#;

/// One assertion's parameter list: `actual` and `expected`, both of operand type
/// `ty` (a concrete scalar for the typed families, or `Var("T")` for the generic
/// `expectEqual`/`expectNEqual`).
fn operands(ty: ParameterType) -> Vec<Parameter> {
    vec![
        Parameter {
            name: "actual",
            desc: "The value produced by the code under test.",
            aliases: &[],
            ty: ty.clone(),
            default: DefaultValue::None,
        },
        Parameter {
            name: "expected",
            desc: "The value `actual` is asserted against.",
            aliases: &[],
            ty,
            default: DefaultValue::None,
        },
    ]
}

/// Build an equality/inequality assertion `RegistryFunction`: two operands of type
/// `ty`, `Nothing` return, and the `Body::Intrinsic` marker (the assertion is a
/// front-end desugar with no codegen realization).
fn assertion(name: &'static str, intro: &'static str, params: Vec<Parameter>) -> RegistryFunction {
    RegistryFunction {
        name,
        intro,
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params,
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::Intrinsic,
        }],
    }
}

/// Register the `testing` package on the clean-room registry. See the module docs
/// for why it is a real-named-but-unqualified-global package.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("testing", INTRO, DESC);
    pkg.mark_unqualified_global();

    // Generic equality / inequality (any `=`-comparable, printable operands).
    pkg.add_function(assertion(
        "expectEqual",
        "Assert `actual` equals `expected` (generic).",
        operands(ParameterType::var("T")),
    ));
    pkg.add_function(assertion(
        "expectNEqual",
        "Assert `actual` does not equal `expected` (generic).",
        operands(ParameterType::var("T")),
    ));

    // Typed equality.
    pkg.add_function(assertion(
        "expectFloat",
        "Assert two `Float` values are equal.",
        operands(ParameterType::Float),
    ));
    pkg.add_function(assertion(
        "expectInteger",
        "Assert two `Integer` values are equal.",
        operands(ParameterType::Integer),
    ));
    pkg.add_function(assertion(
        "expectFixed",
        "Assert two `Fixed` values are equal.",
        operands(ParameterType::Fixed),
    ));
    pkg.add_function(assertion(
        "expectString",
        "Assert two `String` values are equal.",
        operands(ParameterType::String),
    ));

    // Typed inequality.
    pkg.add_function(assertion(
        "expectNFloat",
        "Assert two `Float` values are not equal.",
        operands(ParameterType::Float),
    ));
    pkg.add_function(assertion(
        "expectNInteger",
        "Assert two `Integer` values are not equal.",
        operands(ParameterType::Integer),
    ));
    pkg.add_function(assertion(
        "expectNFixed",
        "Assert two `Fixed` values are not equal.",
        operands(ParameterType::Fixed),
    ));
    pkg.add_function(assertion(
        "expectNString",
        "Assert two `String` values are not equal.",
        operands(ParameterType::String),
    ));

    // Trap assertions. `expectTrap(expr)` / `expectTrap(expr, code)`: a guardable
    // expression plus an optional expected `error.code`. The `code` slot is
    // `Optional` — it widens arity to (1, 2) but is not default-padded (the desugar
    // selects the trap-with-code body by argument count). `expectNTrap(expr)` takes
    // exactly the expression.
    pkg.add_function(assertion(
        "expectTrap",
        "Assert evaluating an expression traps (optionally with a given error code).",
        vec![
            Parameter {
                name: "expression",
                desc: "The expression asserted to trap.",
                aliases: &[],
                ty: ParameterType::var("T"),
                default: DefaultValue::None,
            },
            Parameter {
                name: "code",
                desc: "The trap's expected `error.code`, if given.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::Optional,
            },
        ],
    ));
    pkg.add_function(assertion(
        "expectNTrap",
        "Assert evaluating an expression does not trap.",
        vec![Parameter {
            name: "expression",
            desc: "The expression asserted not to trap.",
            aliases: &[],
            ty: ParameterType::var("T"),
            default: DefaultValue::None,
        }],
    ));

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The package registers exactly the 12 assertion names, all `Nothing`-returning
    /// `Body::Intrinsic` members, and reproduces the legacy `expect_arity` shape —
    /// `(2, 2)` for the equality/type families, `(1, 2)` for `expectTrap` (optional
    /// `code`), and `(1, 1)` for `expectNTrap`.
    #[test]
    fn registers_the_twelve_assertions_with_legacy_arities() {
        let mut r = Registry::new();
        register(&mut r);
        let pkg = r.resolve_package("testing").expect("testing registered");
        assert!(pkg.is_unqualified_global());

        const NAMES: &[&str] = &[
            "expectEqual",
            "expectNEqual",
            "expectFloat",
            "expectInteger",
            "expectFixed",
            "expectString",
            "expectNFloat",
            "expectNInteger",
            "expectNFixed",
            "expectNString",
            "expectTrap",
            "expectNTrap",
        ];
        assert_eq!(pkg.functions().len(), NAMES.len());

        for &name in NAMES {
            let func = pkg
                .function(name)
                .unwrap_or_else(|| panic!("`{name}` missing"));
            let imp = func.implementations().first().expect("one implementation");
            assert_eq!(
                imp.return_type,
                ParameterType::Nothing,
                "{name} returns Nothing"
            );
            assert!(
                matches!(imp.body, Body::Intrinsic),
                "{name} is Body::Intrinsic"
            );

            let expected = match name {
                "expectTrap" => (1, 2),
                "expectNTrap" => (1, 1),
                _ => (2, 2),
            };
            assert_eq!(
                func.arity(),
                Some(expected),
                "arity of `{name}` matches legacy expect_arity",
            );
        }

        // The package injects no source (all Intrinsic, no records/types), so the
        // generic `augment_project` pass skips it.
        assert!(pkg.get_mfb().is_empty());
    }
}
