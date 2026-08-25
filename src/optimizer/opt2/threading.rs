//! Jump threading — a Level-3 Opt2 catalog row
//! (`planning/optimizations.md`): redirect a branch that targets a trivial
//! trampoline block (a label followed immediately by an unconditional `b`)
//! straight to the trampoline's own target, collapsing jump-to-jump chains.
//!
//! Trivially behavior-preserving: a label emits no bytes and the trampoline's
//! `b` executes nothing observable, so landing at the final target directly
//! is the identical computation minus a hop. Chains resolve transitively
//! with a cycle guard (`L: b L` — an intentional infinite loop — is left
//! alone, as is any cycle of trampolines). Only *terminator* `target` fields
//! are rewritten: a label name referenced from any other field (an
//! address-style use this pass has never heard of) keeps its meaning, and
//! the trampoline block itself is left in place — once nothing references
//! it, unreachable-block pruning (`opt2::uce`) and block merging
//! (`opt2::merge`) clean it up.
//!
//! This is the hop-collapsing core of the catalog row; the profitable
//! duplicate-the-block form (threading a *conditional* branch through a
//! block whose condition is correlated) is a separate, larger transform that
//! can extend this row later.

use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::regalloc::analysis::is_block_terminator;
use crate::codegen::engine::types::CodeInstruction;

/// Run the jump-threading row over one function's selected stream, in place.
/// Self-guarded on the row's catalog level (3).
pub(crate) fn thread_jumps(instructions: &mut [CodeInstruction]) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    // Trampolines: `label L` immediately followed by `b M`. (A label is a
    // block leader and `b` a terminator, so the pair *is* the whole block.)
    let mut hop: HashMap<String, String> = HashMap::new();
    for window in instructions.windows(2) {
        if window[0].op == CodeOp::Label && window[1].op == CodeOp::Branch {
            if let (Some(label), Some(target)) = (window[0].get("name"), window[1].get("target")) {
                hop.insert(label, target);
            }
        }
    }
    if hop.is_empty() {
        crate::optimizer::stats::count_jumps_threaded(0);
        return;
    }

    // Resolve a chain to its final label, or `None` when it ends where it
    // started (nothing to do) or runs into a cycle (an intentional spin —
    // leave every edge of it alone).
    let resolve = |start: &str| -> Option<String> {
        let mut seen: Vec<&str> = vec![start];
        let mut current = start;
        while let Some(next) = hop.get(current) {
            if seen.iter().any(|s| *s == next) {
                return None;
            }
            seen.push(next);
            current = next;
        }
        (current != start).then(|| current.to_string())
    };

    let mut threaded = 0;
    for instruction in instructions.iter_mut() {
        if !is_block_terminator(instruction.op) {
            continue;
        }
        for (name, value) in instruction.fields.iter_mut() {
            if *name != "target" {
                continue;
            }
            let target = value.rendered().into_owned();
            if let Some(final_target) = resolve(&target) {
                *value = Operand::from(final_target);
                threaded += 1;
            }
        }
    }
    crate::optimizer::stats::count_jumps_threaded(threaded);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimizer::{with_opt_level, OptLevel};

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.field(k, v);
        }
        inst
    }

    fn run(stream: &mut [CodeInstruction], level: u8) {
        with_opt_level(OptLevel(level), || thread_jumps(stream));
    }

    /// A conditional branch into a two-hop trampoline chain lands on the
    /// final target; the trampolines' own `b`s collapse too.
    #[test]
    fn chains_collapse_to_the_final_target() {
        let mut stream = vec![
            ci("b.eq", &[("target", "hop1")]),
            ci("str_u64", &[("src", "x0"), ("base", "sp"), ("offset", "8")]),
            ci("ret", &[]),
            ci("label", &[("name", "hop1")]),
            ci("b", &[("target", "hop2")]),
            ci("label", &[("name", "hop2")]),
            ci("b", &[("target", "end")]),
            ci("label", &[("name", "end")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[0].get("target").as_deref(), Some("end"));
        assert_eq!(stream[4].get("target").as_deref(), Some("end"));
    }

    /// A self-loop (`spin: b spin`) and a two-block trampoline cycle are
    /// intentional infinite loops: nothing is rewritten into or out of them.
    #[test]
    fn cycles_are_left_alone() {
        let mut stream = vec![
            ci("b.eq", &[("target", "spin")]),
            ci("ret", &[]),
            ci("label", &[("name", "spin")]),
            ci("b", &[("target", "spin2")]),
            ci("label", &[("name", "spin2")]),
            ci("b", &[("target", "spin")]),
        ];
        run(&mut stream, 3);
        // Every edge into or inside the cycle is untouched.
        assert_eq!(stream[0].get("target").as_deref(), Some("spin"));
        assert_eq!(stream[3].get("target").as_deref(), Some("spin2"));
        assert_eq!(stream[5].get("target").as_deref(), Some("spin"));
    }

    /// A non-terminator field naming the label is not a jump and is never
    /// rewritten.
    #[test]
    fn non_terminator_references_are_untouched() {
        let mut stream = vec![
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "hop")],
            ),
            ci("ret", &[]),
            ci("label", &[("name", "hop")]),
            ci("b", &[("target", "end")]),
            ci("label", &[("name", "end")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[0].get("value").as_deref(), Some("hop"));
    }

    /// The row is off at `-O2`.
    #[test]
    fn level_two_disables_the_row() {
        let mut stream = vec![
            ci("b.eq", &[("target", "hop")]),
            ci("ret", &[]),
            ci("label", &[("name", "hop")]),
            ci("b", &[("target", "end")]),
            ci("label", &[("name", "end")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[0].get("target").as_deref(), Some("hop"));
    }
}
