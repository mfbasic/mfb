//! bug-642: a rule that fails inside a built-in package's generic body is reported at
//! the user's call that instantiated it, and its detail names the member — never a
//! line of the injected body.
//!
//! Both checkers carry the rewording: `ir::shape` (here, `collections::distinct`'s
//! comparable check) and `ir::verify` (`collections::sort`'s `<`). The fixture's
//! `golden/build.log` pins the rendered text through `scripts/test-accept.sh`, which
//! spawns `mfb`; this runs the same two passes in process.

use crate::testutil::{check_src_details, fixture_src};

#[test]
fn a_rule_failing_in_a_generic_builtin_body_is_reported_at_the_users_call() {
    let source = fixture_src("bug642-generic-builtin-error-at-call-invalid");
    let reported: Vec<(String, usize, String)> = check_src_details(&source)
        .into_iter()
        .map(|d| (d.rule, d.line, d.detail))
        .collect();
    let prefix = |member: &str| format!("In the body of `{member}`, instantiated by this call: ");
    let expected = [
        (
            "TYPE_REQUIRES_COMPARABLE",
            22,
            format!(
                "{}Call to `collections.contains` requires a comparable type, got `FUNC(Integer) AS Boolean`.",
                prefix("collections::distinct")
            ),
        ),
        (
            "TYPE_BINARY_OPERATOR_MISMATCH",
            13,
            format!(
                "{}Operator `<` requires numeric or String operands, got Boolean and Boolean.",
                prefix("collections::sort")
            ),
        ),
        (
            "TYPE_BINARY_OPERATOR_MISMATCH",
            19,
            format!(
                "{}Operator `<` requires numeric or String operands, got Boolean and Boolean.",
                prefix("collections::sort")
            ),
        ),
        (
            "TYPE_BINARY_OPERATOR_MISMATCH",
            16,
            format!(
                "{}Operator `<` requires numeric or String operands, got Nothing and Nothing.",
                prefix("collections::sort")
            ),
        ),
        (
            "TYPE_BINARY_OPERATOR_MISMATCH",
            20,
            format!(
                "{}Operator `<` requires numeric or String operands, got Boolean and Boolean.",
                prefix("collections::sortBy")
            ),
        ),
    ];
    let expected: Vec<(String, usize, String)> = expected
        .into_iter()
        .map(|(rule, line, detail)| (rule.to_string(), line, detail))
        .collect();
    assert_eq!(reported, expected);
}
