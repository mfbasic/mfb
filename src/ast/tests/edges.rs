//! Parse errors with the message each one gives: TRAP bindings, template
//! parameter lists, a typeless `RES` parameter, resource and alias
//! declarations cut short. Diagnostics are collected, not printed.

use super::*;

/// Parse `src`; the rule and detail of every diagnostic it reported.
fn diagnostics(src: &str) -> Vec<(String, String)> {
    crate::rules::collecting_diagnostics(|| {
        let _ = parse_source(Path::new("main.mfb"), "main.mfb", src);
    })
    .1
}

/// The detail of the first diagnostic `src` reports.
fn first(src: &str) -> String {
    diagnostics(src)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("expected a diagnostic for {src:?}"))
        .1
}

fn in_main(body: &str) -> String {
    format!("FUNC main AS Integer\n{body}\n  RETURN 0\nEND FUNC\n")
}

#[test]
fn a_trap_binding_must_close() {
    assert_eq!(
        first(&in_main("  TRAP (err x)\n  END TRAP")),
        "TRAP error binding must close with `)`."
    );
}

#[test]
fn template_parameter_lists_are_parenthesized() {
    assert_eq!(
        first("TYPE Pair OF A, B\n  a AS A\nEND TYPE\n"),
        "A template with more than one type parameter takes a parenthesized list: write \
         `OF (A, B)`."
    );
    // A second name that is not one stops the list where it is.
    let cut = diagnostics("TYPE Pair OF A, 5\n  a AS A\nEND TYPE\n");
    assert!(
        cut.iter()
            .any(|(_, detail)| detail.ends_with("write `OF (A)`.")),
        "{cut:?}"
    );
    assert_eq!(
        first("TYPE Pair OF (A, 5)\n  a AS A\nEND TYPE\n"),
        "Expected a template parameter name."
    );
    assert_eq!(
        first("TYPE Pair OF (A, B C)\n  a AS A\nEND TYPE\n"),
        "Expected `)` after the template parameter names."
    );
}

#[test]
fn an_included_union_must_be_a_type() {
    assert!(!diagnostics("UNION U INCLUDES 5\n  Integer\nEND UNION\n").is_empty());
}

#[test]
fn a_res_parameter_with_no_type_is_told_its_type_decides() {
    assert_eq!(
        first("FUNC f(RES h) AS Integer\n  RETURN 0\nEND FUNC\n"),
        "Remove `RES` from parameter `h`: a parameter is a resource because its type is one."
    );
    // A trailing comma before the closer is allowed.
    assert!(diagnostics("FUNC f(a AS Integer,) AS Integer\n  RETURN a\nEND FUNC\n").is_empty());
}

#[test]
fn resource_and_alias_declarations_cut_short() {
    assert_eq!(
        first("RESOURCE 5 CLOSE BY f\n"),
        "Resource name must be an identifier."
    );
    assert_eq!(
        first("FUNC f AS pkg::5\n"),
        "Expected identifier after `::`."
    );
}

fn typed(type_name: &str) -> String {
    format!("FUNC main AS Nothing\n  LET v AS {type_name} = NOTHING\nEND FUNC\n")
}

#[test]
fn a_type_argument_list_is_parenthesized_and_closed() {
    assert_eq!(
        first(&typed("Pair OF String, Integer")),
        "A template with more than one type argument takes a parenthesized list: write \
         `Pair OF (String, Integer)`."
    );
    // A second argument that is no type stops the list where it is.
    assert!(diagnostics(&typed("Pair OF Integer, 5"))
        .iter()
        .any(|(_, detail)| detail.ends_with("write `Pair OF (Integer)`.")));
    assert_eq!(
        first(&typed("Pair OF (Integer, String x)")),
        "Expected `)` after the template type arguments."
    );
}

#[test]
fn a_function_type_closes_its_parameters_and_may_end_them_with_a_comma() {
    assert_eq!(
        first(&typed("FUNC(Integer x) AS Integer")),
        "Function type must close with `)`."
    );
    assert!(diagnostics(&typed("FUNC(Integer, String,) AS Integer")).is_empty());
}

#[test]
fn a_doc_example_runs_to_its_end_and_keeps_its_blank_lines() {
    let documented = |example: &str| {
        format!(
            "DOC\nFUNC f\nDESC Adds.\nEXAMPLE\n{example}END DOC\n\
             FUNC f() AS Integer\n  RETURN 1\nEND FUNC\n"
        )
    };
    // The lexer reads an EXAMPLE verbatim up to its END EXAMPLE, so one left
    // open swallows the END DOC and runs to the end of the file.
    assert_eq!(
        first(&documented("  f()\n")),
        "DOC block reached end of file before its `END DOC` line."
    );
    let file = parse_ok(&documented("  LET a = f()\n\n  RETURN a\nEND EXAMPLE\n"));
    let Some(Item::Doc(doc)) = file.items.iter().find(|item| matches!(item, Item::Doc(_))) else {
        panic!("expected a DOC item");
    };
    assert_eq!(doc.examples[0].0, "LET a = f()\n\nRETURN a");
    assert_eq!(DocHeaderKind::Resource.keyword(), "RESOURCE");
}
