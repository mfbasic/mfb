//! `engine/regalloc/mod.rs`'s own contracts, past the per-class linear scan the
//! `regalloc/tests.rs` suite drives: the frame save set merges both classes, a
//! pass rewriting a stream it did not mint draws FP ids above everything in it,
//! and the `MFB_BENCH_LOWERING` probe counts each distinct virtual register once.

use crate::arch::aarch64::regmodel::Aarch64RegisterModel;
use crate::codegen::engine::regalloc::{
    allocate, bench_lowering_line, fp_vreg_name, mint_vreg, mint_vreg_above, parse_fp_vreg,
    vreg_name,
};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegClass;

/// One integer and one FP value, each live across a call, are each kept in a
/// callee-saved register of its own class, and the frame saves both: the save
/// set is the two classes' lists together, each register once.
#[test]
fn the_frame_saves_the_callee_saved_registers_of_both_classes() {
    let mut instructions = vec![
        CodeInstruction::new("label").field("name", "entry"),
        CodeInstruction::new("mov_imm")
            .physical_field("dst", vreg_name(0))
            .field("type", "Integer")
            .field("value", "5"),
        CodeInstruction::new("fmov_d_from_d")
            .physical_field("dst", fp_vreg_name(0))
            .physical_field("src", "d0"),
        CodeInstruction::new("bl").field("target", "_mfb_fn_user"),
        CodeInstruction::new("str_u64")
            .physical_field("src", vreg_name(0))
            .physical_field("base", "sp")
            .field("offset", "0"),
        CodeInstruction::new("fmov_d_from_d")
            .physical_field("dst", "d1")
            .physical_field("src", fp_vreg_name(0)),
        CodeInstruction::new("ret"),
    ];
    let outcome = allocate(&mut instructions, &Aarch64RegisterModel, 64, &[]);
    let int = instructions[1].get("dst").expect("dst").to_string();
    let fp = instructions[2].get("dst").expect("dst").to_string();
    assert!(int.starts_with('x'), "the integer value's home: {int}");
    assert!(fp.starts_with('d'), "the FP value's home: {fp}");
    let mut saved = outcome.extra_callee_saved.clone();
    saved.sort();
    assert_eq!(saved, {
        let mut both = vec![int, fp];
        both.sort();
        both
    });
}

/// GVN rewrites a stream whose ids it did not mint, so the id it draws must be
/// above `floor` — and every later draw of that class above it too, or a fresh
/// register would alias one already in the stream.
#[test]
fn an_fp_register_minted_above_a_floor_is_above_it_and_stays_so() {
    let floor = 5_000_000;
    let above = parse_fp_vreg(&mint_vreg_above(RegClass::Fp, floor).render())
        .expect("an FP virtual register");
    assert!(above > floor, "{above} <= {floor}");
    let next = parse_fp_vreg(&mint_vreg(RegClass::Fp).render()).expect("an FP virtual register");
    assert!(next > above, "{next} <= {above}");
}

/// The probe reports the stream's length and its DISTINCT integer and FP virtual
/// registers: a register named twice counts once, and a physical one not at all.
#[test]
fn the_bench_probe_counts_distinct_virtual_registers_per_class() {
    let instructions = vec![
        CodeInstruction::new("mov_imm")
            .physical_field("dst", vreg_name(3))
            .field("type", "Integer")
            .field("value", "1"),
        CodeInstruction::new("add")
            .physical_field("dst", vreg_name(4))
            .physical_field("left", vreg_name(3))
            .physical_field("right", "x9"),
        CodeInstruction::new("fmov_d_from_d")
            .physical_field("dst", fp_vreg_name(7))
            .physical_field("src", "d0"),
        CodeInstruction::new("ret"),
    ];
    assert_eq!(
        bench_lowering_line(&instructions),
        "MFB_BENCH_LOWERING: function instructions=4 int_vregs=2 fp_vregs=1"
    );
}
