//! Postdominators and control dependence over the register allocator's CFG —
//! the fact base ADCE's "assume dead, prove live" marking needs to decide
//! which conditional branches any instruction depends on.
//!
//! Built on `regalloc::analysis::build_cfg`'s blocks (the compile-required CFG
//! the allocator's liveness already uses, so terminators/edges cannot drift
//! from the allocator's view of the same stream). Immediate postdominators
//! come from the Cooper–Harvey–Kennedy intersection algorithm run on the
//! reverse graph against a virtual exit that joins every no-successor block;
//! control dependence is then read off the postdominator tree: block `B` is
//! control-dependent on branch block `P` when `B` postdominates a successor
//! of `P` but not `P` itself (the classic Ferrante–Ottenstein–Warren walk
//! from each successor up to, and excluding, `ipdom(P)`).
//!
//! Returns `None` — "no facts, callers must not transform" — when any block
//! cannot reach an exit (an infinite loop, or an indirect/unmodeled edge left
//! a block outside the reverse-reachable set). ADCE simply skips such a
//! function.

use crate::codegen::engine::regalloc::analysis::Block;

/// Postdominance facts for one function's CFG.
pub(crate) struct PostDom {
    /// `ipdom[b]` = immediate postdominator block of `b`, or `usize::MAX` when
    /// `b`'s immediate postdominator is the virtual exit. ADCE consumes only
    /// `controllers` (a dead branch deletes outright — see `opt2::adce`), so
    /// outside this module the tree is read only by the unit tests below,
    /// which pin the algorithm through it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) ipdom: Vec<usize>,
    /// `controllers[b]` = indices of blocks whose *conditional terminator*
    /// decides whether `b` executes (control dependence, deduplicated).
    pub(crate) controllers: Vec<Vec<usize>>,
}

pub(crate) const VIRTUAL_EXIT: usize = usize::MAX;

/// Compute postdominators + control dependence, or `None` when the CFG has a
/// block that cannot reach an exit.
pub(crate) fn compute(blocks: &[Block]) -> Option<PostDom> {
    let n = blocks.len();
    if n == 0 {
        return None;
    }
    // Reverse graph: edges succ -> block. Exits attach to the virtual exit.
    let mut preds_of: Vec<Vec<usize>> = vec![Vec::new(); n]; // CFG predecessors
    for (b, block) in blocks.iter().enumerate() {
        for &s in &block.succ {
            preds_of[s].push(b);
        }
    }
    let exits: Vec<usize> = (0..n).filter(|&b| blocks[b].succ.is_empty()).collect();
    if exits.is_empty() {
        return None;
    }

    // Reverse post-order of the *reverse* graph from the virtual exit (i.e.
    // process blocks from the exits backwards). Iterative DFS.
    let mut order = Vec::with_capacity(n); // postorder of reverse graph
    let mut seen = vec![false; n];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for &exit in &exits {
        if seen[exit] {
            continue;
        }
        seen[exit] = true;
        stack.push((exit, 0));
        while let Some(&mut (b, ref mut next)) = stack.last_mut() {
            // The reverse graph's successors of `b` are `b`'s CFG predecessors.
            if *next < preds_of[b].len() {
                let p = preds_of[b][*next];
                *next += 1;
                if !seen[p] {
                    seen[p] = true;
                    stack.push((p, 0));
                }
            } else {
                order.push(b);
                stack.pop();
            }
        }
    }
    if seen.iter().any(|&s| !s) {
        // Some block cannot reach an exit: no postdominance facts.
        return None;
    }
    // Reverse postorder of the reverse graph.
    order.reverse();
    let mut rpo_index = vec![0usize; n];
    for (i, &b) in order.iter().enumerate() {
        rpo_index[b] = i;
    }

    // Cooper–Harvey–Kennedy on the reverse graph. `ipdom[b] = VIRTUAL_EXIT`
    // encodes the virtual exit; exits initialize to it.
    let mut ipdom: Vec<Option<usize>> = vec![None; n];
    for &exit in &exits {
        ipdom[exit] = Some(VIRTUAL_EXIT);
    }
    // Walk both up the postdominator tree until they meet; the virtual exit is
    // the root. Returns `None` when a chain reaches an unassigned node (can
    // happen mid-fixpoint on convoluted orders — the caller just skips that
    // contribution this round and the outer loop retries).
    let intersect = |ipdom: &[Option<usize>], a: usize, b: usize| -> Option<usize> {
        let (mut a, mut b) = (a, b);
        while a != b {
            if a == VIRTUAL_EXIT {
                b = ipdom[b]?;
                continue;
            }
            if b == VIRTUAL_EXIT {
                a = ipdom[a]?;
                continue;
            }
            // Higher rpo position = farther from the exit: walk it up.
            if rpo_index[a] > rpo_index[b] {
                a = ipdom[a]?;
            } else {
                b = ipdom[b]?;
            }
        }
        Some(a)
    };
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &order {
            if blocks[b].succ.is_empty() {
                continue; // exit: pinned to the virtual exit
            }
            // Fold the already-assigned successors through intersect.
            let mut new_ipdom: Option<usize> = None;
            for &s in &blocks[b].succ {
                if ipdom[s].is_none() {
                    continue;
                }
                new_ipdom = match new_ipdom {
                    None => Some(s),
                    Some(current) => match intersect(&ipdom, current, s) {
                        Some(met) => Some(met),
                        None => continue,
                    },
                };
            }
            let Some(new_ipdom) = new_ipdom else { continue };
            if ipdom[b] != Some(new_ipdom) {
                ipdom[b] = Some(new_ipdom);
                changed = true;
            }
        }
    }
    if ipdom.iter().any(|d| d.is_none()) {
        return None;
    }
    let ipdom: Vec<usize> = ipdom.into_iter().map(|d| d.expect("checked")).collect();

    // Control dependence: for each block P with >1 successor, walk from each
    // successor up the postdominator tree to (excluding) ipdom(P).
    let mut controllers: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (p, block) in blocks.iter().enumerate() {
        if block.succ.len() < 2 {
            continue;
        }
        for &s in &block.succ {
            let mut t = s;
            while t != ipdom[p] {
                if t == VIRTUAL_EXIT {
                    break;
                }
                if !controllers[t].contains(&p) {
                    controllers[t].push(p);
                }
                t = ipdom[t];
            }
        }
    }
    Some(PostDom { ipdom, controllers })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(start: usize, end: usize, succ: &[usize]) -> Block {
        Block {
            start,
            end,
            succ: succ.to_vec(),
        }
    }

    /// Diamond: 0 → {1, 2} → 3(exit). ipdom of every block is 3 (or the
    /// virtual exit for 3), and 1/2 are control-dependent on 0.
    #[test]
    fn diamond_control_dependence() {
        let blocks = vec![
            block(0, 2, &[1, 2]),
            block(2, 3, &[3]),
            block(3, 4, &[3]),
            block(4, 5, &[]),
        ];
        let pd = compute(&blocks).expect("facts");
        assert_eq!(pd.ipdom[0], 3);
        assert_eq!(pd.ipdom[1], 3);
        assert_eq!(pd.ipdom[2], 3);
        assert_eq!(pd.ipdom[3], VIRTUAL_EXIT);
        assert_eq!(pd.controllers[1], vec![0]);
        assert_eq!(pd.controllers[2], vec![0]);
        assert!(pd.controllers[0].is_empty());
        assert!(pd.controllers[3].is_empty());
    }

    /// Skip shape: 0 → {1(fallthrough), 2} , 1 → 2(exit). The skipped block 1
    /// is control-dependent on 0; ipdom(0) = 2.
    #[test]
    fn skip_shape() {
        let blocks = vec![block(0, 2, &[2, 1]), block(2, 3, &[2]), block(3, 4, &[])];
        let pd = compute(&blocks).expect("facts");
        assert_eq!(pd.ipdom[0], 2);
        assert_eq!(pd.controllers[1], vec![0]);
        assert!(pd.controllers[2].is_empty());
    }

    /// An infinite loop (no block reaches an exit) yields no facts.
    #[test]
    fn no_exit_means_no_facts() {
        let blocks = vec![block(0, 1, &[1]), block(1, 2, &[0])];
        assert!(compute(&blocks).is_none());
    }

    /// A loop with an exit: the latch and body are control-dependent on the
    /// loop header's conditional branch.
    #[test]
    fn loop_with_exit() {
        // 0: entry -> 1; 1: header -> {2 body, 3 exit}; 2: body -> 1; 3: exit.
        let blocks = vec![
            block(0, 1, &[1]),
            block(1, 2, &[2, 3]),
            block(2, 3, &[1]),
            block(3, 4, &[]),
        ];
        let pd = compute(&blocks).expect("facts");
        assert_eq!(pd.ipdom[1], 3);
        assert_eq!(pd.controllers[2], vec![1]);
        // The header itself re-executes based on its own branch.
        assert_eq!(pd.controllers[1], vec![1]);
    }
}
