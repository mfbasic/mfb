//! `unify.rs`: how the call-site unifier treats the type shapes the descriptors rarely
//! spell — user generics, callable and thread types inside other types, `RES`-wrapped
//! patterns — and what the self-update census (`self_update_shaped`) does with a
//! signature whose parameter and return can or cannot be the same type.
//!
//! Selection tests (`selection_and_thread_probes.rs`) resolve real members; the
//! members use `List`, `Map` and scalar patterns, so these arms of the matcher are
//! only reached by a pattern written for the purpose. Each case states the answer a
//! call site would get: a user generic binds its arguments positionally and only
//! against the same template at the same arity; a callable's parameters and result
//! unify under the isolation widening rule; a variable cannot be bound to a type
//! that mentions itself.

use super::*;
use ParameterType::{Boolean, Float, Integer, String};

fn bindings() -> BTreeMap<Symbol, ParameterType> {
    BTreeMap::new()
}

fn user(name: &str, args: Vec<ParameterType>) -> ParameterType {
    ParameterType::user_of(name, args)
}

fn implementation(params: Vec<ParameterType>, return_type: ParameterType) -> Implementation {
    Implementation {
        params: params
            .into_iter()
            .enumerate()
            .map(|(index, ty)| Parameter {
                name: ["a", "b", "c"][index],
                desc: "",
                aliases: &[],
                ty,
                default: DefaultValue::None,
            })
            .collect(),
        return_type,
        errors: Vec::new(),
        body: Body::Intrinsic,
    }
}

/// A user generic unifies head-then-arguments: the same template at the same arity,
/// each argument recursively, so `Stack OF T` binds `T` from `Stack OF Integer`.
#[test]
fn a_user_generic_unifies_by_template_and_arguments() {
    let pattern = user("Stack", vec![ParameterType::var("T")]);
    let mut bound = bindings();
    assert!(unify(
        &pattern,
        &user("Stack", vec![Integer]),
        &mut bound,
        true
    ));
    assert_eq!(
        substitute(&pattern, &bound),
        Some(user("Stack", vec![Integer])),
        "the argument the concrete carried is what the pattern substitutes back"
    );
    // A different template, a different arity, a different argument once bound.
    assert!(!unify(
        &pattern,
        &user("Queue", vec![Integer]),
        &mut bindings(),
        true
    ));
    assert!(!unify(
        &pattern,
        &user("Stack", vec![Integer, String]),
        &mut bindings(),
        true
    ));
    let pair = user(
        "Pair",
        vec![ParameterType::var("T"), ParameterType::var("T")],
    );
    assert!(unify(
        &pair,
        &user("Pair", vec![Integer, Integer]),
        &mut bindings(),
        true
    ));
    assert!(
        !unify(
            &pair,
            &user("Pair", vec![Integer, String]),
            &mut bindings(),
            true
        ),
        "one variable cannot be both Integer and String"
    );
}

/// A callable's parameters and result unify structurally, and a plain pattern accepts
/// an isolated callable while an isolated pattern accepts only an isolated one.
#[test]
fn a_callable_unifies_by_parameters_result_and_isolation() {
    let var = ParameterType::var("T");
    let plain = ParameterType::func(vec![var.clone()], var.clone());
    let mut bound = bindings();
    assert!(unify(
        &plain,
        &ParameterType::func(vec![Integer], Integer),
        &mut bound,
        true
    ));
    assert_eq!(
        substitute(&plain, &bound),
        Some(ParameterType::func(vec![Integer], Integer))
    );
    assert!(
        !unify(
            &plain,
            &ParameterType::func(vec![Integer], String),
            &mut bindings(),
            true
        ),
        "the variable is bound to Integer by the parameter and cannot become String"
    );
    assert!(
        !unify(
            &plain,
            &ParameterType::func(vec![Integer, Integer], Integer),
            &mut bindings(),
            true
        ),
        "arity differs"
    );
    // Isolation widens in one direction only.
    let isolated = ParameterType::func_isolated(vec![Integer], Integer);
    let plain_concrete = ParameterType::func(vec![Integer], Integer);
    assert!(unify(&plain_concrete, &isolated, &mut bindings(), true));
    assert!(!unify(&isolated, &plain_concrete, &mut bindings(), true));
    assert_eq!(
        substitute(&ParameterType::func_isolated(vec![var.clone()], var), &{
            let mut b = bindings();
            b.insert(Symbol::intern("T"), Integer);
            b
        }),
        Some(ParameterType::func_isolated(vec![Integer], Integer))
    );
}

/// A `List`'s element inside a callable parameter is an invariant position: nothing
/// converts there, so a union cannot be widened through it.
#[test]
fn a_list_inside_a_callable_is_an_invariant_position() {
    let pattern = ParameterType::func(
        vec![ParameterType::list_of(ParameterType::var("T"))],
        ParameterType::var("U"),
    );
    let mut bound = bindings();
    assert!(unify(
        &pattern,
        &ParameterType::func(vec![ParameterType::list_of(Integer)], Boolean),
        &mut bound,
        true
    ));
    assert_eq!(bound.get(&Symbol::intern("T")), Some(&Integer));
    assert_eq!(bound.get(&Symbol::intern("U")), Some(&Boolean));
}

/// A `RES`-wrapped pattern substitutes through its inner type.
#[test]
fn a_res_wrapped_pattern_substitutes_through() {
    let mut bound = bindings();
    bound.insert(Symbol::intern("T"), ParameterType::declared("fs.File"));
    assert_eq!(
        substitute(&ParameterType::res(ParameterType::var("T")), &bound),
        Some(ParameterType::res(ParameterType::declared("fs.File")))
    );
    // And an unbound variable substitutes to nothing, so the call is refused.
    assert_eq!(
        substitute(&ParameterType::var("Missing"), &bindings()),
        None
    );
}

/// Union-variant conversion looks through `RES` on either side.
#[test]
fn union_conversion_looks_through_res() {
    struct Shapes;
    impl TypeKinds for Shapes {
        fn is_enum(&self, _t: &ParameterType) -> bool {
            false
        }
        fn union_accepts(&self, union: &ParameterType, actual: &ParameterType) -> bool {
            union.is_named("Shape") && actual.is_named("Square")
        }
    }
    let square = ParameterType::named("Square");
    let shape = ParameterType::named("Shape");
    assert!(converts_to_union_form(&Shapes, &square, &shape));
    assert!(converts_to_union_form(
        &Shapes,
        &square,
        &ParameterType::res(shape.clone())
    ));
    assert!(converts_to_union_form(
        &Shapes,
        &ParameterType::res(square.clone()),
        &shape
    ));
    assert!(
        !converts_to_union_form(&Shapes, &shape, &square),
        "never the reverse"
    );
    assert!(
        !converts_to_union_form(&Shapes, &square, &square),
        "never for equal types"
    );
}

/// The self-update census: a signature is self-update shaped when its first parameter
/// can be made equal to its return by one substitution of the type variables.
#[test]
fn the_self_update_census_follows_one_substitution() {
    let t = ParameterType::var("T");
    let list_t = ParameterType::list_of(t.clone());
    // `transform(List OF T, ..) AS List OF U`: U can be T.
    assert!(self_update_shaped(&implementation(
        vec![list_t.clone()],
        ParameterType::list_of(ParameterType::var("U"))
    )));
    // A Map, a Set, a String self-update.
    assert!(self_update_shaped(&implementation(
        vec![ParameterType::map_of(String, t.clone())],
        ParameterType::map_of(String, ParameterType::var("V"))
    )));
    assert!(self_update_shaped(&implementation(
        vec![ParameterType::set_of(t.clone())],
        ParameterType::set_of(t.clone())
    )));
    assert!(self_update_shaped(&implementation(vec![String], String)));
    // An `Arg(0)` return echoes the first parameter; an `Arg` out of range is refused.
    assert!(self_update_shaped(&implementation(
        vec![list_t.clone()],
        ParameterType::Arg(0)
    )));
    assert!(!self_update_shaped(&implementation(
        vec![list_t.clone()],
        ParameterType::Arg(3)
    )));
    // No parameters, or a scalar first parameter, is not a self-update.
    assert!(!self_update_shaped(&implementation(vec![], Integer)));
    assert!(!self_update_shaped(&implementation(vec![Integer], Integer)));
    assert!(!self_update_shaped(&implementation(
        vec![list_t.clone()],
        Integer
    )));
    assert!(!self_update_shaped(&implementation(
        vec![list_t.clone()],
        ParameterType::map_of(String, Integer)
    )));
    // A variable cannot equal a type that mentions itself, through any wrapper.
    for wrapper in [
        ParameterType::list_of(t.clone()),
        ParameterType::set_of(t.clone()),
        ParameterType::result_of(t.clone()),
        ParameterType::map_of(Integer, t.clone()),
        ParameterType::map_entry_of(t.clone(), Integer),
        user("Stack", vec![t.clone()]),
        ParameterType::func(vec![t.clone()], Integer),
        ParameterType::func(vec![Integer], t.clone()),
    ] {
        assert!(
            !self_update_shaped(&implementation(vec![t.clone()], wrapper.clone())),
            "T cannot be {}",
            wrapper.name()
        );
    }
    // Two sides built from the same shape coincide through every wrapper.
    for shape in [
        user("Stack", vec![t.clone()]),
        ParameterType::func(vec![t.clone()], Float),
        ParameterType::func_isolated(vec![Integer], t.clone()),
        ParameterType::result_of(t.clone()),
        ParameterType::map_entry_of(String, t.clone()),
    ] {
        // Not a collection or String, so not self-update shaped — but the walk must
        // get as far as comparing the two sides without refusing them.
        assert!(
            !self_update_shaped(&implementation(vec![shape.clone()], shape.clone())),
            "{}",
            shape.name()
        );
    }
    assert!(
        !self_update_shaped(&implementation(
            vec![user("Stack", vec![t.clone()])],
            user("Queue", vec![t.clone()])
        )),
        "different templates never coincide"
    );
    assert!(
        !self_update_shaped(&implementation(
            vec![ParameterType::func(vec![Integer], t.clone())],
            ParameterType::func_isolated(vec![Integer], t.clone())
        )),
        "isolation must agree"
    );
}

/// `contains_var` looks inside user generics.
#[test]
fn contains_var_looks_inside_user_generics_and_callables() {
    assert!(contains_var(&user("Stack", vec![ParameterType::var("T")])));
    assert!(!contains_var(&user("Stack", vec![Integer])));
    assert!(contains_var(&ParameterType::func(
        vec![ParameterType::var("T")],
        Integer
    )));
}

/// `RES` is transparent to leaf matching on both sides: the marker never changes
/// whether two leaves match.
#[test]
fn res_is_transparent_to_leaf_matching() {
    let file = ParameterType::declared("fs.File");
    for strict in [false, true] {
        assert!(leaf_matches(
            &file,
            &ParameterType::res(file.clone()),
            strict
        ));
        assert!(leaf_matches(
            &ParameterType::res(file.clone()),
            &file,
            strict
        ));
        assert!(!leaf_matches(&Integer, &ParameterType::res(String), strict));
    }
}
