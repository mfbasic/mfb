//! What a function of a lowered program contains, as sets of names — the audit
//! trail the engine's lowering routes leave: the calls it makes, its label stems
//! (serial dropped) and the kinds of stack object it reserves. Shared by the
//! suites that assert WHICH route a source construct took rather than that it
//! lowered.

use crate::arch::ops::CodeOp;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

/// The targets of the calls function `name` of `source` makes on `target`, in order.
pub(super) fn calls_on(source: &str, name: &str, target: CodeTarget) -> Vec<String> {
    let plan = code_for_src_cached(source, target, Console);
    code_function(plan, name)
        .instructions
        .iter()
        .filter(|i| i.op == CodeOp::BranchLink)
        .filter_map(|i| i.get("target"))
        .collect()
}
