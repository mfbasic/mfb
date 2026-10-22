//! Type-name machinery at the edges: renaming a generic's head and type
//! variables under `Result`/`MapEntry`, reclassifying a template parameter
//! under `Result`, the spellings of the internal argument and attribute
//! types, and measuring and splitting spellings with parentheses and
//! comma-separated generic arguments.

use super::*;

#[test]
fn a_generic_s_head_is_renamed_with_its_arguments() {
    let renamed = ParameterType::parse("Box OF Integer")
        .map_nominals(&|name: &str| (name == "Box").then(|| "pkg.Box".to_string()));
    assert_eq!(renamed.name(), "pkg.Box OF Integer");
}

#[test]
fn type_variables_are_renamed_inside_result_and_map_entry() {
    let t = Symbol::intern("T");
    let u = Symbol::intern("U");
    let rename = |sym: Symbol| (sym == t).then_some(u);
    let result = ParameterType::result_of(ParameterType::var("T")).rename_vars(&rename);
    assert_eq!(result, ParameterType::result_of(ParameterType::var("U")));
    let entry = ParameterType::map_entry_of(ParameterType::var("T"), ParameterType::var("T"))
        .rename_vars(&rename);
    assert_eq!(
        entry,
        ParameterType::map_entry_of(ParameterType::var("U"), ParameterType::var("U"))
    );
}

#[test]
fn a_template_parameter_under_result_becomes_a_type_variable() {
    let declared = ParameterType::result_of(ParameterType::named("T"));
    assert_eq!(
        declared.with_vars(&["T".to_string()]),
        ParameterType::result_of(ParameterType::var("T"))
    );
}

#[test]
fn the_internal_argument_and_attribute_types_have_their_spellings() {
    assert_eq!(ParameterType::Arg(2).name(), "Arg2");
    assert_eq!(ParameterType::AttributeString.name(), "AttributeString");
}

#[test]
fn spellings_with_groups_and_generic_arguments_measure_and_split() {
    // A ` TO ` inside parentheses is not the top-level one.
    assert_eq!(split_top_level_to("(A TO B) TO C"), Some(("(A TO B)", "C")));
    assert_eq!(type_prefix_len("(Integer, String) rest"), Some(17));
    assert_eq!(type_prefix_len("(Integer"), None);
    assert_eq!(type_prefix_len("!x"), None);
    assert_eq!(type_prefix_len("Pair OF Integer, String"), Some(23));
    // A single parenthesized argument is a grouped type, not an argument list.
    assert_eq!(
        ParameterType::parse("Box OF (Integer)").name(),
        ParameterType::parse("Box OF Integer").name()
    );
}
