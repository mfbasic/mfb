use crate::ir::IrProject;
use crate::target::NativeBuildMode;
use std::path::PathBuf;

use super::nir::{self, NirModule};
use super::runtime;

/// Every [`nir::NirOp`] in the module, counted at every nesting level.
///
/// Only walked when `-vv` is on: the visit is linear in the tree, but it is a
/// second full traversal of every function body and buys nothing for a build
/// that is not being profiled.
fn recursive_op_count(module: &NirModule) -> u64 {
    if !crate::trace::enabled() {
        return 0;
    }
    // The one authoritative recursion (bug-328): overriding `visit_op` and
    // delegating to `walk_op` inherits complete traversal, so a new `NirOp`
    // variant cannot silently go uncounted here.
    struct Counter {
        ops: u64,
    }
    impl nir::visit::NirVisitor for Counter {
        fn visit_op(&mut self, op: &nir::NirOp) {
            self.ops += 1;
            nir::visit::walk_op(self, op);
        }
    }
    let mut counter = Counter { ops: 0 };
    for function in &module.functions {
        nir::visit::NirVisitor::visit_ops(&mut counter, &function.body);
    }
    counter.ops
}

pub fn lower_project(
    ir: &IrProject,
    target_name: String,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
    // plan-15 D3: stdin broadcast-log backpressure cap from the manifest `"config"`
    // section, or `None` to bake the default (used by every non-executable / dump path).
    stdin_log_cap: Option<u64>,
) -> Result<NirModule, String> {
    // Merge imported packages' Binary Representation into the project up front so runtime
    // helper detection and codegen both see the complete, unified function set.
    //
    // The `-vv` spans (`crate::trace`) split the `codegen: lowering module`
    // stage into its four steps. This is the sole `NirModule` producer, so one
    // set of spans here covers all five targets.
    let merged = crate::trace::timed("merge packages", || nir::merge_packages(ir, packages))?;
    let helpers = crate::trace::timed("required helpers", || runtime::required_helpers(&merged));
    let module = crate::trace::timed("IR -> NIR", || {
        nir::lower_module(&merged, target_name, build_mode, stdin_log_cap, helpers)
    })?;
    crate::trace::count("NIR functions", module.functions.len() as u64);
    crate::trace::count(
        "NIR statements",
        module
            .functions
            .iter()
            .map(|function| function.body.len() as u64)
            .sum(),
    );
    // plan-118-A: the counter above is FLAT — it sums `body.len()`, so a loop
    // whose body holds fifty statements counts as one, and a whole nested
    // `IF`/`MATCH` tree as one per top-level arm. That makes it useless as the
    // denominator of an "instructions emitted per NIR op" expansion ratio: it
    // undercounts by ~1.8x over the acceptance corpus, inflating the ratio by
    // the same factor. This counter walks every op at every nesting level and is
    // the honest denominator. Both are kept: the flat one is what
    // `planning/speed.md`'s historical numbers mean, and a renamed counter would
    // silently re-point them.
    crate::trace::count("NIR ops (recursive)", recursive_op_count(&module));
    // plan-100 §3: the Opt1 seam. This is the sole `NirModule` producer, so one
    // wrap here covers all five targets. Occupied by the Level-1 local-rewrite
    // rows — constant folding, algebraic simplification, strength reduction
    // (`optimizer::opt1`).
    Ok(crate::trace::timed("opt1 (NIR)", || {
        crate::optimizer::opt1::optimize_nir(module, crate::optimizer::active_opt_level())
    }))
}
