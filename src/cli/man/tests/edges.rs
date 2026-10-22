// ---------------------------------------------------------------------------
// cli/man.rs — a guide topic with --all, the command-line refusals for an
// unknown name with a page and for a third operand, the no-types page of a
// package with both functions and constants, and the Arg(n) return-type
// substitution through nested type constructors.
// ---------------------------------------------------------------------------

use super::*;

fn s(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

/// A guide topic no package name claims.
fn guide_topic() -> &'static crate::docs::man::ManTopic {
    crate::docs::man::topics()
        .iter()
        .find(|topic| registry().resolve_package(topic.name).is_none() && !topic.pages.is_empty())
        .expect("a guide topic with pages that no package shadows")
}

/// `mfb man <topic> --all` is the overview followed by every one of its pages.
#[test]
fn a_guide_topic_with_all_prints_every_page() {
    let topic = guide_topic();
    assert_eq!(show_man(&s(&[topic.name, "--all"])), Ok(()));
    let all = render_topic_all_markdown(topic);
    assert!(all.starts_with(&render_topic_overview(topic)));
    for page in &topic.pages {
        assert!(all.contains(page.page), "page `{}` is missing", page.name);
    }
}

#[test]
fn an_unknown_name_with_a_page_is_an_unknown_package() {
    assert_eq!(
        show_man(&s(&["nosuchthing", "page"])),
        Err("mfb man: unknown package `nosuchthing`".to_string())
    );
}

#[test]
fn a_third_operand_is_a_usage_error() {
    assert_eq!(
        show_man(&s(&["a", "b", "c"])),
        Err("Usage: mfb man <package> [function] [--all]".to_string())
    );
}

/// A package with functions and constants but no public types points its
/// `types` page at both.
#[test]
fn the_no_types_page_of_a_package_with_functions_and_constants_names_both() {
    let package = registry()
        .packages()
        .iter()
        .find(|package| {
            !has_public_types(package)
                && !package.constants().is_empty()
                && package.functions().iter().any(|f| !f.internal_only)
        })
        .expect("a package with functions and constants and no public types");
    assert_eq!(
        render_no_types_markdown(package),
        format!(
            "The `{0}` package has no public types.\n\nRun `mfb man {0}` to list its functions \
             and constants.\n",
            package.import_name()
        )
    );
}

/// bug-616: an `Arg(n)` placeholder nested in a map entry, a result, a
/// resource or a function type resolves to the n-th parameter's type.
#[test]
fn a_nested_arg_placeholder_resolves_inside_every_constructor() {
    let mut implementation = registry()
        .packages()
        .iter()
        .flat_map(|package| package.functions())
        .flat_map(|function| function.implementations.iter())
        .find(|implementation| {
            implementation.params.len() == 1
                && implementation.params[0].ty == ParameterType::Integer
        })
        .expect("a one-Integer-parameter overload")
        .clone();
    let arg = || ParameterType::Arg(0);
    for (return_type, shown) in [
        (
            ParameterType::map_entry_of(arg(), arg()),
            "MapEntry OF Integer TO Integer",
        ),
        (ParameterType::result_of(arg()), "#Result OF Integer"),
        (ParameterType::res(arg()), "RES Integer"),
        (
            ParameterType::func(vec![arg()], arg()),
            "FUNC(Integer) AS Integer",
        ),
        (
            ParameterType::func_isolated(vec![arg()], arg()),
            "ISOLATED FUNC(Integer) AS Integer",
        ),
    ] {
        implementation.return_type = return_type;
        assert_eq!(resolved_return_type(&implementation).display(), shown);
    }
}
