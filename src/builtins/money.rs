//! Built-in `money::` package seam (plan-29-D).
//!
//! `money::` controls how Money *arithmetic* settles the half case. The
//! `Rounding` enum (`Commercial` / `Banker`) is declared in
//! `money_package.mfb`; the three callables — `setRounding`, `getRounding`, and
//! `round` — are lowered inline in native codegen (`builder_money`), reading and
//! writing the per-arena rounding-mode field. This module owns the syntaxcheck
//! metadata (arity, parameter names, return types) and the source-package
//! plumbing that makes the enum visible.

use std::borrow::Cow;

const SET_ROUNDING: &str = "money.setRounding";
const GET_ROUNDING: &str = "money.getRounding";
const ROUND: &str = "money.round";

/// The public rounding-mode enum defined in `money_package.mfb`, referenced bare
/// (`Rounding`) like every other builtin type.
pub(crate) fn is_builtin_type(name: &str) -> bool {
    name == "Rounding"
}

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_money_call(name: &str) -> bool {
    matches!(name, SET_ROUNDING | GET_ROUNDING | ROUND)
}

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
    let type_ = match name {
        SET_ROUNDING => "Nothing",
        GET_ROUNDING => "Rounding",
        ROUND => "Money",
        _ => return None,
    };
    Some(type_)
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type: &str = match name {
        SET_ROUNDING if exact(arg_types, &["Rounding"]) => "Nothing",
        GET_ROUNDING if arg_types.is_empty() => "Rounding",
        ROUND if exact(arg_types, &["Money", "Integer"]) => "Money",
        _ => return None,
    };
    Some(ResolvedCall {
        return_type: Cow::Borrowed(return_type),
    })
}

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
/// or generic call — returns `None`. This is the same shape `term::param_types`
/// uses and the reason `money` no longer has to be recovered by parsing the
/// `expected_arguments` diagnostic string.
pub(crate) fn argument_types(name: &str) -> Option<&'static [&'static str]> {
    match name {
        SET_ROUNDING => Some(&["Rounding"]),
        ROUND => Some(&["Money", "Integer"]),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    let span = match name {
        SET_ROUNDING => (1, 1),
        GET_ROUNDING => (0, 0),
        ROUND => (2, 2),
        _ => return None,
    };
    Some(span)
}

super::package_source_glue!(
    "money",
    "<builtin-money>",
    "builtins/money.mfb",
    include_str!("money_package.mfb")
);

use super::exact;

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
}
