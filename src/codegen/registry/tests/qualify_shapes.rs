//! `qualify.rs`: package-qualifying the type leaves of a spelled type.
//!
//! A descriptor spells its signature types bare; the registry maps each NOMINAL leaf
//! to its package-qualified identity (`List OF Json` becomes `List OF json.Json`), and
//! renders a record field back into source with the LOCAL name left bare. Each shape
//! that can hold a leaf must be descended into — a user generic's head and arguments,
//! a `Result`, a `Set` — or a package's own signature would name a type the
//! type system cannot find. A leaf the package does not declare is an authoring error
//! and stops the build loudly.

use super::*;
use std::collections::HashMap;

fn owners() -> HashMap<String, Vec<String>> {
    HashMap::from([
        ("Json".to_string(), vec!["json".to_string()]),
        ("Box".to_string(), vec!["json".to_string()]),
        (
            "Shared".to_string(),
            vec!["json".to_string(), "other".to_string()],
        ),
    ])
}

/// A user generic qualifies its head AND its arguments; a `Result` and a `Set`
/// are descended into.
#[test]
fn a_user_generic_a_result_and_a_set_are_qualified_through() {
    let owners = owners();
    let boxed = ParameterType::user_of("Box", vec![ParameterType::named("Json")]);
    assert_eq!(
        qualify_type_leaves(&boxed, "json", &owners),
        ParameterType::user_of("json.Box", vec![ParameterType::named("json.Json")])
    );
    assert_eq!(
        qualify_type_leaves(
            &ParameterType::result_of(ParameterType::named("Json")),
            "json",
            &owners
        ),
        ParameterType::result_of(ParameterType::named("json.Json"))
    );
    assert_eq!(
        qualify_type_leaves(
            &ParameterType::set_of(ParameterType::named("Json")),
            "json",
            &owners
        ),
        ParameterType::set_of(ParameterType::named("json.Json"))
    );
    // A head that no package declares (a user template) keeps its own name; its
    // arguments are still qualified.
    let foreign = ParameterType::user_of("Mine", vec![ParameterType::named("Json")]);
    assert_eq!(
        qualify_type_leaves(&foreign, "json", &owners),
        ParameterType::user_of("Mine", vec![ParameterType::named("json.Json")])
    );
}

/// Rendered back into source, a local name stays bare — the type the parser produces
/// for the package's own declaration — while a signature qualifies it.
#[test]
fn a_rendered_field_keeps_its_local_name_bare() {
    let owners = owners();
    let json = ParameterType::list_of(ParameterType::named("Json"));
    assert_eq!(
        qualify_type_leaves_for_source(&json, "json", &owners),
        json,
        "a local name needs no prefix in source"
    );
    assert_eq!(
        qualify_type_leaves(&json, "json", &owners),
        ParameterType::list_of(ParameterType::named("json.Json"))
    );
}

/// A bare name the package does not declare is an authoring error: the build stops and
/// the message names the leaf and who declares it.
#[test]
fn a_bare_name_the_package_does_not_declare_is_refused() {
    let owners = owners();
    let outcome = crate::testutil::silence_panics(|| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            qualify_type_leaves(&ParameterType::named("Shared"), "third", &owners)
        }))
    });
    let payload = outcome.expect_err("a leaf this package does not declare must stop the build");
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(
        message.contains("references the bare type name `Shared`")
            && message.contains("declared by"),
        "the refusal must name the leaf and its declaring packages: {message}"
    );
}
