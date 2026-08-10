//! Assertion builtins for the built-in test framework (plan-18-testing.md §1).
//!
//! The assertion builtins are compiler-lowered: they are recognized here,
//! type-checked in `syntaxcheck`, and lowered directly in `src/ir/lower.rs`
//! (there is no runtime helper). They are valid only inside a `TCASE` body —
//! placement is enforced by `crate::testing` before any other front-end pass.

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, DefaultResolver, DefaultValue,
    Implementation, Lowering, Parameter, ParameterType, ReturnType,
};

/// `expectEqual(actual, expected)` — pass iff `actual = expected`. Generic: any
/// `=`-comparable, printable operands.
pub(crate) const EXPECT_EQUAL: &str = "expectEqual";
/// `expectNEqual(actual, expected)` — pass iff `actual <> expected`. Generic.
pub(crate) const EXPECT_NEQUAL: &str = "expectNEqual";
/// `expectFloat(actual, expected)` — both operands must be `Float`; pass iff equal.
pub(crate) const EXPECT_FLOAT: &str = "expectFloat";
/// `expectInteger(actual, expected)` — both `Integer`; pass iff equal.
pub(crate) const EXPECT_INTEGER: &str = "expectInteger";
/// `expectFixed(actual, expected)` — both `Fixed`; pass iff equal.
pub(crate) const EXPECT_FIXED: &str = "expectFixed";
/// `expectString(actual, expected)` — both `String`; pass iff equal.
pub(crate) const EXPECT_STRING: &str = "expectString";
/// `expectNFloat(actual, expected)` — both `Float`; pass iff not equal.
pub(crate) const EXPECT_NFLOAT: &str = "expectNFloat";
/// `expectNInteger(actual, expected)` — both `Integer`; pass iff not equal.
pub(crate) const EXPECT_NINTEGER: &str = "expectNInteger";
/// `expectNFixed(actual, expected)` — both `Fixed`; pass iff not equal.
pub(crate) const EXPECT_NFIXED: &str = "expectNFixed";
/// `expectNString(actual, expected)` — both `String`; pass iff not equal.
pub(crate) const EXPECT_NSTRING: &str = "expectNString";
/// `expectTrap(expr)` / `expectTrap(expr, code)` — pass iff evaluating `expr`
/// traps (and, with `code`, the trap's `error.code = code`).
pub(crate) const EXPECT_TRAP: &str = "expectTrap";
/// `expectNTrap(expr)` — pass iff evaluating `expr` does not trap.
pub(crate) const EXPECT_NTRAP: &str = "expectNTrap";

/// The reserved internal error code a failed assertion raises. It sits in the
/// `7-706-*` (trap/failure) subsystem but is deliberately absent from the
/// `errorCode::` registry, so user code can neither name it nor — barring a
/// deliberate `FAIL error(77069001, …)` — collide with it. The synthesized driver
/// recognizes it to distinguish an assertion failure from a genuine runtime error
/// (plan-18-B §3.1).
pub(crate) const TEST_ABORT_CODE: i64 = 77069001;

// plan-72-X: `TESTING` is the descriptor authority for assertion-builtin
// membership. Every assertion is compiler-lowered (no runtime helper, no
// implementation rewrite → `Implementation::Same`, `Lowering::Inline`) and
// type-checks as `Nothing` — `check_expect_call` always returns `Type::Nothing`
// (`src/syntaxcheck/inference.rs`). The two `expectEqual`/`expectNEqual` operands
// are generic (any `=`-comparable, printable type), spelled `"T"` per the
// codebase's generic-parameter convention (`collections`' `"List OF T"`); a typed
// assertion pins both operands to its concrete type. `expectTrap`'s trailing
// `code` is `DefaultValue::Optional` — it widens arity to `(1, 2)` but is not
// default-padded, because the lowering selects the trap-with-code body by argument
// count (like `datetime.parse`'s trailing `zone`), never by injecting a literal.
//
// NOTE for plan-72-BB: `testing` was NEVER routed through the `mod.rs` aggregate
// chains (`is_builtin_call`, `call_return_type_name`, `call_param_names`, …) — the
// assertions are dispatched separately via `is_testing_call` before general
// builtin dispatch (`inference.rs`, `resolver::resolution`, `ir::lower`). So when
// BB collapses those aggregates to registry iteration, it must keep `testing`'s
// functions out of them (or verify the flip is inert, since the testing path
// early-returns first) to preserve today's behavior. This letter only makes
// `is_testing_call` a wrapper over `TESTING` and registers the module.

// Operand parameter lists per assertion family. Named `const` items so the
// nested `&'static` slices are promoted (a `const fn` cannot return a reference
// to a slice it builds from its arguments); the `assert_fn` helper below only
// forwards these already-`'static` overload slices, mirroring `bits`.
const PARAMS_T: &[Parameter] = &[
    Parameter::required("actual", "T"),
    Parameter::required("expected", "T"),
];
const PARAMS_FLOAT: &[Parameter] = &[
    Parameter::required("actual", "Float"),
    Parameter::required("expected", "Float"),
];
const PARAMS_INTEGER: &[Parameter] = &[
    Parameter::required("actual", "Integer"),
    Parameter::required("expected", "Integer"),
];
const PARAMS_FIXED: &[Parameter] = &[
    Parameter::required("actual", "Fixed"),
    Parameter::required("expected", "Fixed"),
];
const PARAMS_STRING: &[Parameter] = &[
    Parameter::required("actual", "String"),
    Parameter::required("expected", "String"),
];
// `expectTrap(expr)` / `expectTrap(expr, code)`: a guardable expression plus an
// optional expected `error.code`. The `code` slot is `Optional` — it widens arity
// to (1, 2) but is not default-padded.
const PARAMS_TRAP: &[Parameter] = &[
    Parameter::required("expression", "T"),
    Parameter {
        name: "code",
        aliases: &[],
        ty: ParameterType::Named("Integer"),
        default: DefaultValue::Optional,
    },
];
const PARAMS_NTRAP: &[Parameter] = &[Parameter::required("expression", "T")];

const OV_T: &[BuiltinOverload] = &[BuiltinOverload {
    params: PARAMS_T,
    return_type: ReturnType::Fixed("Nothing"),
}];
const OV_FLOAT: &[BuiltinOverload] = &[BuiltinOverload {
    params: PARAMS_FLOAT,
    return_type: ReturnType::Fixed("Nothing"),
}];
const OV_INTEGER: &[BuiltinOverload] = &[BuiltinOverload {
    params: PARAMS_INTEGER,
    return_type: ReturnType::Fixed("Nothing"),
}];
const OV_FIXED: &[BuiltinOverload] = &[BuiltinOverload {
    params: PARAMS_FIXED,
    return_type: ReturnType::Fixed("Nothing"),
}];
const OV_STRING: &[BuiltinOverload] = &[BuiltinOverload {
    params: PARAMS_STRING,
    return_type: ReturnType::Fixed("Nothing"),
}];
const OV_TRAP: &[BuiltinOverload] = &[BuiltinOverload {
    params: PARAMS_TRAP,
    return_type: ReturnType::Fixed("Nothing"),
}];
const OV_NTRAP: &[BuiltinOverload] = &[BuiltinOverload {
    params: PARAMS_NTRAP,
    return_type: ReturnType::Fixed("Nothing"),
}];

const fn assert_fn(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        doc_intro: "",
        doc_desc: "",
        errors: &[],
        overloads,
        implementation: Implementation::Same,
        lowering: Lowering::Inline,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

const TESTING_FUNCTIONS: &[BuiltinFunction] = &[
    // Generic equality / inequality (any `=`-comparable, printable operands).
    assert_fn(EXPECT_EQUAL, "expectEqual", OV_T),
    assert_fn(EXPECT_NEQUAL, "expectNEqual", OV_T),
    // Typed equality.
    assert_fn(EXPECT_FLOAT, "expectFloat", OV_FLOAT),
    assert_fn(EXPECT_INTEGER, "expectInteger", OV_INTEGER),
    assert_fn(EXPECT_FIXED, "expectFixed", OV_FIXED),
    assert_fn(EXPECT_STRING, "expectString", OV_STRING),
    // Typed inequality.
    assert_fn(EXPECT_NFLOAT, "expectNFloat", OV_FLOAT),
    assert_fn(EXPECT_NINTEGER, "expectNInteger", OV_INTEGER),
    assert_fn(EXPECT_NFIXED, "expectNFixed", OV_FIXED),
    assert_fn(EXPECT_NSTRING, "expectNString", OV_STRING),
    // Trap assertions.
    assert_fn(EXPECT_TRAP, "expectTrap", OV_TRAP),
    assert_fn(EXPECT_NTRAP, "expectNTrap", OV_NTRAP),
];

pub(crate) static TESTING: BuiltinModule = BuiltinModule {
    name: "testing",
    doc_intro: "",
    doc_desc: "",
    functions: TESTING_FUNCTIONS,
    types: &[],
    source: None,
    resolver: None,
};

/// Whether `name` is one of the assertion builtins. Wrapper over [`TESTING`]
/// (plan-72-X); pinned equal to the legacy family predicates by the parity test
/// until plan-72-BB deletes the wrappers.
pub(crate) fn is_testing_call(name: &str) -> bool {
    DefaultResolver::contains(&TESTING, name)
}

/// An equality assertion (`actual = expected`): the generic `expectEqual` or a
/// typed `expectFloat`/`expectInteger`/`expectFixed`/`expectString`.
pub(crate) fn is_equality_assert(name: &str) -> bool {
    matches!(
        name,
        EXPECT_EQUAL | EXPECT_FLOAT | EXPECT_INTEGER | EXPECT_FIXED | EXPECT_STRING
    )
}

/// An inequality assertion (`actual <> expected`): the generic `expectNEqual` or
/// a typed `expectNFloat`/`expectNInteger`/`expectNFixed`/`expectNString`.
pub(crate) fn is_inequality_assert(name: &str) -> bool {
    matches!(
        name,
        EXPECT_NEQUAL | EXPECT_NFLOAT | EXPECT_NINTEGER | EXPECT_NFIXED | EXPECT_NSTRING
    )
}

/// The exact operand type a *typed* equality/inequality assertion requires, or
/// `None` for the generic `expectEqual`/`expectNEqual` (any comparable operands).
pub(crate) fn expect_operand_type(name: &str) -> Option<&'static str> {
    match name {
        EXPECT_FLOAT | EXPECT_NFLOAT => Some("Float"),
        EXPECT_INTEGER | EXPECT_NINTEGER => Some("Integer"),
        EXPECT_FIXED | EXPECT_NFIXED => Some("Fixed"),
        EXPECT_STRING | EXPECT_NSTRING => Some("String"),
        _ => None,
    }
}

/// The `(min, max)` argument count accepted by an assertion builtin.
pub(crate) fn expect_arity(name: &str) -> Option<(usize, usize)> {
    if is_equality_assert(name) || is_inequality_assert(name) {
        return Some((2, 2));
    }
    match name {
        EXPECT_TRAP => Some((1, 2)),
        EXPECT_NTRAP => Some((1, 1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_every_assertion_family_as_an_expect_call() {
        // Equality, inequality, and the two trap families are all `expect*` calls;
        // anything else (an ordinary function name) is not.
        for name in [
            EXPECT_EQUAL,
            EXPECT_FLOAT,
            EXPECT_INTEGER,
            EXPECT_FIXED,
            EXPECT_STRING,
            EXPECT_NEQUAL,
            EXPECT_NFLOAT,
            EXPECT_NINTEGER,
            EXPECT_NFIXED,
            EXPECT_NSTRING,
            EXPECT_TRAP,
            EXPECT_NTRAP,
        ] {
            assert!(is_testing_call(name), "`{name}` should be an expect call");
        }
        assert!(!is_testing_call("print"));
        assert!(!is_testing_call("expectSomethingElse"));
    }

    #[test]
    fn classifies_equality_and_inequality_families() {
        for name in [
            EXPECT_EQUAL,
            EXPECT_FLOAT,
            EXPECT_INTEGER,
            EXPECT_FIXED,
            EXPECT_STRING,
        ] {
            assert!(is_equality_assert(name));
            assert!(!is_inequality_assert(name));
        }
        for name in [
            EXPECT_NEQUAL,
            EXPECT_NFLOAT,
            EXPECT_NINTEGER,
            EXPECT_NFIXED,
            EXPECT_NSTRING,
        ] {
            assert!(is_inequality_assert(name));
            assert!(!is_equality_assert(name));
        }
        // The trap families are neither an equality nor an inequality assertion.
        assert!(!is_equality_assert(EXPECT_TRAP));
        assert!(!is_inequality_assert(EXPECT_NTRAP));
    }

    #[test]
    fn typed_assertions_carry_their_operand_type() {
        assert_eq!(expect_operand_type(EXPECT_FLOAT), Some("Float"));
        assert_eq!(expect_operand_type(EXPECT_NFLOAT), Some("Float"));
        assert_eq!(expect_operand_type(EXPECT_INTEGER), Some("Integer"));
        assert_eq!(expect_operand_type(EXPECT_NINTEGER), Some("Integer"));
        assert_eq!(expect_operand_type(EXPECT_FIXED), Some("Fixed"));
        assert_eq!(expect_operand_type(EXPECT_NFIXED), Some("Fixed"));
        assert_eq!(expect_operand_type(EXPECT_STRING), Some("String"));
        assert_eq!(expect_operand_type(EXPECT_NSTRING), Some("String"));
        // The generic families and non-assertions have no fixed operand type.
        assert_eq!(expect_operand_type(EXPECT_EQUAL), None);
        assert_eq!(expect_operand_type(EXPECT_NEQUAL), None);
        assert_eq!(expect_operand_type("print"), None);
    }

    #[test]
    fn arity_matches_each_assertion_family() {
        assert_eq!(expect_arity(EXPECT_EQUAL), Some((2, 2)));
        assert_eq!(expect_arity(EXPECT_STRING), Some((2, 2)));
        assert_eq!(expect_arity(EXPECT_NEQUAL), Some((2, 2)));
        assert_eq!(expect_arity(EXPECT_NFIXED), Some((2, 2)));
        // `expectTrap` takes the expression plus an optional code; `expectNTrap`
        // takes exactly the expression.
        assert_eq!(expect_arity(EXPECT_TRAP), Some((1, 2)));
        assert_eq!(expect_arity(EXPECT_NTRAP), Some((1, 1)));
        assert_eq!(expect_arity("print"), None);
    }

    // plan-72-X migration gate: prove `TESTING` reproduces the two real legacy
    // helpers testing owns — membership (`is_testing_call`) and arity
    // (`expect_arity`) — for every assertion name plus a non-member. `testing` has
    // no `call_param_names`/`call_return_type_name` helper (it is not in any
    // `mod.rs` aggregate chain), so those parity rows pin the descriptor to its
    // authored honest shape (`Nothing` return; positional `actual`/`expected`
    // operands never bound by name). Kept until plan-72-BB.
    #[test]
    fn parity_matches_descriptor() {
        use crate::builtins::descriptor::{parity, DefaultResolver, REGISTRY};

        const NAMES: &[&str] = &[
            EXPECT_EQUAL,
            EXPECT_NEQUAL,
            EXPECT_FLOAT,
            EXPECT_INTEGER,
            EXPECT_FIXED,
            EXPECT_STRING,
            EXPECT_NFLOAT,
            EXPECT_NINTEGER,
            EXPECT_NFIXED,
            EXPECT_NSTRING,
            EXPECT_TRAP,
            EXPECT_NTRAP,
        ];

        // The descriptor owns exactly the 12 assertion names — no more, no less.
        assert_eq!(TESTING.functions.len(), NAMES.len());

        let legacy = parity::LegacySet {
            // The real helper wrapped this letter.
            is_call: &is_testing_call,
            // The real hand-written arity helper (not descriptor-owned; stays).
            arity: &expect_arity,
            // No legacy call_param_names for testing; the descriptor's positional
            // operands are the authored shape.
            param_names: &|name| DefaultResolver::param_names(&TESTING, name),
            // Every assertion types as `Nothing`; testing has no aggregate return
            // helper, so pin to the descriptor's authored fixed return.
            return_type_name: &|name| DefaultResolver::return_type_name(&TESTING, name),
            expected_arguments: None,
            param_name_overloads: None,
            argument_types: None,
            implementation_name: None,
            default_padding: None,
            builtin_type_fields: None,
        };
        let mut probe = NAMES.to_vec();
        probe.push("print");
        probe.push("expectSomethingElse");
        parity::assert_parity(&TESTING, &probe, &legacy, &[]);

        // Independent cross-checks of the two real helpers against the descriptor,
        // in case the LegacySet wiring above ever drifts.
        for &name in NAMES {
            assert!(is_testing_call(name), "`{name}` is a testing call");
            assert_eq!(
                DefaultResolver::arity(&TESTING, name),
                expect_arity(name),
                "descriptor arity == expect_arity for `{name}`"
            );
        }
        assert!(!is_testing_call("print"));

        // `expectTrap`'s optional `code` widens arity but is never default-padded
        // (the lowering selects the body by count, not by injecting a literal).
        assert_eq!(DefaultResolver::arity(&TESTING, EXPECT_TRAP), Some((1, 2)));
        assert!(DefaultResolver::default_padding(&TESTING, EXPECT_TRAP, 1).is_empty());

        // Registered and well-formed alongside every other package.
        assert!(REGISTRY.module("testing").is_some());
        assert!(REGISTRY.function(EXPECT_EQUAL).is_some());
        assert_eq!(REGISTRY.duplicate_module_name(), None);
        assert_eq!(REGISTRY.duplicate_function_name(), None);
    }

    #[test]
    fn descriptor_constructors_execute_at_runtime() {
        // `assert_fn` is a const fn invoked only in const context
        // (`TESTING_FUNCTIONS`), so its body never runs at runtime and shows as
        // uncovered. Call it at runtime to exercise (and pin the shape of) the
        // constructor. Use a named const slice to satisfy the `&'static`
        // overloads parameter (E0716).
        const OV: &[BuiltinOverload] = &[BuiltinOverload {
            params: PARAMS_T,
            return_type: ReturnType::Fixed("Nothing"),
        }];
        let func = assert_fn(EXPECT_EQUAL, "expectEqual", OV);
        assert_eq!(func.name, EXPECT_EQUAL);
        assert_eq!(func.doc_slug, "expectEqual");
        assert_eq!(func.overloads.len(), 1);
        // Every assertion is compiler-lowered inline with no rewrite.
        assert_eq!(func.implementation, Implementation::Same);
        assert_eq!(func.lowering, Lowering::Inline);
        assert!(!func.flags.internal_only);
        assert!(!func.flags.return_type_overloaded);
    }
}
