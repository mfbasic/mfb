use super::*;
use crate::optimizer::{with_opt_level, OptLevel};

fn store(src: &str, base: &str, offset: i64) -> CodeInstruction {
    CodeInstruction::new("str_u64")
        .physical_field("src", src)
        .physical_field("base", base)
        .field("offset", offset.to_string().as_str())
}

fn load(dst: &str, base: &str, offset: i64) -> CodeInstruction {
    CodeInstruction::new("ldr_u64")
        .physical_field("dst", dst)
        .physical_field("base", base)
        .field("offset", offset.to_string().as_str())
}

fn label() -> CodeInstruction {
    CodeInstruction::new("label").field("name", "join")
}

/// One instruction as `op field=value …`, in field order.
fn show(instruction: &CodeInstruction) -> String {
    let mut text = instruction.op.mnemonic().to_string();
    for (name, value) in &instruction.fields {
        text.push_str(&format!(" {name}={}", value.rendered()));
    }
    text
}

fn paired(mut instructions: Vec<CodeInstruction>) -> (u64, Vec<String>) {
    let formed = pair_adjacent_accesses(&mut instructions);
    (formed, instructions.iter().map(show).collect())
}

fn one(instructions: Vec<CodeInstruction>) -> Option<String> {
    match paired(instructions) {
        (1, stream) if stream.len() == 1 => Some(stream[0].clone()),
        (0, _) => None,
        other => panic!("expected one pair or none: {other:?}"),
    }
}

#[test]
fn adjacent_stores_and_loads_pair_with_registers_ordered_by_address() {
    assert_eq!(
        one(vec![store("x1", "x9", 16), store("x2", "x9", 24)]).as_deref(),
        Some("stp src=x1 src2=x2 base=x9 offset=16 addressing=offset width=8")
    );
    // Reversed: the higher address first still pairs, lower register first.
    assert_eq!(
        one(vec![store("x1", "x9", 24), store("x2", "x9", 16)]).as_deref(),
        Some("stp src=x2 src2=x1 base=x9 offset=16 addressing=offset width=8")
    );
    assert_eq!(
        one(vec![load("x3", "sp", 40), load("x4", "sp", 32)]).as_deref(),
        Some("ldp dst=x4 dst2=x3 base=sp offset=32 addressing=offset width=8")
    );
    assert_eq!(
        one(vec![store("xzr", "sp", 0), store("xzr", "sp", 8)]).as_deref(),
        Some("stp src=xzr src2=xzr base=sp offset=0 addressing=offset width=8")
    );
}

#[test]
fn the_lower_offset_must_be_within_the_imm7_reach() {
    assert!(one(vec![store("x1", "x9", 504), store("x2", "x9", 512)]).is_some());
    assert!(one(vec![store("x1", "x9", 512), store("x2", "x9", 520)]).is_none());
    assert!(one(vec![load("x1", "x9", -512), load("x2", "x9", -504)]).is_some());
    assert!(one(vec![load("x1", "x9", -520), load("x2", "x9", -512)]).is_none());
    // 8 apart but not 8-aligned: no scaled encoding.
    assert!(one(vec![store("x1", "x9", 4), store("x2", "x9", 12)]).is_none());
}

#[test]
fn only_two_accesses_of_one_kind_through_one_base_8_apart_pair() {
    assert!(one(vec![store("x1", "x9", 0), store("x2", "x10", 8)]).is_none());
    assert!(one(vec![store("x1", "x9", 0), store("x2", "x9", 16)]).is_none());
    assert!(one(vec![store("x1", "x9", 0), store("x2", "x9", 0)]).is_none());
    assert!(one(vec![store("x1", "x9", 0), load("x2", "x9", 8)]).is_none());
    let narrow = CodeInstruction::new("str_u32")
        .physical_field("src", "x2")
        .physical_field("base", "x9")
        .field("offset", "8");
    assert!(one(vec![store("x1", "x9", 0), narrow]).is_none());
    // A stack-argument sentinel is not a register yet.
    assert!(one(vec![
        store("x1", abi::OUTGOING_ARGS_SPELLING, 0),
        store("x2", abi::OUTGOING_ARGS_SPELLING, 8)
    ])
    .is_none());
    // `sp` cannot be paired data: register 31 is XZR there.
    assert!(one(vec![store("sp", "x9", 0), store("x2", "x9", 8)]).is_none());
    // A field the pair would not carry keeps the access single.
    let tagged = store("x2", "x9", 8).field("type", "Integer");
    assert!(one(vec![store("x1", "x9", 0), tagged]).is_none());
}

#[test]
fn a_load_into_the_base_or_twice_into_one_register_never_pairs() {
    // The first load moves the base under the second.
    assert!(one(vec![load("x9", "x9", 0), load("x2", "x9", 8)]).is_none());
    // The second may write the base: both addresses are read first.
    assert_eq!(
        one(vec![load("x1", "x9", 0), load("x9", "x9", 8)]).as_deref(),
        Some("ldp dst=x1 dst2=x9 base=x9 offset=0 addressing=offset width=8")
    );
    assert!(one(vec![load("x1", "x9", 0), load("x1", "x9", 8)]).is_none());
    // Stores have no such rule.
    assert!(one(vec![store("x9", "x9", 0), store("x9", "x9", 8)]).is_some());
}

#[test]
fn a_label_between_two_accesses_keeps_them_apart() {
    let (formed, stream) = paired(vec![store("x1", "x9", 0), label(), store("x2", "x9", 8)]);
    assert_eq!(formed, 0);
    assert_eq!(stream.len(), 3);
}

#[test]
fn a_run_pairs_greedily_from_the_left() {
    let (formed, stream) = paired(vec![
        store("x1", "sp", 0),
        store("x2", "sp", 8),
        store("x3", "sp", 16),
    ]);
    assert_eq!(formed, 1);
    assert_eq!(
        stream,
        [
            "stp src=x1 src2=x2 base=sp offset=0 addressing=offset width=8",
            "str_u64 src=x3 base=sp offset=16"
        ]
    );
}

#[test]
fn the_pair_keeps_the_first_access_s_source() {
    let first = store("x1", "x9", 0);
    let mut instructions = vec![first.clone(), store("x2", "x9", 8)];
    pair_adjacent_accesses(&mut instructions);
    assert_eq!(instructions[0].source, first.source);
}

#[test]
fn level_zero_forms_no_pair() {
    with_opt_level(OptLevel(0), || {
        let (formed, stream) = paired(vec![store("x1", "x9", 0), store("x2", "x9", 8)]);
        assert_eq!(formed, 0);
        assert_eq!(stream.len(), 2);
    });
}
