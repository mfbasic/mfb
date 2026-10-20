//! Stack-slot promotion — the Level-1 "Stack slot promotion" row: turn a frame
//! slot that is only ever stored and reloaded as a whole word into a virtual
//! register, so the register allocator — not the lowering — decides whether the
//! value lives in a register or in memory.
//!
//! The lowering keeps every local, parameter and operand temporary in its own
//! 8-byte frame object and round-trips each value through it (`str` after the
//! producer, `ldr` before the consumer). For a slot nobody can reach except
//! through those two instructions, the slot *is* a register with a memory
//! address nobody uses: replacing `str src, [sp, #o]` by `mov %vO, src` and
//! `ldr dst, [sp, #o]` by `mov dst, %vO` computes the identical value on every
//! path, including paths that read the slot before writing it (the register
//! then holds whatever the frame word would have — nothing reads such a value
//! in a well-formed stream, and the prologue-zeroed slots are excluded, below).
//! The allocator colors `%vO` like any other virtual register: kept in a
//! register when one is free for its whole life (a callee-saved one across a
//! call), spilled to a slot of its own otherwise.
//!
//! That is **not** "never worse than the frame round-trip it replaces", which
//! this comment used to claim. Every promoted slot is a new long-lived value
//! competing for the register file, so promotion raises pressure on *other*
//! values. Under the first-come allocator that let promoted values take the
//! registers and pushed each hot loop temporary into a spill whose every use
//! evicted a live register around it — `nbody` 7.5× slower, -O1 inline `sort`
//! 6× (bug-760 C/D). What bounds the cost now is `linear_scan::run`'s spill
//! choice: under pressure the value with the least weighted use per instruction
//! of its interval goes to memory, which for a slot touched only a few times
//! across a long range is the promoted value itself, back to one load or store
//! per access (plus a copy the coalescer could not remove). That is a
//! heuristic, not a proof, so a function can still come out slower than
//! without promotion.
//!
//! What makes it exact is the **reachability proof**, per frame object, over
//! the whole function (the stream is scanned once, before any rewrite):
//!
//! - every instruction that names the stack pointer is classified; an
//!   instruction naming it in any way this module does not recognize disables
//!   promotion for the **whole function** — mis-modeling can only lose a
//!   rewrite, never make one;
//! - a full-word `ldr_u64`/`str_u64` at a numeric offset is an *access* of the
//!   object that holds `[offset, offset + 8)`; a narrower or wider `sp`-based
//!   load or store *pins* every object it could overlap;
//! - taking a frame address (`add_imm dst, sp, #k`) pins the object at `k`,
//!   and is only accepted in the one shape whose extent is known — the owned
//!   drop helpers' single-slot argument ([`SINGLE_SLOT_HELPERS`]), with the
//!   address consumed by the helper call and by nothing else. Any other address
//!   escape disables the function: the lowering has 568 frame-address sites
//!   (`grep -A3 'add_immediate(' src/codegen | grep -B1 -A2 'stack_pointer()' |
//!   grep -c add_immediate`), and a pointer into one object says nothing about
//!   how far past it the code behind the pointer reads;
//! - an access outside every registered object disables the function (a
//!   hand-built frame layout this module cannot reason about);
//! - the slots the prologue zeroes after allocation
//!   (`FrameFacts::zeroed_after_allocation`) are pinned: that store is inserted
//!   after coloring, so a promoted slot would miss it.
//!
//! An object is promoted when it is exactly one word, unpinned, and every
//! access to it is a full-word access at its start.

use std::collections::{BTreeMap, BTreeSet};

use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::regalloc::mint_vreg;
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::abi;
use crate::target::shared::regmodel::RegClass;

/// The runtime helpers whose first argument is the address of ONE frame word,
/// which they read and then zero (free-and-null) — the owned-value drops
/// (`builder_owned_cleanup/`, `error_constants.rs`: "`x0` is the slot's
/// ADDRESS"). Handing a frame address to one of these reaches that word and no
/// other.
pub(crate) const SINGLE_SLOT_HELPERS: &[&str] = &[
    crate::codegen::error::constants::DROP_OWNED_STRING_SYMBOL,
    crate::codegen::error::constants::DROP_OWNED_STRING_CAP_SYMBOL,
    crate::codegen::error::constants::DROP_OWNED_COLLECTION_SYMBOL,
];

/// The frame facts the lowering knows and the stream does not.
pub(crate) struct FrameFacts<'a> {
    /// Start offset of every registered frame object, in any order. An
    /// object's extent runs to the next distinct start (or to `frame_size`).
    pub(crate) object_starts: &'a [usize],
    /// The frame size the objects were carved from.
    pub(crate) frame_size: usize,
    /// Slots the function prologue zeroes after register allocation.
    pub(crate) zeroed_after_allocation: &'a [usize],
}

/// How one instruction touches the frame.
enum Touch {
    /// Names no stack pointer.
    None,
    /// A full-word load or store at this offset.
    Word(usize),
    /// A frame access of unknown width at this offset: pin what it overlaps.
    Pin(usize),
    /// A frame address that only a single-slot helper consumes.
    Address(usize),
    /// Anything else: promotion is off for the whole function.
    Unknown,
}

/// Promote every provably private one-word frame slot in `instructions` (a
/// pre-selection stream) to a fresh integer virtual register. Returns how many
/// objects were promoted.
pub(crate) fn promote(instructions: &mut [CodeInstruction], frame: &FrameFacts<'_>) -> u64 {
    if !crate::optimizer::level_enabled(1) {
        return 0;
    }
    let starts: BTreeSet<usize> = frame.object_starts.iter().copied().collect();
    if starts.is_empty() {
        return 0;
    }
    // The object holding byte `offset`: its start and its end.
    let object_of = |offset: usize| -> Option<(usize, usize)> {
        let start = *starts.range(..=offset).next_back()?;
        let end = starts
            .range(start + 1..)
            .next()
            .copied()
            .unwrap_or(frame.frame_size);
        (offset < end).then_some((start, end))
    };

    let mut pinned: BTreeSet<usize> = frame.zeroed_after_allocation.iter().copied().collect();
    // Object start -> the instructions accessing it as a whole word there.
    let mut accesses: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..instructions.len() {
        match touch(instructions, index) {
            Touch::None => {}
            Touch::Word(offset) => {
                let Some((start, end)) = object_of(offset) else {
                    return 0;
                };
                if offset != start || end - start != 8 {
                    pinned.insert(start);
                } else {
                    accesses.entry(start).or_default().push(index);
                }
            }
            Touch::Pin(offset) => {
                // Up to a 16-byte access (`ldr_q`/`str_q`) from `offset`.
                let Some((start, _)) = object_of(offset) else {
                    return 0;
                };
                pinned.insert(start);
                for other in starts.range(start + 1..offset + 16) {
                    pinned.insert(*other);
                }
            }
            Touch::Address(offset) => {
                let Some((start, _)) = object_of(offset) else {
                    return 0;
                };
                pinned.insert(start);
            }
            Touch::Unknown => return 0,
        }
    }

    accesses.retain(|start, _| !pinned.contains(start));
    // A slot some path reads before any store is not promoted: its first read
    // sees whatever the frame word held, and a register would hold something
    // else. Keeping it in memory keeps even that read exactly as it was.
    for start in read_before_written(instructions, &accesses) {
        accesses.remove(&start);
    }

    let mut promoted = 0;
    for (_, sites) in accesses {
        let home = Operand::from(mint_vreg(RegClass::Int));
        for index in sites {
            instructions[index] = rewrite(&instructions[index], &home);
        }
        promoted += 1;
    }
    promoted
}

/// The candidate slots (by start offset) that some path from the function
/// entry loads before storing: backward liveness over the allocator's own CFG,
/// with a load as a use and a store as a definition, read off at the entry
/// block.
///
/// bug-860: solved one slot at a time, never as a `blocks × slots` matrix. A
/// candidate is a statement-local temporary, live in a handful of blocks, while
/// a function whose every exit path repeats its cleanup has a block count and a
/// candidate count that both grow with the square of its statement count (100
/// inline `TRAP` handlers: 110,985 candidates), so three dense bit matrices were
/// 4 GB there and 18 GB at 150 handlers. Each slot's search walks only the
/// blocks the slot is live into, with two stamp arrays shared by every search,
/// so the memory is linear in the stream and the work is the size of the
/// liveness relation itself.
fn read_before_written(
    instructions: &[CodeInstruction],
    accesses: &BTreeMap<usize, Vec<usize>>,
) -> Vec<usize> {
    if accesses.is_empty() {
        return Vec::new();
    }
    let blocks = crate::codegen::engine::regalloc::analysis::build_cfg(instructions);
    let mut search = EntryLiveness::new(&blocks);
    accesses
        .iter()
        .filter(|(_, sites)| search.live_into_entry(instructions, sites))
        .map(|(&start, _)| start)
        .collect()
}

/// The per-slot backward liveness search of [`read_before_written`], and the
/// state every search shares.
struct EntryLiveness<'a> {
    blocks: &'a [crate::codegen::engine::regalloc::analysis::Block],
    /// Predecessor lists, flattened: block `b`'s are
    /// `preds[pred_start[b]..pred_start[b + 1]]`.
    pred_start: Vec<usize>,
    preds: Vec<u32>,
    /// `stores[b] == stamp`: block `b` stores the slot being searched.
    stores: Vec<u32>,
    /// `live_in[b] == stamp`: the slot being searched is live into block `b`.
    live_in: Vec<u32>,
    stamp: u32,
    worklist: Vec<u32>,
    /// Blocks marked live-in, over every search: the work the pass did, which
    /// `the_search_visits_only_the_blocks_a_slot_is_live_into` bounds.
    #[cfg(test)]
    visited: u64,
}

impl<'a> EntryLiveness<'a> {
    fn new(blocks: &'a [crate::codegen::engine::regalloc::analysis::Block]) -> Self {
        let mut pred_start = vec![0usize; blocks.len() + 1];
        for block in blocks {
            for &s in &block.succ {
                pred_start[s + 1] += 1;
            }
        }
        for b in 0..blocks.len() {
            pred_start[b + 1] += pred_start[b];
        }
        let mut fill = pred_start.clone();
        let mut preds = vec![0u32; pred_start[blocks.len()]];
        for (b, block) in blocks.iter().enumerate() {
            for &s in &block.succ {
                preds[fill[s]] = b as u32;
                fill[s] += 1;
            }
        }
        Self {
            blocks,
            pred_start,
            preds,
            stores: vec![0; blocks.len()],
            live_in: vec![0; blocks.len()],
            stamp: 0,
            worklist: Vec::new(),
            #[cfg(test)]
            visited: 0,
        }
    }

    /// Mark the searched slot live into `block` and queue the block.
    fn mark_live_in(&mut self, block: usize) {
        self.live_in[block] = self.stamp;
        self.worklist.push(block as u32);
        #[cfg(test)]
        {
            self.visited += 1;
        }
    }

    /// The block holding instruction `index`.
    fn block_of(&self, index: usize) -> usize {
        self.blocks.partition_point(|block| block.end <= index)
    }

    /// Whether the slot accessed at `sites` (ascending instruction indices, each
    /// a full-word load or store of it) is live into the entry block: the least
    /// fixed point of `in[b] = gen[b] ∪ (out[b] − kill[b])`, where a block
    /// generates the slot when its first access is a load and kills it when it
    /// stores it anywhere.
    fn live_into_entry(&mut self, instructions: &[CodeInstruction], sites: &[usize]) -> bool {
        self.stamp += 1;
        let stamp = self.stamp;
        self.worklist.clear();
        let mut previous_block = usize::MAX;
        for &index in sites {
            let block = self.block_of(index);
            let is_load = instructions[index].op == CodeOp::LdrU64;
            if block != previous_block && is_load && self.live_in[block] != stamp {
                // Upward-exposed: nothing in the block stores the slot first.
                self.mark_live_in(block);
            }
            if !is_load {
                self.stores[block] = stamp;
            }
            previous_block = block;
        }
        while let Some(block) = self.worklist.pop() {
            let block = block as usize;
            if block == 0 {
                return true;
            }
            for at in self.pred_start[block]..self.pred_start[block + 1] {
                let pred = self.preds[at] as usize;
                if self.stores[pred] != stamp && self.live_in[pred] != stamp {
                    self.mark_live_in(pred);
                }
            }
        }
        false
    }
}

/// The copy that replaces a full-word frame access to a promoted slot.
fn rewrite(instruction: &CodeInstruction, home: &Operand) -> CodeInstruction {
    let mut out = match instruction.op {
        CodeOp::StrU64 => {
            let src = instruction
                .operand("src")
                .cloned()
                .expect("a store names its source");
            if src.rendered() == abi::ZERO_SPELLING {
                // The zero register is a token, not a register every backend
                // has; materialize the zero instead of copying it.
                super::rebuilt(
                    CodeOp::MovImm,
                    vec![
                        ("dst", home.clone()),
                        ("type", Operand::from(abi::IMMEDIATE_CLASS_INTEGER)),
                        ("value", Operand::from("0")),
                    ],
                )
            } else {
                super::rebuilt(CodeOp::Mov, vec![("dst", home.clone()), ("src", src)])
            }
        }
        CodeOp::LdrU64 => {
            let dst = instruction
                .operand("dst")
                .cloned()
                .expect("a load names its destination");
            super::rebuilt(CodeOp::Mov, vec![("dst", dst), ("src", home.clone())])
        }
        other => unreachable!("only full-word frame accesses are rewritten, not {other:?}"),
    };
    out.source = instruction.source;
    out
}

fn names_sp(operand: &Operand) -> bool {
    operand.rendered() == abi::SP_SPELLING
}

fn numeric(instruction: &CodeInstruction, field: &str) -> Option<usize> {
    instruction.get(field)?.parse().ok()
}

/// Classify how `instructions[index]` touches the frame.
fn touch(instructions: &[CodeInstruction], index: usize) -> Touch {
    let instruction = &instructions[index];
    let mentions: Vec<&'static str> = instruction
        .fields
        .iter()
        .filter(|(_, operand)| names_sp(operand))
        .map(|(name, _)| *name)
        .collect();
    if mentions.is_empty() {
        return Touch::None;
    }
    let only_base = mentions == ["base"];
    match instruction.op {
        CodeOp::LdrU64 | CodeOp::StrU64 if only_base => {
            // The value operand must be an integer-class register or the zero
            // token; a word moved through an FP register is not a GPR value.
            let value = if instruction.op == CodeOp::LdrU64 {
                "dst"
            } else {
                "src"
            };
            let integer = instruction.operand(value).is_some_and(is_integer_register);
            match numeric(instruction, "offset") {
                Some(offset) if integer => Touch::Word(offset),
                Some(offset) => Touch::Pin(offset),
                None => Touch::Unknown,
            }
        }
        CodeOp::LdrU32
        | CodeOp::LdrU16
        | CodeOp::LdrU8
        | CodeOp::StrU32
        | CodeOp::StrU16
        | CodeOp::StrU8
        | CodeOp::LdrD
        | CodeOp::StrD
        | CodeOp::LdrQ
        | CodeOp::StrQ
            if only_base =>
        {
            match numeric(instruction, "offset") {
                Some(offset) => Touch::Pin(offset),
                None => Touch::Unknown,
            }
        }
        CodeOp::AddImm if mentions == ["src"] => match numeric(instruction, "imm") {
            Some(0) if sp_value_feeds_only_the_next_compare(instructions, index) => Touch::None,
            Some(offset) if address_feeds_only_a_single_slot_helper(instructions, index) => {
                Touch::Address(offset)
            }
            _ => Touch::Unknown,
        },
        _ => Touch::Unknown,
    }
}

/// An integer virtual register, or a token / physical name that is not an FP
/// register.
fn is_integer_register(operand: &Operand) -> bool {
    match operand {
        Operand::VReg { class, .. } => *class == RegClass::Int,
        _ => {
            let spelling = operand.rendered();
            crate::codegen::engine::regalloc::analysis::fp_physical_index(&spelling).is_none()
                && !spelling.starts_with("%f")
        }
    }
}

/// bug-789: `add_imm dst, sp, #0` whose `dst` is named only by the very next
/// instruction, a `cmp` — the stack check every recursing function opens with
/// (`CodeBuilder::emit_stack_check`). It reads the stack pointer's value; no frame
/// object's address leaves it, so it neither pins a slot nor blocks promotion.
fn sp_value_feeds_only_the_next_compare(instructions: &[CodeInstruction], index: usize) -> bool {
    let Some(dst) = instructions[index].operand("dst") else {
        return false;
    };
    let Some(next) = instructions.get(index + 1) else {
        return false;
    };
    if next.op != CodeOp::Cmp
        || !(next.operand("lhs") == Some(dst) || next.operand("rhs") == Some(dst))
    {
        return false;
    }
    instructions.iter().enumerate().all(|(other, instruction)| {
        other == index
            || other == index + 1
            || !instruction.fields.iter().any(|(_, operand)| operand == dst)
    })
}

/// Whether the frame address `add_imm dst, sp, #k` at `index` reaches nothing
/// but a [`SINGLE_SLOT_HELPERS`] call: straight-line to the first call, that
/// call is to one of those helpers, and no instruction in between reads or
/// writes `dst` (so the address is in `dst` at the call and is used by nothing
/// else; the call itself clobbers the caller-saved argument register it sits
/// in).
fn address_feeds_only_a_single_slot_helper(instructions: &[CodeInstruction], index: usize) -> bool {
    let Some(dst) = instructions[index].operand("dst") else {
        return false;
    };
    if matches!(dst, Operand::VReg { .. }) {
        // A virtual register could be kept past the call; only the argument
        // token the drop emitters use is accepted.
        return false;
    }
    let dst = dst.rendered();
    for instruction in &instructions[index + 1..] {
        match instruction.op {
            CodeOp::BranchLink => {
                return instruction
                    .get("target")
                    .is_some_and(|target| SINGLE_SLOT_HELPERS.contains(&target.as_str()));
            }
            op if op == CodeOp::Label
                || crate::codegen::engine::regalloc::analysis::is_block_terminator(op) =>
            {
                return false
            }
            _ => {}
        }
        if instruction
            .fields
            .iter()
            .any(|(_, operand)| operand.rendered() == dst)
        {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests;
