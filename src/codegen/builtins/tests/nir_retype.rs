//! Shared harness for the "retyped function" refusal suites.
//!
//! NIR carries a type on every parameter and binding but almost none on its values:
//! a `Local` takes its type from the binding that declared it. So re-declaring a
//! function's parameters and bindings is a CONSISTENT retyping of that function —
//! the program the front end would have produced had the source said those types —
//! and the backend sees no contradiction it could catch by a local check. What it
//! is handed is exactly a shape the type checker stops (an incomparable element
//! under `replace`, a `transform` callback that is not a function, an index that is
//! not an `Integer`), and the contract these suites pin is that the backend refuses
//! such a module with a sentence naming the problem rather than emitting code for a
//! type it does not handle.

use super::nir_fault::walk_ops_mut;
use crate::target::shared::nir::{NirFunction, NirOp};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_nir, nir_for_src, CodeTarget};
use crate::types::ParameterType;

/// Lower `src` for linux-x86_64 after `mutate` has rewritten the function named
/// `func`; the backend's verdict. The unmutated program must lower.
pub(super) fn lowered_with(
    src: &str,
    func: &str,
    mutate: impl Fn(&mut NirFunction),
) -> Result<(), String> {
    let target = CodeTarget::LinuxX86_64;
    let mut module = nir_for_src(src, target, Console).expect("the probe program lowers to NIR");
    let mut found = false;
    for function in module.functions.iter_mut().filter(|f| f.name == func) {
        mutate(function);
        found = true;
    }
    assert!(found, "the probe program has no function `{func}`");
    code_for_nir(&module, target).map(|_| ())
}

/// Re-declare the named parameters and bindings of `func` (every `Bind`, at any
/// depth) with the given types.
pub(super) fn retype_locals(function: &mut NirFunction, retype: &[(&str, ParameterType)]) {
    let lookup = |name: &str| retype.iter().find(|(n, _)| *n == name).map(|(_, t)| t);
    for param in &mut function.params {
        if let Some(type_) = lookup(&param.name) {
            param.type_ = type_.clone();
        }
    }
    walk_ops_mut(
        &mut function.body,
        &mut |op| {
            if let NirOp::Bind { name, type_, .. } = op {
                if let Some(new) = lookup(name) {
                    *type_ = new.clone();
                }
            }
        },
        &mut |_| {},
    );
}

/// [`lowered_with`] + [`retype_locals`].
pub(super) fn retyped(
    src: &str,
    func: &str,
    retype: &[(&str, ParameterType)],
) -> Result<(), String> {
    lowered_with(src, func, |function| retype_locals(function, retype))
}

/// The refusal of a mutated module; panics if it lowered.
pub(super) fn refusal(result: Result<(), String>, what: &str) -> String {
    match result {
        Err(err) => err,
        Ok(()) => panic!("{what}: the backend must refuse this module, not lower it"),
    }
}

/// `List OF <element>`, spelled.
pub(super) fn list(element: &str) -> ParameterType {
    ParameterType::list_of(ParameterType::parse(element))
}
