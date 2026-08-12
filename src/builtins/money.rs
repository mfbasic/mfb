//! Built-in `money::` package seam (plan-29-D).
//!
//! `money::` controls how Money *arithmetic* settles the half case. The
//! `Rounding` enum (`Commercial` / `Banker`) is declared in
//! `money_package.mfb`; the three callables — `setRounding`, `getRounding`, and
//! `round` — are lowered inline in native codegen (`builder_money`), reading and
//! writing the per-arena rounding-mode field. This module owns the syntaxcheck
//! metadata (arity, parameter names, return types) and the source-package
//! plumbing that makes the enum visible.
//!
//! plan-72-Q: `MONEY` is the descriptor authority. money is fully data-only —
//! every call has fixed positional argument types and a fixed return, so
//! `resolve_call`, `argument_types`, `expected_arguments`, `arity`, and
//! `call_return_type_name` all derive from the descriptor with no resolver. The
//! calls lower inline (no implementation rewrite → `Implementation::Same`). The
//! `Rounding` enum is a builtin type; the `package_source_glue!` companion is
//! `WhenImported`.

use crate::target::shared::registry::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinSource, BuiltinType,
    Implementation, InjectionRule, Lowering, Parameter, ReturnType, TypeKind,
};

const SET_ROUNDING: &str = "money.setRounding";
const GET_ROUNDING: &str = "money.getRounding";
const ROUND: &str = "money.round";

const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}

const fn money_fn(
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
        doc_example: "",
        implementation: Implementation::Same,
        lowering: Lowering::Inline,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

const P_MODE: &[Parameter] = &[Parameter::required("mode", "Rounding")];
const P_ROUND: &[Parameter] = &[
    Parameter::required("value", "Money"),
    Parameter::required("decimals", "Integer"),
];

const OV_SET_ROUNDING: &[BuiltinOverload] = &[ov(P_MODE, "Nothing")];
const OV_GET_ROUNDING: &[BuiltinOverload] = &[ov(&[], "Rounding")];
const OV_ROUND: &[BuiltinOverload] = &[ov(P_ROUND, "Money")];

const MONEY_FUNCTIONS: &[BuiltinFunction] = &[
    money_fn(SET_ROUNDING, "setRounding", OV_SET_ROUNDING),
    money_fn(GET_ROUNDING, "getRounding", OV_GET_ROUNDING),
    money_fn(ROUND, "round", OV_ROUND),
];

/// The public rounding-mode enum defined in `money_package.mfb`, referenced bare
/// (`Rounding`) like every other builtin type.
const MONEY_TYPES: &[BuiltinType] = &[BuiltinType {
    name: "Rounding",
    kind: TypeKind::Enum,
    fields: &[],
}];

pub(crate) static MONEY: BuiltinModule = BuiltinModule {
    name: "money",
    doc_intro: "",
    doc_desc: "",
    functions: MONEY_FUNCTIONS,
    types: MONEY_TYPES,
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: None,
};

/// The public rounding-mode enum defined in `money_package.mfb`, referenced bare
/// (`Rounding`) like every other builtin type.
pub(crate) fn is_builtin_type(name: &str) -> bool {
    MONEY.types.iter().any(|ty| ty.name == name)
}

// `call_param_names` returns a `&'static` borrowed shape the owned
// `DefaultResolver` (which yields `Vec`) cannot produce, so it stays a static
// literal PINNED equal to `MONEY` by `parity_matches_descriptor`.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    let params: &'static [&'static [&'static str]] = match name {
        SET_ROUNDING => &[&["mode"]],
        GET_ROUNDING => &[],
        ROUND => &[&["value"], &["decimals"]],
        _ => return None,
    };
    Some(params)
}

/// The machine-readable positional argument-type signature (bug-340 A1): the
/// concrete per-parameter types IR lowering hands to `call_argument_expected_type`.
/// `getRounding` takes no arguments (nothing to type), so it — like an overloaded
/// or generic call — returns `None`. Returns a `&'static` borrowed slice the owned
/// `DefaultResolver::argument_types` (which yields `Vec`) cannot produce, so it
/// stays a static literal PINNED equal to `MONEY` by `parity_matches_descriptor`.
pub(crate) fn argument_types(name: &str) -> Option<&'static [&'static str]> {
    match name {
        SET_ROUNDING => Some(&["Rounding"]),
        ROUND => Some(&["Money", "Integer"]),
        _ => None,
    }
}

super::package_source_glue!(
    "money",
    "<builtin-money>",
    "builtins/money.mfb",
    include_str!("money_package.mfb")
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_types_machine_table() {
        // bug-340 A1: the machine-readable positional signature IR lowering reads.
        assert_eq!(argument_types(ROUND), Some(&["Money", "Integer"][..]));
        assert_eq!(argument_types(SET_ROUNDING), Some(&["Rounding"][..]));
        // getRounding takes no arguments -> nothing to type.
        assert_eq!(argument_types(GET_ROUNDING), None);
        assert_eq!(argument_types("money.nope"), None);
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn descriptor_constructors_execute_at_runtime() {
        // `ov`/`money_fn` are const fns used only in const context, so their
        // bodies never run at runtime. Call them at runtime to cover the shape.
        let overload = ov(P_ROUND, "Money");
        assert_eq!(overload.params.len(), 2);
        assert_eq!(overload.return_type, ReturnType::Fixed("Money"));

        let func = money_fn(ROUND, "round", OV_ROUND);
        assert_eq!(func.name, ROUND);
        assert_eq!(func.doc_slug, "round");
        assert_eq!(func.implementation, Implementation::Same);
        assert_eq!(func.lowering, Lowering::Inline);
        assert_eq!(func.overloads.len(), 1);
        assert!(!func.flags.internal_only);
        assert!(!func.flags.return_type_overloaded);
    }

    #[test]
    fn is_builtin_type_recognizes_rounding() {
        assert!(is_builtin_type("Rounding"));
        assert!(!is_builtin_type("Money"));
        assert!(!is_builtin_type("Nope"));
    }
}
