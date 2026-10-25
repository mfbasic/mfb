//! What a function of a lowered program contains, as sets of names — the audit
//! trail the engine's lowering routes leave: the calls it makes, its label stems
//! (serial dropped) and the kinds of stack object it reserves. Shared by the
//! suites that assert WHICH route a source construct took rather than that it
//! lowered.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::tests::test_support::Stream;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_for_src_lowered, code_function, CodeTarget};

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

/// The kinds of stack object function `name` of `source` reserves on `target`,
/// sorted and deduplicated. A route that reserves scratch under its own name
/// (`inline_append_newrec`, `record_field_replaced`, ...) leaves that name here.
pub(super) fn slots_on(source: &str, name: &str, target: CodeTarget) -> Vec<String> {
    let plan = code_for_src_cached(source, target, Console);
    let mut kinds: Vec<String> = code_function(plan, name)
        .stack_slots
        .iter()
        .map(|slot| slot.type_.clone())
        .collect();
    kinds.sort();
    kinds.dedup();
    kinds
}

/// The label stems (serial dropped) of function `name` of `source` on `target`,
/// sorted and deduplicated — read at `-O0`, where every route's labels are as
/// lowering named them (see [`code_for_src_lowered`]).
pub(super) fn labels_on(source: &str, name: &str, target: CodeTarget) -> Vec<String> {
    let plan = code_for_src_lowered(source, target, Console);
    let mut stems: Vec<String> = Stream::of(code_function(plan, name))
        .labels()
        .into_iter()
        .map(|(_, label)| {
            label
                .trim_end_matches(|c: char| c.is_ascii_digit())
                .trim_end_matches('_')
                .to_string()
        })
        .collect();
    stems.sort();
    stems.dedup();
    stems
}
