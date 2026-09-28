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
/// (`builder_owned_cleanup.rs`, `error_constants.rs`: "`x0` is the slot's
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
fn read_before_written(
    instructions: &[CodeInstruction],
    accesses: &BTreeMap<usize, Vec<usize>>,
) -> Vec<usize> {
    if accesses.is_empty() {
        return Vec::new();
    }
    let slots: Vec<usize> = accesses.keys().copied().collect();
    let words = slots.len().div_ceil(64);
    // Per instruction: which candidate it loads or stores, if any.
    let mut slot_at: std::collections::HashMap<usize, (usize, bool)> =
        std::collections::HashMap::new();
    for (k, sites) in accesses.values().enumerate() {
        for &index in sites {
            slot_at.insert(index, (k, instructions[index].op == CodeOp::LdrU64));
        }
    }
    let blocks = crate::codegen::engine::regalloc::analysis::build_cfg(instructions);
    // Per block: `gen` = loaded before any store in the block; `kill` = stored.
    let mut gen = vec![vec![0u64; words]; blocks.len()];
    let mut kill = vec![vec![0u64; words]; blocks.len()];
    for (b, block) in blocks.iter().enumerate() {
        for index in (block.start..block.end).rev() {
            if let Some(&(k, is_load)) = slot_at.get(&index) {
                let (word, bit) = (k / 64, 1u64 << (k % 64));
                if is_load {
                    gen[b][word] |= bit;
                } else {
                    gen[b][word] &= !bit;
                    kill[b][word] |= bit;
                }
            }
        }
    }
    let mut live_in = vec![vec![0u64; words]; blocks.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..blocks.len()).rev() {
            let mut out = vec![0u64; words];
            for &s in &blocks[b].succ {
                for w in 0..words {
                    out[w] |= live_in[s][w];
                }
            }
            for w in 0..words {
                let value = gen[b][w] | (out[w] & !kill[b][w]);
                if value != live_in[b][w] {
                    live_in[b][w] = value;
                    changed = true;
                }
            }
        }
    }
    (0..slots.len())
        .filter(|&k| {
            live_in
                .first()
                .is_some_and(|entry| entry[k / 64] & (1u64 << (k % 64)) != 0)
        })
        .map(|k| slots[k])
        .collect()
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
            let integer = instruction
                .operand(value)
                .is_some_and(|operand| is_integer_register(operand));
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
mod tests {
    use super::*;
    use crate::optimizer::{with_opt_level, OptLevel};

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.physical_field(k, v);
        }
        inst
    }

    fn store(src: &str, offset: &str) -> CodeInstruction {
        ci(
            "str_u64",
            &[("src", src), ("base", "sp"), ("offset", offset)],
        )
    }

    fn load(dst: &str, offset: &str) -> CodeInstruction {
        ci(
            "ldr_u64",
            &[("dst", dst), ("base", "sp"), ("offset", offset)],
        )
    }

    /// Run the row at `level` over `stream` with frame objects at `starts`
    /// (frame size `size`) and the post-allocation zeroed slots `zeroed`.
    fn run(
        stream: &mut [CodeInstruction],
        starts: &[usize],
        size: usize,
        zeroed: &[usize],
        level: u8,
    ) -> u64 {
        with_opt_level(OptLevel(level), || {
            promote(
                stream,
                &FrameFacts {
                    object_starts: starts,
                    frame_size: size,
                    zeroed_after_allocation: zeroed,
                },
            )
        })
    }

    fn ops(stream: &[CodeInstruction]) -> Vec<CodeOp> {
        stream.iter().map(|instruction| instruction.op).collect()
    }

    /// The row's core: a slot only ever stored and reloaded whole becomes one
    /// register, and the load reads exactly the register the store wrote.
    #[test]
    fn a_private_slot_becomes_a_register() {
        let mut stream = vec![store("%v1", "8"), load("%v2", "8"), ci("ret", &[])];
        assert_eq!(run(&mut stream, &[0, 8], 16, &[], 1), 1);
        assert_eq!(ops(&stream), vec![CodeOp::Mov, CodeOp::Mov, CodeOp::Ret]);
        let home = stream[0].get("dst").unwrap();
        assert_eq!(stream[0].get("src").as_deref(), Some("%v1"));
        assert_eq!(stream[1].get("src").as_deref(), Some(home.as_str()));
        assert_eq!(stream[1].get("dst").as_deref(), Some("%v2"));
    }

    /// The zero register is a token only AArch64 has as a register; a zero
    /// store becomes a zero materialization.
    #[test]
    fn a_zero_store_becomes_an_immediate() {
        let mut stream = vec![store("xzr", "0"), load("%v2", "0"), ci("ret", &[])];
        assert_eq!(run(&mut stream, &[0], 8, &[], 1), 1);
        assert_eq!(stream[0].op, CodeOp::MovImm);
        assert_eq!(stream[0].get("value").as_deref(), Some("0"));
    }

    /// A slot some path reads before writing keeps its memory, so even that
    /// read observes what it always did.
    #[test]
    fn a_slot_read_before_written_stays_in_memory() {
        let mut stream = vec![
            ci("label", &[("name", "entry")]),
            ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
            ci("b.eq", &[("target", "skip")]),
            store("%v1", "0"),
            ci("label", &[("name", "skip")]),
            load("%v2", "0"),
            ci("ret", &[]),
        ];
        let before = ops(&stream);
        assert_eq!(run(&mut stream, &[0], 8, &[], 1), 0);
        assert_eq!(ops(&stream), before);
    }

    /// The owned drops take a slot's address and touch that one word: the slot
    /// stays in memory, every other slot is still promoted.
    #[test]
    fn a_slot_handed_to_a_drop_helper_is_pinned() {
        let mut stream = vec![
            store("%v1", "0"),
            store("%v3", "8"),
            load("%v4", "8"),
            ci("add_imm", &[("dst", "x0"), ("src", "sp"), ("imm", "0")]),
            ci(
                "bl",
                &[(
                    "target",
                    crate::codegen::error::constants::DROP_OWNED_STRING_SYMBOL,
                )],
            ),
            ci("ret", &[]),
        ];
        assert_eq!(run(&mut stream, &[0, 8], 16, &[], 1), 1);
        assert_eq!(stream[0].op, CodeOp::StrU64, "the dropped slot stays");
        assert_eq!(stream[1].op, CodeOp::Mov);
        assert_eq!(stream[2].op, CodeOp::Mov);
    }

    /// Any other frame address — to an arbitrary call, into a virtual register,
    /// or read by something before the call — turns the row off for the whole
    /// function: nothing says how far past the object that pointer reaches.
    #[test]
    fn any_other_frame_address_disables_the_function() {
        let cases: Vec<Vec<CodeInstruction>> = vec![
            vec![
                ci("add_imm", &[("dst", "x0"), ("src", "sp"), ("imm", "0")]),
                ci("bl", &[("target", "_mfb_rt_something_else")]),
            ],
            vec![ci(
                "add_imm",
                &[("dst", "%v7"), ("src", "sp"), ("imm", "0")],
            )],
            vec![
                ci("add_imm", &[("dst", "x0"), ("src", "sp"), ("imm", "0")]),
                ci("mov", &[("dst", "%v7"), ("src", "x0")]),
                ci(
                    "bl",
                    &[(
                        "target",
                        crate::codegen::error::constants::DROP_OWNED_STRING_SYMBOL,
                    )],
                ),
            ],
            vec![ci("mov", &[("dst", "%v7"), ("src", "sp")])],
        ];
        for prefix in cases {
            let mut stream = prefix;
            stream.extend([store("%v3", "8"), load("%v4", "8"), ci("ret", &[])]);
            let before = ops(&stream);
            assert_eq!(run(&mut stream, &[0, 8], 16, &[], 1), 0);
            assert_eq!(ops(&stream), before);
        }
    }

    /// An access outside every registered object is a frame layout this row
    /// does not understand: the function is left alone.
    #[test]
    fn an_access_outside_every_object_disables_the_function() {
        let mut stream = vec![store("%v1", "0"), load("%v2", "0"), store("%v3", "64")];
        assert_eq!(run(&mut stream, &[0], 8, &[], 1), 0);
        assert_eq!(stream[0].op, CodeOp::StrU64);
    }

    /// A narrower (or FP) access pins the object it lands in; a slot the
    /// prologue zeroes after allocation is pinned; a two-word object is not a
    /// word slot.
    #[test]
    fn pinned_and_wide_objects_stay_in_memory() {
        let mut narrow = vec![
            store("%v1", "0"),
            ci("str_u8", &[("src", "%v5"), ("base", "sp"), ("offset", "3")]),
            load("%v2", "0"),
        ];
        assert_eq!(run(&mut narrow, &[0], 8, &[], 1), 0);

        let mut zeroed = vec![store("%v1", "0"), load("%v2", "0")];
        assert_eq!(run(&mut zeroed, &[0], 8, &[0], 1), 0);

        let mut wide = vec![store("%v1", "0"), load("%v2", "0")];
        assert_eq!(run(&mut wide, &[0, 16], 24, &[], 1), 0);
    }

    /// Off at `-O0`.
    #[test]
    fn the_row_is_off_at_level_zero() {
        let mut stream = vec![store("%v1", "8"), load("%v2", "8")];
        assert_eq!(run(&mut stream, &[0, 8], 16, &[], 0), 0);
        assert_eq!(stream[0].op, CodeOp::StrU64);
    }
}
