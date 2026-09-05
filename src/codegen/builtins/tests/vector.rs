//! Every `vector::` member's body selector fails CLOSED on an unknown type.
//!
//! Each of the nineteen members is an overload set over nine vector types
//! (`Float2/3/4`, `Fixed2/3/4`, `Integer2/3/4`), and each picks the MFBASIC body
//! to instantiate with a `match ty { "Float2" => …, … }` keyed on the type name
//! the monomorph target carries. The final arm is `unreachable!`, and it is the
//! only thing standing between a name the selector does not know and one of the
//! other nine bodies.
//!
//! A selector that failed OPEN here — returning, say, the `Float2` body for a
//! `Float4` value — would read and write two of the four lanes and leave the
//! rest as whatever was in the block. Nothing would report it: the program
//! compiles, links, runs, and computes a plausible wrong answer. That is the
//! failure mode `.ai/codegen-invariants.md` records for every type-keyed
//! selector in the tree, and the reason the arm is `unreachable!` rather than a
//! silent default.
//!
//! No program can reach the arm — the monomorph target is built from the
//! descriptor's own applicable types — so it is dead in coverage terms and rots.

use crate::codegen::builtins::vector;
use crate::codegen::registry::registry;

/// `(member, selector)` for every `vector::` overload set.
type Selector = fn(&str) -> &'static str;

fn selectors() -> Vec<(&'static str, Selector)> {
    vec![
        ("abs", vector::func_abs::body as Selector),
        ("angle", vector::func_angle::body),
        ("clamp_length", vector::func_clamp_length::body),
        ("cross", vector::func_cross::body),
        ("distance", vector::func_distance::body),
        ("dot", vector::func_dot::body),
        ("length", vector::func_length::body),
        ("lerp", vector::func_lerp::body),
        ("lerp_unclamped", vector::func_lerp_unclamped::body),
        ("max", vector::func_max::body),
        ("min", vector::func_min::body),
        ("normalize", vector::func_normalize::body),
        ("perpendicular", vector::func_perpendicular::body),
        ("project", vector::func_project::body),
        ("reflect", vector::func_reflect::body),
        ("reject", vector::func_reject::body),
        ("rotate_2d", vector::func_rotate_2d::body),
        ("scale", vector::func_scale::body),
        ("slerp", vector::func_slerp::body),
    ]
}

/// An unknown type name aborts rather than selecting some other type's body.
#[test]
fn every_vector_selector_refuses_a_type_it_does_not_know() {
    let mut accepted = Vec::new();
    for (member, select) in selectors() {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let outcome = std::panic::catch_unwind(|| select("Float5"));
        std::panic::set_hook(hook);
        if let Ok(body) = outcome {
            accepted.push(format!("vector::{member} -> {body:?}"));
        }
    }
    assert!(
        accepted.is_empty(),
        "{} vector selector(s) returned a body for a type they do not know, so a \
         value of one shape would be lowered with another shape's layout:\n  {}",
        accepted.len(),
        accepted.join("\n  ")
    );
}

/// ...and it maps every type its DESCRIPTOR declares to a body of its own.
///
/// The applicable types come from the registry rather than from a list here,
/// because they differ per member: a perpendicular is only defined in 2D and a
/// cross product only in 3D, so assuming all nine would be asserting something
/// false. Tying the sweep to the descriptor also makes it the real contract -
/// the selector must cover exactly what the package promises.
///
/// The other half of the failure the arm above guards: an arm that maps two type
/// names to one body compiles, is not `unreachable!`, and lowers `Float3` with
/// `Float2`'s arithmetic. Distinctness is what makes the match a dispatch rather
/// than a decoration.
#[test]
fn every_vector_selector_covers_exactly_the_types_its_descriptor_declares() {
    let package = registry()
        .resolve_package("vector")
        .expect("the vector package is registered");
    let mut checked = 0;
    for (member, select) in selectors() {
        let function = package
            .function(member)
            .unwrap_or_else(|| panic!("vector::{member} is not registered"));
        let mut declared: Vec<String> = function
            .implementations()
            .iter()
            .filter_map(|implementation| implementation.params.first())
            // The descriptor's type name is QUALIFIED (`vector.Fixed2`); the
            // selector is keyed on the bare name the monomorph target carries.
            // That difference is a real seam - a selector fed the qualified
            // spelling would match nothing and hit its `unreachable!`.
            .map(|param| {
                let name = param.ty.name();
                name.rsplit('.').next().unwrap_or(&name).to_string()
            })
            .collect();
        declared.sort();
        declared.dedup();
        assert!(
            !declared.is_empty(),
            "vector::{member} declares no overloads to select between"
        );
        let mut seen: Vec<(String, &'static str)> = Vec::new();
        for ty in declared {
            let hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let type_name = ty.clone();
            let outcome = std::panic::catch_unwind(move || select(&type_name));
            std::panic::set_hook(hook);
            let body = outcome.unwrap_or_else(|_| {
                panic!("vector::{member} has no body for `{ty}`, which its descriptor declares")
            });
            if let Some((other, _)) = seen.iter().find(|(_, b)| *b == body) {
                panic!(
                    "vector::{member} maps `{ty}` and `{other}` to the same body - \
                     one of them would be lowered with the other's lane count"
                );
            }
            seen.push((ty, body));
            checked += 1;
        }
    }
    assert!(
        checked >= 100,
        "the sweep checked only {checked} (member, type) pairs; there were more \
         than 100 when this was written"
    );
}
