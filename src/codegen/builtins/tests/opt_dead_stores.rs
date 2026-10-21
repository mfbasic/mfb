//! Dead-store elimination (`optimizer::opt2::dse`) — the full row (a store
//! no path reads before it is overwritten goes) and its Level-3 partial half
//! (a store dead on exactly one edge out of a branch sinks into the other edge,
//! when the edge's block has no other way in and the path to it neither
//! touches memory nor redefines the stored register).
//!
//! These cover what the row's own tests do not: a full 64-slot universe, the
//! partial sink in both edge orders (into a labelled block it lands after the
//! label), each refusal of the sink (a memory-touching instruction on the way,
//! the stored register redefined, a physical-register store, a store with no
//! value operand, a store already fully dead), and a non-numeric offset, which
//! forgets every slot rather than guess.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::CodeInstruction;
use crate::optimizer::{with_opt_level, OptLevel};

fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    let mut instruction = CodeInstruction::new(op);
    for (name, value) in fields {
        instruction = instruction.physical_field(name, *value);
    }
    instruction
}

fn store(src: &'static str, offset: &'static str) -> CodeInstruction {
    ci(
        "str_u64",
        &[("src", src), ("base", "sp"), ("offset", offset)],
    )
}

fn load(dst: &'static str, offset: &'static str) -> CodeInstruction {
    ci(
        "ldr_u64",
        &[("dst", dst), ("base", "sp"), ("offset", offset)],
    )
}

fn ops(stream: &[CodeInstruction]) -> Vec<CodeOp> {
    stream.iter().map(|instruction| instruction.op).collect()
}

fn run(stream: &mut Vec<CodeInstruction>) {
    let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    with_opt_level(OptLevel(3), || {
        crate::optimizer::opt2::dse::eliminate(stream, &model)
    });
}

#[test]
fn an_empty_stream_is_left_empty() {
    let mut stream = Vec::new();
    run(&mut stream);
    assert!(stream.is_empty());
}

/// Sixty-four slots — a whole word of the slot bitset — each written twice
/// with no read between: every first store is dead.
#[test]
fn a_full_word_of_overwritten_slots_loses_its_first_stores() {
    let offsets: Vec<String> = (0..64).map(|slot| (slot * 8).to_string()).collect();
    let write = |src: &'static str| -> Vec<CodeInstruction> {
        offsets
            .iter()
            .map(|offset| {
                CodeInstruction::new("str_u64")
                    .physical_field("src", src)
                    .physical_field("base", "sp")
                    .physical_field("offset", offset.as_str())
            })
            .collect()
    };
    let mut stream = write("%v1");
    stream.extend(write("%v2"));
    stream.push(ci("ret", &[]));
    run(&mut stream);
    assert_eq!(stream.len(), 65, "the 64 overwritten stores are gone");
    assert!(stream[..64]
        .iter()
        .all(|i| i.op == CodeOp::StrU64 && i.get("src").as_deref() == Some("%v2")));
}

/// `prefix` (ending in the store under test), then a compare and a branch to
/// `other`; the fall-through arm and the `other` arm follow.
fn branch(
    prefix: Vec<CodeInstruction>,
    fallthrough: Vec<CodeInstruction>,
    other: Vec<CodeInstruction>,
) -> Vec<CodeInstruction> {
    let mut stream = prefix;
    stream.push(ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]));
    stream.push(ci("b.eq", &[("target", "other")]));
    stream.extend(fallthrough);
    stream.push(ci("label", &[("name", "other")]));
    stream.extend(other);
    stream
}

#[test]
fn a_store_dead_on_one_edge_sinks_into_the_other() {
    // Read on the fall-through, overwritten on `other`.
    let mut stream = branch(
        vec![store("%v1", "8")],
        vec![load("%v2", "8"), store("%v2", "16"), ci("ret", &[])],
        vec![store("%v3", "8"), ci("ret", &[])],
    );
    run(&mut stream);
    assert_eq!(
        ops(&stream)[..3],
        [CodeOp::CmpImm, CodeOp::BranchEq, CodeOp::StrU64],
        "the store now opens the fall-through"
    );
    assert_eq!(stream[2].get("src").as_deref(), Some("%v1"));

    // Overwritten on the fall-through, read on `other`: it lands after the
    // label.
    let mut stream = branch(
        vec![store("%v1", "8")],
        vec![store("%v3", "8"), ci("ret", &[])],
        vec![load("%v2", "8"), store("%v2", "16"), ci("ret", &[])],
    );
    run(&mut stream);
    let label = stream
        .iter()
        .position(|i| i.op == CodeOp::Label)
        .expect("the label");
    assert_eq!(stream[label + 1].op, CodeOp::StrU64);
    assert_eq!(stream[label + 1].get("src").as_deref(), Some("%v1"));
    assert_eq!(stream[0].op, CodeOp::CmpImm, "it left the branching block");
}

/// Builds one instruction stream; `CodeInstruction` is not `Clone`.
type Shape = fn() -> Vec<CodeInstruction>;

#[test]
fn a_store_whose_path_is_not_clear_stays_put() {
    let fallthrough = || vec![load("%v2", "8"), store("%v2", "16"), ci("ret", &[])];
    let other = || vec![store("%v3", "8"), ci("ret", &[])];
    let cases: Vec<(&str, Shape)> = vec![
        ("a memory-touching call on the way", || {
            vec![store("%v1", "8"), ci("bl", &[("target", "_mfb_rt_helper")])]
        }),
        ("the stored register redefined", || {
            vec![
                store("%v1", "8"),
                ci(
                    "mov_imm",
                    &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
                ),
            ]
        }),
        ("a physical-register store", || vec![store("x3", "8")]),
        ("a store with no value operand", || {
            vec![ci("str_u64", &[("base", "sp"), ("offset", "8")])]
        }),
    ];
    for (what, prefix) in cases {
        let shape = || branch(prefix(), fallthrough(), other());
        let mut stream = shape();
        run(&mut stream);
        assert_eq!(ops(&stream), ops(&shape()), "{what}: nothing moves");
    }
}

#[test]
fn a_store_already_fully_dead_is_removed_not_sunk() {
    let mut stream = branch(
        vec![store("%v1", "8"), store("%v4", "8")],
        vec![load("%v2", "8"), store("%v2", "16"), ci("ret", &[])],
        vec![store("%v3", "8"), ci("ret", &[])],
    );
    run(&mut stream);
    let sources: Vec<String> = stream
        .iter()
        .filter(|i| i.op == CodeOp::StrU64)
        .filter_map(|i| i.get("src"))
        .collect();
    assert!(
        !sources.iter().any(|src| src == "%v1"),
        "the overwritten store goes: {sources:?}"
    );
}

#[test]
fn a_non_numeric_offset_forgets_every_slot() {
    // Without the unknown accesses the first store would be dead.
    for unknown in [store("%v5", "%v4"), load("%v6", "%v4")] {
        let mut stream = vec![
            store("%v1", "8"),
            unknown,
            store("%v2", "8"),
            ci("ret", &[]),
        ];
        run(&mut stream);
        assert_eq!(
            stream[0].get("src").as_deref(),
            Some("%v1"),
            "an access of unknown address may read the slot"
        );
    }
}
