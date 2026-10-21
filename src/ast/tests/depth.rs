//! The expression-tree depth bound (256) at every node that adds a level:
//! each binary operator's own chain, and each wrapper — NOT, unary minus,
//! power, call, constructor, WITH, lambda, list, map and set literals —
//! around a chain already at the bound. Also the trailing commas and
//! missing closers of those forms. Every chain is flat (left-associative),
//! so the parser loops rather than recursing.

use super::*;

const TOO_DEEP: &str = "Expression nesting is too deep.";

/// `1 + 1 + …` with `ops` additions: a tree `ops` deep.
fn chain(ops: usize) -> String {
    format!("1{}", " + 1".repeat(ops))
}

/// `term OP term OP …` with `ops` operators.
fn chain_of(term: &str, op: &str, ops: usize) -> String {
    format!("{term}{}", format!(" {op} {term}").repeat(ops))
}

fn details(src: &str) -> Vec<String> {
    crate::rules::collecting_diagnostics(|| {
        let _ = parse_source(Path::new("main.mfb"), "main.mfb", src);
    })
    .1
    .into_iter()
    .map(|(_, detail)| detail)
    .collect()
}

fn in_main(expression: &str) -> String {
    format!("FUNC main AS Nothing\n  LET v AS Integer = {expression}\nEND FUNC\n")
}

#[test]
fn a_chain_at_the_bound_parses_and_one_more_level_does_not() {
    assert!(details(&in_main(&chain(256))).is_empty());
    assert_eq!(details(&in_main(&chain(257))), [TOO_DEEP]);
}

#[test]
fn every_binary_operator_s_chain_is_bounded() {
    for (term, op) in [
        ("TRUE", "OR"),
        ("TRUE", "AND"),
        ("1", "="),
        ("\"a\"", "&"),
        ("1", "*"),
    ] {
        assert_eq!(
            details(&in_main(&chain_of(term, op, 257))),
            [TOO_DEEP],
            "{op}"
        );
    }
}

#[test]
fn every_wrapper_around_a_chain_at_the_bound_is_one_level_too_deep() {
    let full = chain(256);
    for wrapped in [
        format!("NOT {full}"),
        format!("-({full})"),
        format!("2 ^ ({full})"),
        format!("f({full})"),
        format!("P[{full}]"),
        format!("WITH p {{ x := {full} }}"),
        format!("LAMBDA() -> {full}"),
        format!("[{full}]"),
        format!("Map OF Integer TO Integer {{ {full} := 1 }}"),
        format!("Set OF Integer {{ {full} }}"),
    ] {
        assert_eq!(
            details(&in_main(&wrapped)),
            [TOO_DEEP],
            "{}",
            &wrapped[..12]
        );
    }
}

#[test]
fn trailing_commas_close_every_list_form() {
    for expression in [
        "f(1, 2,)",
        "P[1, 2,]",
        "[1, 2,]",
        "Map OF Integer TO Integer { 1 := 2, }",
        "Set OF Integer { 1, 2, }",
        "WITH p { x := 1, }",
    ] {
        assert!(details(&in_main(expression)).is_empty(), "{expression}");
    }
}

#[test]
fn an_unclosed_constructor_or_lambda_list_is_named() {
    assert_eq!(
        details(&in_main("P[1 2]"))[0],
        "Expected `]` after constructor arguments."
    );
    assert_eq!(
        details(&in_main("LAMBDA(x AS Integer -> x)"))[0],
        "Lambda must close the parameter list."
    );
    assert_eq!(
        details(&in_main("WITH p { x := 1 y := 2 }"))[0],
        "Expected `}` after WITH updates."
    );
}
