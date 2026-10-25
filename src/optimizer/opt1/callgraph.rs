//! Interprocedural / global DCE (call-graph) — the Level-1 catalog row of that
//! name (`planning/optimizations.md`): remove every function no root reaches,
//! unreachable cycles (mutually recursive orphans) included.
//!
//! Removing a function that never runs cannot change a value, a trap or an
//! ordering: its code never executes, so nothing it could raise is observable.
//! MFBASIC has no stack traces and function values are not comparable, so the
//! missing code is invisible to the program. The one thing the walk must not
//! do is miss a caller, and the roots are therefore conservative:
//!
//! - the program entry ([`NirModule::entry`]);
//! - the global initializer function (`__mfb_init_globals_*`, every package's
//!   bindings merged into it) — it runs at startup and is never removed — and
//!   every function any global's initializer value mentions;
//! - every natively-called builtin helper
//!   ([`natively_called_helpers`](crate::codegen::registry::natively_called_helpers)):
//!   a `crypto` lowering branches to its bare `_mfb_ifn_crypto_…` symbol, not
//!   through a NIR call;
//! - every function of the `canvas` package (`#canvas_*`, or a merged
//!   package's `<id>.<pkg>.#canvas_*` copy) the program carries. The
//!   graphics-thread trampoline enters `#canvas_renderLoop` by symbol
//!   (`runtime::canvas::render_loop_symbol`), and the hand-written app backends
//!   (the macOS/GTK/Windows window callbacks, the Metal and Vulkan presenters,
//!   the `--debug` render profile) consume the rest by symbol and gate their own
//!   emission — data objects such as `_mfb_rt_canvas_graphics`, platform imports
//!   such as `atoi` or `IsWindowVisible`, the `canvas::signalRedraw` helper the
//!   window callbacks call — on which `canvas` calls the module makes. Those
//!   gates were only ever exercised with the whole package present, and a canvas
//!   program left with part of it (one calling `canvas::getSize` but never
//!   presenting) fails to build on every app backend. So the package stays
//!   whole; what it reaches is kept, and every other package's unused helpers
//!   still go.
//!
//! From the roots, the edges are [`plans::globals::reachable_from`]'s: a direct
//! call, any mention of a function as a value (`FunctionRef` / `Closure` — a
//! callback, a thread entry, a lambda), and a reached function's parameter
//! defaults. A function value only exists once the code that mentions it runs,
//! so a mention from reachable code keeps its function (a function that
//! escapes as a value is never removed), and a mention from a removed function
//! cannot create one.
//!
//! The declared runtime-helper groups are narrowed to what the survivors use
//! ([`prune_runtime_helpers`]). Everything else codegen derives per function — owned/destination variants,
//! closure descriptors, `construct.T` synthesis, string tables — is computed
//! from the surviving functions' bodies, so it stays consistent with the
//! removal by construction. A hidden by-symbol caller this list missed is not a
//! silent miscompile: the module still names the removed symbol, and the build
//! fails with `internal relocation target '…' is not defined`
//! (`NativeCodePlan::validate`) — or, for a NIR mention, the NIR validator's
//! "does not resolve".
//!
//! A module without an entry function (no program root to start from) is left
//! alone;
//! package builds never reach NIR at all (`.mfp` carries IR), so a package's
//! exports are never candidates.

use crate::target::shared::nir::NirModule;

use super::plans;

/// Apply the row to the whole module. Self-guarded on its catalog level (1):
/// `-O0` keeps every function. The removal count feeds `optimizer::stats`.
pub(crate) fn eliminate(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(1) {
        return;
    }
    let Some(roots) = roots(module) else {
        return;
    };
    let seeds = module
        .globals
        .iter()
        .filter_map(|global| global.value.as_ref());
    let live = plans::globals::reachable_from(module, roots, seeds);
    let before = module.functions.len();
    // `retain` keeps the survivors in their original order, so emission order
    // (visible in `.ncode`) stays a function of the module alone.
    module
        .functions
        .retain(|function| live.contains(&function.name));
    let removed = before - module.functions.len();
    if removed > 0 {
        prune_runtime_helpers(module);
    }
    crate::optimizer::stats::count_functions_eliminated(removed as u64);
}

/// Drop the runtime helpers only removed functions used.
///
/// `runtime::required_helpers` declared the set from the IR, before any
/// function was removed, and `validate_nir` requires the declared set to be
/// exactly what the NIR uses — a helper declared and unused fails the build
/// ("NIR declares unused runtime helper"). So the declared list is narrowed to
/// the validator's own used set, keeping its order. A walk error leaves the list
/// alone: the module is invalid either way, and `validate_nir` reports it.
fn prune_runtime_helpers(module: &mut NirModule) {
    if let Ok(used) = crate::target::shared::validate::module_used_runtime_helpers(module) {
        module
            .runtime_helpers
            .retain(|helper| used.contains(helper));
    }
}

/// The names reachability starts from, or `None` for a module with no entry —
/// or one whose entry names no function, which `validate_nir` rejects anyway
/// ("NIR entry point … does not resolve to a function"): with no program root there is nothing to measure
/// reachability from, and emptying the module would only bury that diagnostic.
fn roots(module: &NirModule) -> Option<Vec<String>> {
    let entry = module
        .entry
        .as_ref()
        .filter(|entry| module.functions.iter().any(|f| f.name == entry.name))?;
    let mut roots = vec![
        entry.name.clone(),
        crate::target::shared::nir::global_initializer_name(&module.project),
    ];
    roots.extend(crate::codegen::registry::natively_called_helpers());
    // The whole `canvas` package half, whenever the program carries it.
    let canvas = crate::internal_name::internalize("__canvas_");
    let qualified_canvas = format!(".{canvas}");
    roots.extend(
        module
            .functions
            .iter()
            .filter(|function| {
                function.name.starts_with(&canvas) || function.name.contains(&qualified_canvas)
            })
            .map(|function| function.name.clone()),
    );
    Some(roots)
}

#[cfg(test)]
mod tests;
