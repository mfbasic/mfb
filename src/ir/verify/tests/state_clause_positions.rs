//! The `STATE`-type rule (`check_state_clauses` / `state_type_fault`) at the
//! positions and kinds the bug-1013 cases do not reach:
//!
//! - a clause nested in a user template's type argument
//!   (`Box OF fs.File STATE Integer`) is judged like one written bare;
//! - a built-in package's nominal that is not a record (`net.PingStatus`, an
//!   enum) is refused;
//! - an unresolved state type is left to the resolver, never reported here.

use super::*;

/// The rules a function with one parameter of `type_` reports.
fn param_rules(type_: crate::types::ParameterType) -> Vec<String> {
    let mut p = param("h", "Integer", None);
    p.type_ = type_;
    let f = func_returns("run", "Nothing", vec![p], vec![ret_none()]);
    rules(&project(vec![f], vec![]))
}

#[test]
fn a_clause_inside_a_template_argument_is_judged() {
    let got = param_rules(ParameterType::parse("Box OF fs.File STATE Integer"));
    assert!(
        matches!(
            ParameterType::parse("Box OF fs.File STATE Integer"),
            ParameterType::UserOf(..)
        ),
        "the shape under test is a user template"
    );
    assert_eq!(count_rule(&got, "TYPE_STATE_INVALID"), 1, "{got:?}");
}

#[test]
fn a_package_nominal_that_is_not_a_record_is_refused() {
    let mut p = param("h", "Integer", None);
    p.type_ = ParameterType::parse("fs.File STATE net.PingStatus");
    let f = func_returns("run", "Nothing", vec![p], vec![ret_none()]);
    let details: Vec<String> = collect_diagnostics(&project(vec![f], vec![]))
        .into_iter()
        .filter(|d| d.rule == "TYPE_STATE_INVALID")
        .map(|d| d.detail)
        .collect();
    assert_eq!(details.len(), 1, "{details:?}");
    assert!(
        details[0].contains("STATE type `net.PingStatus` is not a record"),
        "refused as NOT A RECORD, not as a non-defaultable one: {details:?}"
    );
}

#[test]
fn an_unresolved_state_type_is_not_reported() {
    let got = param_rules(ParameterType::Stateful {
        base: Box::new(ParameterType::parse("fs.File")),
        state: Box::new(ParameterType::Unknown),
    });
    assert_eq!(count_rule(&got, "TYPE_STATE_INVALID"), 0, "{got:?}");
    assert_eq!(
        count_rule(&got, "TYPE_STATE_REQUIRES_RESOURCE"),
        0,
        "{got:?}"
    );
}
