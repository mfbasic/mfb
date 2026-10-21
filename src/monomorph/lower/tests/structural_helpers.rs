//! The structural type walks of `monomorph::lower` at the shapes the program
//! corpora do not produce: the containers `types_compatible` recurses into,
//! the template view of a mangled instance nested in each container, a
//! template parameter spelled with a C ABI token, and the expected element type
//! of a `Set` literal.

use super::*;
use std::collections::HashMap;

fn mono_with_box(dir: &std::path::Path) -> (crate::hir::HirProject, std::path::PathBuf) {
    let ast = project(&[(
        "src/main.mfb",
        "TYPE Box OF T\n  value AS T\nEND TYPE\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
    )]);
    (crate::hir::elaborate(&ast), dir.to_path_buf())
}

fn t(spelling: &str) -> ParameterType {
    ParameterType::parse(spelling)
}

/// `Unknown` is a LEAF wildcard inside every container `types_compatible`
/// walks, and never stands for a whole container.
#[test]
fn types_compatible_recurses_through_every_container() {
    let (hir, dir) = mono_with_box(&std::env::temp_dir());
    let mono = Monomorphizer::new(&dir, &hir);
    let stateful = |base: &str, state: ParameterType| ParameterType::Stateful {
        base: Box::new(t(base)),
        state: Box::new(state),
    };
    let cases: &[(ParameterType, ParameterType, bool)] = &[
        (t("Set OF Integer"), t("Set OF Unknown"), true),
        (t("Set OF Unknown"), t("Set OF String"), true),
        (t("Set OF Integer"), t("Set OF String"), false),
        (t("Set OF List OF Integer"), t("Set OF Unknown"), false),
        (t("#Result OF Integer"), t("#Result OF Unknown"), true),
        (t("#Result OF Integer"), t("#Result OF String"), false),
        (
            t("MapEntry OF String TO Integer"),
            t("MapEntry OF Unknown TO Integer"),
            true,
        ),
        (
            t("MapEntry OF String TO Integer"),
            t("MapEntry OF String TO String"),
            false,
        ),
        (
            stateful("File", t("Integer")),
            stateful("File", ParameterType::Unknown),
            true,
        ),
        (
            stateful("File", t("Integer")),
            stateful("Socket", t("Integer")),
            false,
        ),
    ];
    for (param, actual, want) in cases {
        assert_eq!(
            mono.types_compatible(param, actual),
            *want,
            "types_compatible({}, {})",
            param.name(),
            actual.name()
        );
    }
}

/// A mangled instance reads back as its template spelling at any depth inside
/// a container — the spelling a diagnostic shows the author (bug-697).
#[test]
fn template_view_restores_an_instance_inside_every_container() {
    let (hir, dir) = mono_with_box(&std::env::temp_dir());
    let mut mono = Monomorphizer::new(&dir, &hir);
    let boxed = mono.instantiate_type("Box", &[ParameterType::Integer]);
    let view = ParameterType::user_of("Box", vec![ParameterType::Integer]);
    let pairs = [
        (
            ParameterType::set_of(boxed.clone()),
            ParameterType::set_of(view.clone()),
        ),
        (
            ParameterType::result_of(boxed.clone()),
            ParameterType::result_of(view.clone()),
        ),
        (
            ParameterType::map_of(ParameterType::String, boxed.clone()),
            ParameterType::map_of(ParameterType::String, view.clone()),
        ),
        (
            ParameterType::map_entry_of(boxed.clone(), ParameterType::Integer),
            ParameterType::map_entry_of(view.clone(), ParameterType::Integer),
        ),
        (
            ParameterType::thread_handle(
                false,
                boxed.clone(),
                ParameterType::Nothing,
                boxed.clone(),
            ),
            ParameterType::thread_handle(false, view.clone(), ParameterType::Nothing, view.clone()),
        ),
        (
            ParameterType::thread_handle(
                true,
                ParameterType::Integer,
                boxed.clone(),
                ParameterType::String,
            ),
            ParameterType::thread_handle(
                true,
                ParameterType::Integer,
                view.clone(),
                ParameterType::String,
            ),
        ),
    ];
    for (concrete, want) in pairs {
        assert_eq!(
            mono.template_view(&concrete),
            want,
            "template_view({})",
            concrete.name()
        );
    }
}

/// A template parameter spelled with a C ABI token (`TYPE Box OF CPtr`) is a
/// leaf like any other, in both the substitution walk and template binding.
#[test]
fn a_c_abi_spelled_template_parameter_substitutes() {
    let (hir, dir) = mono_with_box(&std::env::temp_dir());
    let mut mono = Monomorphizer::new(&dir, &hir);
    let c_param = t("CPtr");
    assert!(
        matches!(c_param, ParameterType::C(_)),
        "`CPtr` must parse as a C ABI type for this test to mean anything"
    );
    let substitutions = HashMap::from([(
        crate::intern::Symbol::intern("CPtr"),
        ParameterType::Integer,
    )]);
    assert_eq!(
        mono.concrete_type(&ParameterType::list_of(c_param.clone()), &substitutions),
        t("List OF Integer")
    );

    let params = std::collections::HashSet::from([crate::intern::Symbol::intern("CPtr")]);
    let mut bound = HashMap::new();
    assert!(crate::monomorph::helpers::unify_type(
        &ParameterType::list_of(c_param),
        &t("List OF String"),
        &params,
        &mut bound,
    ));
    assert_eq!(
        bound.get(&crate::intern::Symbol::intern("CPtr")),
        Some(&ParameterType::String)
    );
}

/// A substitution that maps a parameter to its own spelling is returned as it
/// is: walking it again would never terminate.
#[test]
fn a_self_naming_substitution_is_not_walked_again() {
    let (hir, dir) = mono_with_box(&std::env::temp_dir());
    let mut mono = Monomorphizer::new(&dir, &hir);
    let substitutions = HashMap::from([(
        crate::intern::Symbol::intern("T"),
        ParameterType::named("T"),
    )]);
    assert_eq!(
        mono.concrete_type(&ParameterType::named("T"), &substitutions),
        ParameterType::named("T")
    );
}

#[test]
fn a_set_literal_takes_its_element_type_only_from_a_set_context() {
    use super::super::types::expected_element_type;
    let set = t("Set OF Integer");
    let list = t("List OF String");
    assert_eq!(
        expected_element_type(Some(&set), true),
        Some(ParameterType::Integer)
    );
    assert_eq!(expected_element_type(Some(&list), true), None);
    assert_eq!(
        expected_element_type(Some(&list), false),
        Some(ParameterType::String)
    );
    assert_eq!(expected_element_type(Some(&set), false), None);
    assert_eq!(expected_element_type(None, true), None);
}

#[test]
fn an_empty_qualifier_strips_nothing() {
    assert_eq!(strip_qualifier_prefixes("io.File", ""), "io.File");
    assert_eq!(
        strip_qualifier_prefixes("List OF io.File", "io."),
        "List OF File"
    );
    assert_eq!(strip_qualifier_prefixes("radio.File", "io."), "radio.File");
}
