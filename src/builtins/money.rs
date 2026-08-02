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

use std::borrow::Cow;

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinSource, BuiltinType,
    DefaultResolver, Implementation, InjectionRule, Lowering, Parameter, ReturnType, TypeKind,
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
        overloads,
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

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_money_call(name: &str) -> bool {
    DefaultResolver::contains(&MONEY, name)
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

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    DefaultResolver::return_type_name(&MONEY, name)
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    DefaultResolver::resolve_call(&MONEY, name, arg_types).map(|return_type| ResolvedCall {
        return_type: Cow::Borrowed(return_type),
    })
}

// `expected_arguments` returns a `&'static str` the owned `DefaultResolver`
// (which yields `String`) cannot produce, so it stays a static literal PINNED
// equal to `MONEY`'s per-position type rendering by `parity_matches_descriptor`.
pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    let text = match name {
        SET_ROUNDING => "Rounding",
        GET_ROUNDING => "()",
        ROUND => "Money, Integer",
        _ => return None,
    };
    Some(text)
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

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    DefaultResolver::arity(&MONEY, name)
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

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn resolves_the_three_callables() {
        assert_eq!(
            resolve_call(SET_ROUNDING, &strings(&["Rounding"]))
                .unwrap()
                .return_type,
            "Nothing"
        );
        assert_eq!(
            resolve_call(GET_ROUNDING, &[]).unwrap().return_type,
            "Rounding"
        );
        assert_eq!(
            resolve_call(ROUND, &strings(&["Money", "Integer"]))
                .unwrap()
                .return_type,
            "Money"
        );
    }

    #[test]
    fn rejects_wrong_arguments() {
        assert!(resolve_call(SET_ROUNDING, &strings(&["Integer"])).is_none());
        assert!(resolve_call(GET_ROUNDING, &strings(&["Integer"])).is_none());
        assert!(resolve_call(ROUND, &strings(&["Money"])).is_none());
        assert!(resolve_call(ROUND, &strings(&["Integer", "Integer"])).is_none());
    }

    #[test]
    fn arity_and_type_metadata_present() {
        assert_eq!(arity(SET_ROUNDING), Some((1, 1)));
        assert_eq!(arity(GET_ROUNDING), Some((0, 0)));
        assert_eq!(arity(ROUND), Some((2, 2)));
        assert!(is_builtin_type("Rounding"));
        assert!(!is_builtin_type("Money"));
        assert!(is_money_call(ROUND));
        assert!(!is_money_call("money.nope"));
    }

    #[test]
    fn return_type_names_for_each_callable() {
        assert_eq!(call_return_type_name(SET_ROUNDING), Some("Nothing"));
        assert_eq!(call_return_type_name(GET_ROUNDING), Some("Rounding"));
        assert_eq!(call_return_type_name(ROUND), Some("Money"));
        assert_eq!(call_return_type_name("not_a_money_fn"), None);
    }

    #[test]
    fn expected_arguments_and_param_names() {
        assert_eq!(expected_arguments(SET_ROUNDING), Some("Rounding"));
        assert_eq!(expected_arguments(GET_ROUNDING), Some("()"));
        assert_eq!(expected_arguments(ROUND), Some("Money, Integer"));
        assert_eq!(expected_arguments("not_a_money_fn"), None);

        assert!(call_param_names(SET_ROUNDING).is_some());
        assert!(call_param_names(GET_ROUNDING).is_some());
        assert!(call_param_names(ROUND).is_some());
        assert!(call_param_names("not_a_money_fn").is_none());
    }

    #[test]
    fn arity_none_for_unknown() {
        assert_eq!(arity("not_a_money_fn"), None);
    }

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

    // plan-72-Q migration gate: prove `MONEY` reproduces every legacy helper
    // answer for every `money.*` name (and an unknown name) — membership, arity,
    // param names, return type, expected arguments, and the machine argument-type
    // table — pinning the borrowed `call_param_names`/`expected_arguments`/
    // `argument_types` statics equal to `MONEY`, and the `Rounding` enum type.
    // Keep until plan-72-BB deletes the legacy helpers.
    #[test]
    fn parity_matches_descriptor() {
        use crate::builtins::descriptor::parity;

        let calls: Vec<&str> = MONEY_FUNCTIONS.iter().map(|f| f.name).collect();
        let legacy = parity::LegacySet {
            is_call: &is_money_call,
            arity: &arity,
            param_names: &|name| {
                call_param_names(name).map(|rows| rows.iter().map(|row| row.to_vec()).collect())
            },
            return_type_name: &call_return_type_name,
            expected_arguments: Some(&|name| expected_arguments(name).map(str::to_string)),
            param_name_overloads: None,
            argument_types: Some(&|name| argument_types(name).map(|types| types.to_vec())),
            implementation_name: None,
            default_padding: None,
            builtin_type_fields: None,
        };
        let mut probe = calls.clone();
        probe.push("money.nope");
        parity::assert_parity(&MONEY, &probe, &legacy, &[]);

        // The Rounding enum is the descriptor's builtin-type authority.
        assert!(is_builtin_type("Rounding"));
        assert!(!is_builtin_type("Money"));
    }
}
