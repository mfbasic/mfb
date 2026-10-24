//! plan-26: an inline `TRAP` is legal on EVERY inline-lowered built-in.
//!
//! An inline-lowered built-in has no standalone symbol, so an inline `TRAP` on one
//! must take either its raw-`Result` lowering (`lower_inline_builtin_raw`) or the
//! always-`Ok` wrap of an infallible one (`lower_inline_infallible_raw`); the generic
//! raw-call path would emit `bl <target>` to a symbol that does not exist.
//! `inline_trap_unsupported` names a built-in with neither. The set is empty, and
//! this sweep holds it empty over the whole registry, every argument shape the
//! arg-type-dependent members (`replace`, `toInt`) distinguish included — so a
//! member added without a raw or infallible lowering fails here, at test time,
//! instead of at a codegen backstop no program can reach.

use crate::codegen::builtins::{inline_trap_unsupported, native_builtin_target, NoTypeKinds};
use crate::codegen::registry::registry;
use crate::types::ParameterType;

#[test]
fn every_inline_builtin_has_a_raw_or_infallible_inline_trap_lowering() {
    let mut targets: Vec<String> = vec!["len".into(), "toString".into(), "typeName".into()];
    for package in registry().packages() {
        for function in package.functions() {
            targets.push(format!("{}.{}", package.import_name(), function.name));
        }
    }
    for member in ["find", "mid", "replace"] {
        targets.push(format!("strings.{member}"));
        targets.push(format!("collections.{member}"));
    }
    let list = ParameterType::list_of(ParameterType::Integer);
    let shapes: Vec<Vec<ParameterType>> = vec![
        Vec::new(),
        vec![ParameterType::Unknown],
        vec![ParameterType::Unknown; 3],
        vec![ParameterType::String; 3],
        vec![list.clone(), ParameterType::Integer, ParameterType::Integer],
        vec![list.clone(), list.clone(), list],
        vec![ParameterType::declared("Color")],
    ];
    let mut inline = 0usize;
    for target in &targets {
        if native_builtin_target(target).is_none()
            && !matches!(target.as_str(), "len" | "toString" | "typeName")
        {
            continue;
        }
        inline += 1;
        for shape in &shapes {
            assert!(
                !inline_trap_unsupported(target, shape, &NoTypeKinds),
                "an inline TRAP on `{target}` with arguments {shape:?} has no raw or \
                 infallible lowering"
            );
        }
    }
    // The floor keeps the sweep from passing over nothing.
    assert!(inline > 50, "only {inline} inline built-ins swept");
}
