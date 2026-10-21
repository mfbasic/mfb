//! Native plan storage classes: what size and class each NIR type plans to.
//!
//! Every parameter, return and local is planned with a storage class, and a
//! type with none is a build failure rather than a guess. These tests mutate a
//! lowered NIR module to carry the type shapes the front end produces for
//! resources and file-private types, and the corrupt ones it never should.

use super::plan_imports::{module_for, plan_of, HELLO};
use crate::target::shared::nir::{NirModule, NirParam, NirType};
use crate::target::shared::plan::StorageClass;
use crate::target::NativeBuildMode;
use crate::testutil::CodeTarget;
use crate::types::ParameterType;

const TARGET: CodeTarget = CodeTarget::LinuxX86_64;

fn hello() -> NirModule {
    module_for(HELLO, TARGET, NativeBuildMode::Console)
}

fn with_param(type_: ParameterType) -> NirModule {
    let mut module = hello();
    module.functions[0].params.push(NirParam {
        name: "extra".to_string(),
        type_,
        default: None,
    });
    module
}

fn planned_param(module: &NirModule) -> (&'static str, usize) {
    let plan = plan_of(module, TARGET).expect("the module plans");
    let main = plan
        .functions
        .iter()
        .find(|function| function.name == "main")
        .expect("main is planned");
    let storage = &main.params.last().expect("the extra parameter").storage;
    let class = match storage.class {
        StorageClass::Integer => "integer",
        StorageClass::Reference => "reference",
        _ => "other",
    };
    (class, storage.size)
}

/// A `RES`-marked collection element stores exactly like the bare type it marks.
#[test]
fn a_res_marked_element_plans_like_the_type_it_marks() {
    let marked = planned_param(&with_param(ParameterType::Res(Box::new(
        ParameterType::Integer,
    ))));
    let bare = planned_param(&with_param(ParameterType::Integer));
    assert_eq!(marked, bare);
    assert_eq!(marked, ("integer", 8));
}

/// A file-private type (`#<hash>$Name`) is a user type: one pointer to its record.
#[test]
fn a_file_private_type_name_plans_as_a_reference() {
    let private = ParameterType::named("#1a2b3c4d$Db");
    assert_eq!(planned_param(&with_param(private)), ("reference", 8));
}

/// A type the planner has no storage class for is refused, naming it.
#[test]
fn a_type_with_no_storage_class_is_refused_by_name() {
    for (type_, name) in [
        (ParameterType::Unknown, "Unknown"),
        (ParameterType::named("not a type"), "not a type"),
    ] {
        let error = plan_of(&with_param(type_), TARGET)
            .err()
            .expect("no storage class");
        assert_eq!(
            error,
            format!("native plan has no storage class for type '{name}'")
        );
    }
}

/// A NIR type declaration of a kind the planner does not know is refused.
#[test]
fn a_type_declaration_of_an_unknown_kind_is_refused() {
    let mut module = hello();
    module.types.push(NirType {
        kind: "bogus".to_string(),
        visibility: "public".to_string(),
        name: "Odd".to_string(),
        fields: Vec::new(),
        includes: Vec::new(),
        variants: Vec::new(),
        members: Vec::new(),
    });
    let error = plan_of(&module, TARGET)
        .err()
        .expect("an unknown type kind");
    assert_eq!(
        error,
        "native plan has no storage class for type kind 'bogus'"
    );
}

/// A module with no entry plans with no entry symbol, and that plan is valid.
#[test]
fn a_module_without_an_entry_plans_with_no_entry_symbol() {
    let mut module = hello();
    module.entry = None;
    let plan = plan_of(&module, TARGET).expect("an entry-less module plans");
    assert_eq!(plan.entry_symbol, None);
    plan.validate().expect("an entry-less plan is valid");
}
