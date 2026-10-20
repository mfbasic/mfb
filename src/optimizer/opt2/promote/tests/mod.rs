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

/// The `blocks × slots` bit-matrix dataflow `read_before_written` used
/// before bug-860, kept as the oracle the per-slot search is held to.
fn read_before_written_dense(
    instructions: &[CodeInstruction],
    accesses: &BTreeMap<usize, Vec<usize>>,
) -> Vec<usize> {
    let slots: Vec<usize> = accesses.keys().copied().collect();
    let words = slots.len().div_ceil(64);
    let mut slot_at: std::collections::HashMap<usize, (usize, bool)> =
        std::collections::HashMap::new();
    for (k, sites) in accesses.values().enumerate() {
        for &index in sites {
            slot_at.insert(index, (k, instructions[index].op == CodeOp::LdrU64));
        }
    }
    let blocks = crate::codegen::engine::regalloc::analysis::build_cfg(instructions);
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
        .filter(|&k| live_in[0][k / 64] & (1u64 << (k % 64)) != 0)
        .map(|k| slots[k])
        .collect()
}

/// Every full-word frame access in `stream`, by slot offset, as `promote`
/// collects them.
fn word_accesses(stream: &[CodeInstruction]) -> BTreeMap<usize, Vec<usize>> {
    let mut accesses: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..stream.len() {
        if let Touch::Word(offset) = touch(stream, index) {
            accesses.entry(offset).or_default().push(index);
        }
    }
    accesses
}

/// bug-860: the per-slot search answers exactly what the dense dataflow did,
/// over random control flow — forward and backward branches, loops, blocks
/// nothing reaches, slots stored on one arm only.
#[test]
fn the_per_slot_search_agrees_with_the_dense_dataflow() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move |bound: u64| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) % bound
    };
    let mut read_first_cases = 0;
    for _ in 0..400 {
        let block_count = 1 + next(12) as usize;
        let slot_count = 1 + next(70);
        let mut stream = Vec::new();
        for block in 0..block_count {
            stream.push(ci("label", &[("name", &format!("L{block}"))]));
            for _ in 0..next(5) {
                let offset = (8 * next(slot_count)).to_string();
                stream.push(if next(2) == 0 {
                    store("%v1", &offset)
                } else {
                    load("%v2", &offset)
                });
            }
            let target = format!("L{}", next(block_count as u64));
            match next(4) {
                0 => stream.push(ci("b", &[("target", &target)])),
                1 => stream.push(ci("b.eq", &[("target", &target)])),
                2 => stream.push(ci("ret", &[])),
                _ => {}
            }
        }
        let accesses = word_accesses(&stream);
        let expected = read_before_written_dense(&stream, &accesses);
        assert_eq!(read_before_written(&stream, &accesses), expected);
        read_first_cases += usize::from(!expected.is_empty());
    }
    assert!(
        read_first_cases > 50,
        "the random streams must exercise both answers, got {read_first_cases} of 400"
    );
}

/// bug-860: the search for one slot visits only the blocks that slot is live
/// into, so the work is linear in a stream of statement-local slots however
/// many blocks and slots it has. The dense dataflow this replaced kept three
/// `blocks × slots` bit matrices: 4 GB for 100 inline `TRAP` handlers in one
/// function, 18 GB for 150.
#[test]
fn the_search_visits_only_the_blocks_a_slot_is_live_into() {
    for statements in [100usize, 1_000, 10_000] {
        // Each statement: a block that stores a temporary, a conditional
        // branch around a second block, and a third block that reloads it.
        let mut stream = Vec::new();
        for k in 0..statements {
            let offset = (8 * k).to_string();
            stream.push(ci("label", &[("name", &format!("s{k}"))]));
            stream.push(store("%v1", &offset));
            stream.push(ci("b.eq", &[("target", &format!("j{k}"))]));
            stream.push(ci("label", &[("name", &format!("m{k}"))]));
            stream.push(ci("mov", &[("dst", "%v3"), ("src", "%v1")]));
            stream.push(ci("label", &[("name", &format!("j{k}"))]));
            stream.push(load("%v2", &offset));
        }
        stream.push(ci("ret", &[]));
        let accesses = word_accesses(&stream);
        assert_eq!(accesses.len(), statements);
        let blocks = crate::codegen::engine::regalloc::analysis::build_cfg(&stream);
        assert_eq!(blocks.len(), 3 * statements);
        let mut search = EntryLiveness::new(&blocks);
        for sites in accesses.values() {
            assert!(!search.live_into_entry(&stream, sites));
        }
        // Live into the reloading block and the skipped block; the storing
        // block kills it. Two blocks per slot, at every size.
        assert_eq!(search.visited, 2 * statements as u64);
    }
}

/// Off at `-O0`.
#[test]
fn the_row_is_off_at_level_zero() {
    let mut stream = vec![store("%v1", "8"), load("%v2", "8")];
    assert_eq!(run(&mut stream, &[0, 8], 16, &[], 0), 0);
    assert_eq!(stream[0].op, CodeOp::StrU64);
}
