//! Natural loops over the MIR CFG — the fact base the Opt2 loop rows share.
//!
//! Opt1 has its own loop facts (`opt1::plans::loops`), but those describe the
//! *structured* NIR: a `FOR`/`WHILE` node with a body list. By the Opt2 seam
//! the program is a flat instruction stream with labels and branches, and the
//! loops in it are not only the ones the source wrote — desugaring, inlining
//! and the collection lowerings all emit their own. So the MIR half needs its
//! own, purely graph-theoretic answer.
//!
//! A **back edge** is an edge `n → h` where `h` dominates `n`; the loop it
//! defines is `h` plus every block that can reach `n` without going through
//! `h`. Two back edges to the same header describe one loop, so their bodies
//! are merged. Dominance comes from the SSA overlay, which already computes
//! it — there is no second dominator implementation here.
//!
//! A loop's **preheader** is the one predecessor of the header that lies
//! outside the loop *and* has the header as its only successor. That second
//! condition is what makes the preheader a safe place to put work: a block
//! that also branches elsewhere would run hoisted code on iterations of
//! nothing. When no predecessor qualifies the loop simply has no preheader,
//! and the rows that need one decline — this module never creates blocks.
//!
//! **Depth** is containment depth: a loop whose block set is a strict subset
//! of another's is nested inside it. The loop-nest code-motion row reads it
//! to hoist to the shallowest level that is still safe rather than only one
//! level out.

use crate::codegen::engine::regalloc::analysis::Block;

use super::ssa::Ssa;

/// One natural loop.
pub(crate) struct Loop {
    /// The block every path into the loop enters through.
    pub(crate) header: usize,
    /// Every block in the loop, sorted, including the header.
    pub(crate) blocks: Vec<usize>,
    /// The single outside predecessor that reaches only the header, if there
    /// is one. Rows that move code need it; rows that only read facts do not.
    pub(crate) preheader: Option<usize>,
    /// Containment depth: 0 for an outermost loop.
    pub(crate) depth: usize,
}

impl Loop {
    /// Whether `block` is inside this loop.
    pub(crate) fn contains(&self, block: usize) -> bool {
        self.blocks.binary_search(&block).is_ok()
    }
}

/// Find every natural loop in the CFG, outermost first (so a walk that hoists
/// progressively outward sees enclosing loops after the ones they contain).
pub(crate) fn find(blocks: &[Block], overlay: &Ssa) -> Vec<Loop> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); blocks.len()];
    for (b, block) in blocks.iter().enumerate() {
        for &successor in &block.succ {
            preds[successor].push(b);
        }
    }

    // Header -> body, merging the back edges that share a header.
    let mut bodies: Vec<(usize, Vec<bool>)> = Vec::new();
    for (n, block) in blocks.iter().enumerate() {
        for &header in &block.succ {
            if !overlay.dominates(header, n) {
                continue;
            }
            let slot = match bodies.iter().position(|(h, _)| *h == header) {
                Some(index) => index,
                None => {
                    bodies.push((header, vec![false; blocks.len()]));
                    bodies.len() - 1
                }
            };
            // Everything that reaches `n` without passing through `header`.
            let body = &mut bodies[slot].1;
            body[header] = true;
            let mut stack = vec![n];
            while let Some(current) = stack.pop() {
                if body[current] {
                    continue;
                }
                body[current] = true;
                for &pred in &preds[current] {
                    if !body[pred] {
                        stack.push(pred);
                    }
                }
            }
        }
    }

    let mut loops: Vec<Loop> = bodies
        .into_iter()
        .map(|(header, body)| {
            let members: Vec<usize> = body
                .iter()
                .enumerate()
                .filter_map(|(b, inside)| inside.then_some(b))
                .collect();
            let preheader = preheader_of(blocks, &preds, header, &members);
            Loop {
                header,
                blocks: members,
                preheader,
                depth: 0,
            }
        })
        .collect();

    // Containment depth: a loop nested in another has a strictly smaller body.
    let sizes: Vec<usize> = loops.iter().map(|l| l.blocks.len()).collect();
    let headers: Vec<usize> = loops.iter().map(|l| l.header).collect();
    for index in 0..loops.len() {
        let mut depth = 0;
        for other in 0..loops.len() {
            if other == index || sizes[other] <= sizes[index] {
                continue;
            }
            if loops[other].contains(headers[index]) {
                depth += 1;
            }
        }
        loops[index].depth = depth;
    }
    loops.sort_by_key(|l| (l.depth, l.header));
    loops
}

/// The loop's preheader: the one predecessor of `header` outside the loop
/// whose only successor is the header.
fn preheader_of(
    blocks: &[Block],
    preds: &[Vec<usize>],
    header: usize,
    members: &[usize],
) -> Option<usize> {
    let outside: Vec<usize> = preds[header]
        .iter()
        .copied()
        .filter(|pred| members.binary_search(pred).is_err())
        .collect();
    let &only = outside.first()?;
    if outside.len() != 1 || blocks[only].succ.len() != 1 {
        return None;
    }
    Some(only)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::engine::regalloc::analysis::build_cfg;
    use crate::codegen::engine::regalloc::class_models;
    use crate::codegen::engine::types::CodeInstruction;

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.field(k, v);
        }
        inst
    }

    fn loops_for(stream: &[CodeInstruction]) -> (Vec<Loop>, Vec<Block>) {
        let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
        let models = class_models(&model);
        let blocks = build_cfg(stream);
        let overlay = super::super::ssa::build(stream, &blocks, &models);
        let found = find(&blocks, &overlay);
        (found, blocks)
    }

    /// A plain bottom-tested loop: one back edge, one header, and the block
    /// in front of it is the preheader.
    #[test]
    fn a_simple_loop_is_found_with_its_preheader() {
        let stream = vec![
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "0")],
            ),
            ci("b", &[("target", "head")]),
            ci("label", &[("name", "head")]),
            ci("add_imm", &[("dst", "%v1"), ("src", "%v1"), ("imm", "1")]),
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "10")]),
            ci("b.lt", &[("target", "head")]),
            ci("ret", &[]),
        ];
        let (found, _) = loops_for(&stream);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].blocks.len(), 1, "header and latch are one block");
        assert_eq!(found[0].preheader, Some(0));
        assert_eq!(found[0].depth, 0);
    }

    /// A nested loop is reported inside its parent, and depth reflects it.
    #[test]
    fn nesting_is_reported_by_depth() {
        let stream = vec![
            ci("b", &[("target", "outer")]),
            ci("label", &[("name", "outer")]),
            ci("b", &[("target", "inner")]),
            ci("label", &[("name", "inner")]),
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "10")]),
            ci("b.lt", &[("target", "inner")]),
            ci("cmp_imm", &[("lhs", "%v2"), ("rhs", "10")]),
            ci("b.lt", &[("target", "outer")]),
            ci("ret", &[]),
        ];
        let (found, _) = loops_for(&stream);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].depth, 0, "outermost first");
        assert_eq!(found[1].depth, 1);
        assert!(found[0].blocks.len() > found[1].blocks.len());
    }

    /// A straight-line function has no loops at all.
    #[test]
    fn straight_line_code_has_no_loops() {
        let stream = vec![
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "0")],
            ),
            ci("ret", &[]),
        ];
        let (found, _) = loops_for(&stream);
        assert!(found.is_empty());
    }

    /// A header reachable from two outside blocks has no preheader, and the
    /// module never invents one.
    #[test]
    fn two_entries_means_no_preheader() {
        let stream = vec![
            ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
            ci("b.eq", &[("target", "head")]),
            ci("b", &[("target", "head")]),
            ci("label", &[("name", "head")]),
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "10")]),
            ci("b.lt", &[("target", "head")]),
            ci("ret", &[]),
        ];
        let (found, _) = loops_for(&stream);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].preheader, None);
    }
}
