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
//! [`crate::codegen::builtins_testing::is_testing_call`], queries this package by bare name
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

// One file per assertion, holding its descriptor AND its man-page prose — the
// same shape every other builtin package uses. The two shared signature helpers
// (`operands`, `assertion`) stay here because every member spells one of them.
mod func_expect_equal;
mod func_expect_fixed;
mod func_expect_float;
mod func_expect_integer;
mod func_expect_n_equal;
mod func_expect_n_fixed;
mod func_expect_n_float;
mod func_expect_n_integer;
mod func_expect_n_string;
mod func_expect_n_trap;
mod func_expect_string;
mod func_expect_trap;

const INTRO: &str = r#"Assertion builtins for the built-in test framework."#;

const DESC: &str = r#"The `testing` package provides the assertion builtins used inside a `TESTING`
block's `TCASE` bodies — `expectEqual`/`expectNEqual`, the typed
`expectFloat`/`expectInteger`/`expectFixed`/`expectString` (and their `expectN*`
inequality twins), and the `expectTrap`/`expectNTrap` trap assertions.

These are **unqualified global** builtins: you write them as bare names
(`expectEqual(actual, expected)`), never `testing::expectEqual`, and there is no
`IMPORT testing`.

An assertion is only valid inside a `TCASE` body; using one anywhere else is a
compile error. Each returns nothing, and the **first failed assertion ends its
case** — later lines in that `TCASE` do not run, while sibling cases and groups
carry on. So a case reports at most one failure, and it is the first one.

A failure prints the case as `[F]` with the mismatch and the source line beneath
it:

```
* a failing case, to see what a failure reports
  * [F] this one is meant to fail
    X expected 99, got 5  (src/main.mfb:49)
```

Which assertion to reach for:

- **`expectEqual`/`expectNEqual`** are the general pair. They compare with the
  language's own `=` and `<>`, so the operands must be comparable and printable
  (a number, `Boolean`, `String`, `Byte`, `Scalar`, or `List OF Byte`) — printable because the failure
  message has to show them.
- **The typed forms** — `expectInteger`, `expectFloat`, `expectFixed`,
  `expectString` and their `expectN…` twins — additionally require both operands
  to be exactly that type. Reach for these when a wrong type would be a real bug
  in the code under test; `expectEqual` would happily compare an `Integer` `1`
  with a `Float` `1.0`, and `expectInteger` will not.
- **`expectTrap`/`expectNTrap`** assert on failure rather than on a value:
  whether evaluating an expression raises. `expectTrap` can also pin the exact
  `error.code`.

Tests live in a `TESTING … END TESTING` block, which `mfb build` drops entirely
— a release binary is identical to one with the tests deleted — and which
`mfb test` compiles and runs. See `mfb spec language test-framework` for the
block structure."#;

/// One assertion's parameter list: `actual` and `expected`, both of operand type
/// `ty` (a concrete scalar for the typed families, or `Var("T")` for the generic
/// `expectEqual`/`expectNEqual`).
pub(super) fn operands(ty: ParameterType) -> Vec<Parameter> {
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
/// `prose` is (intro, desc, example) — the three man-page fields.
pub(super) fn assertion(
    name: &'static str,
    prose: (&'static str, &'static str, &'static str),
    params: Vec<Parameter>,
) -> RegistryFunction {
    let (intro, desc, example) = prose;
    RegistryFunction {
        name,
        intro,
        desc,
        example,
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
    func_expect_equal::register(&mut pkg);
    func_expect_n_equal::register(&mut pkg);

    // Typed equality.
    func_expect_float::register(&mut pkg);
    func_expect_integer::register(&mut pkg);
    func_expect_fixed::register(&mut pkg);
    func_expect_string::register(&mut pkg);

    // Typed inequality.
    func_expect_n_float::register(&mut pkg);
    func_expect_n_integer::register(&mut pkg);
    func_expect_n_fixed::register(&mut pkg);
    func_expect_n_string::register(&mut pkg);

    // Trap assertions.
    func_expect_trap::register(&mut pkg);
    func_expect_n_trap::register(&mut pkg);

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
