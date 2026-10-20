use super::*;

fn map(pairs: &[(&str, usize)]) -> HashMap<String, usize> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

#[test]
fn detects_vector_operands() {
    assert!(is_vector_operand("v0"));
    assert!(is_vector_operand("d16"));
    assert!(is_vector_operand("%f7"));
    assert!(!is_vector_operand("a0"));
    assert!(!is_vector_operand("%v3"));
    assert!(!is_vector_operand("128"));
}

#[test]
fn fadd_v_scalarizes_to_two_lane_adds() {
    let fields = vec![
        ("dst", "v0".into()),
        ("lhs", "%f7".into()),
        ("rhs", "v2".into()),
    ];
    let slots = map(&[("v0", 0), ("%f7", 1), ("v2", 2)]);
    let out = scalarize_v128(CodeOp::FAddV, &fields, &slots);
    // Per-thread slot base is one `addi t2, s11, ARENA_V128_SLOTS_OFFSET`
    // (bug-122), then 2 lanes × (ldr,ldr,fadd,str) = 1 + 8.
    assert_eq!(out.len(), 9);
    assert_eq!(out[0].op.mnemonic(), "add_imm");
    assert_eq!(out[0].get("src").as_deref(), Some("s11"));
    let expected_offset = crate::codegen::error::constants::ARENA_V128_SLOTS_OFFSET.to_string();
    assert_eq!(out[0].get("imm").as_deref(), Some(expected_offset.as_str()));
    assert_eq!(
        out.iter().filter(|i| i.op.mnemonic() == "fadd_d").count(),
        2
    );
    // %f7 (slot 1) low lane reads offset 16.
    assert!(out.iter().any(|i| i.get("offset").as_deref() == Some("16")));
}

/// bug-284 C4: the lane index was parsed with `unwrap_or(0)`, which mapped a
/// malformed index to lane 0, and nothing bounded it. `index = 2` computes
/// `slot * 16 + 16`, i.e. the low lane of the *adjacent slot* -- an unrelated
/// value's data. AArch64 rejects the same input loudly, so a builder bug that
/// failed the build there miscompiled silently here.
#[test]
#[should_panic(expected = "umov .d lane index out of range")]
fn umov_rejects_an_out_of_range_lane_index() {
    let fields = vec![
        ("dst", "a0".into()),
        ("src", "v1".into()),
        ("index", "2".into()),
    ];
    scalarize_v128(CodeOp::UmovXFromV, &fields, &map(&[("v1", 0)]));
}

#[test]
#[should_panic(expected = "umov .d lane index out of range")]
fn umov_rejects_a_malformed_lane_index() {
    let fields = vec![
        ("dst", "a0".into()),
        ("src", "v1".into()),
        ("index", "high".into()),
    ];
    scalarize_v128(CodeOp::UmovXFromV, &fields, &map(&[("v1", 0)]));
}

#[test]
fn umov_still_accepts_both_real_lanes() {
    for (index, offset) in [("0", "0"), ("1", "8")] {
        let fields = vec![
            ("dst", "a0".into()),
            ("src", "v1".into()),
            ("index", index.into()),
        ];
        let out = scalarize_v128(CodeOp::UmovXFromV, &fields, &map(&[("v1", 0)]));
        assert!(
            out.iter()
                .any(|i| i.get("offset").as_deref() == Some(offset)),
            "lane {index} should read offset {offset}"
        );
    }
}

/// bug-284 C5: the high lane sits 8 bytes past the low lane, so both offsets
/// must derive from the same value. `unwrap_or(0) + 8` instead produced the
/// literal 8 for any non-u64 spelling while the low-lane op forwarded the raw
/// string, leaving the two lanes addressing inconsistent locations. It failed
/// loudly only because the sibling op happened to hand the same string to
/// `operand::immediate()`, which rejects it -- a coincidence, not a guarantee.
#[test]
#[should_panic(expected = "needs a numeric offset")]
fn ldr_q_rejects_a_non_numeric_offset_on_its_own_terms() {
    let fields = vec![
        ("dst", "v0".into()),
        ("base", "a0".into()),
        ("offset", "-8".into()),
    ];
    scalarize_v128(CodeOp::LdrQ, &fields, &map(&[("v0", 0)]));
}

#[test]
#[should_panic(expected = "needs a numeric offset")]
fn str_q_rejects_a_non_numeric_offset_on_its_own_terms() {
    let fields = vec![
        ("src", "v0".into()),
        ("base", "a0".into()),
        ("offset", "some_symbol".into()),
    ];
    scalarize_v128(CodeOp::StrQ, &fields, &map(&[("v0", 0)]));
}

#[test]
fn ldr_q_lanes_stay_eight_bytes_apart_for_a_real_offset() {
    let fields = vec![
        ("dst", "v0".into()),
        ("base", "a0".into()),
        ("offset", "32".into()),
    ];
    let out = scalarize_v128(CodeOp::LdrQ, &fields, &map(&[("v0", 0)]));
    assert!(out.iter().any(|i| i.get("offset").as_deref() == Some("32")));
    assert!(out.iter().any(|i| i.get("offset").as_deref() == Some("40")));
}

#[test]
fn lane_shift_by_sixty_four_matches_aarch64() {
    // bug-16: AArch64 `.2d` right shifts take 1..=64. Forwarding 64 to a
    // scalar RISC-V shift is an encoder error (shamt is 6 bits), so the
    // boundary is lowered directly: arithmetic → sign fill, logical → zero.
    let fields = vec![
        ("dst", "v0".into()),
        ("src", "v1".into()),
        ("shift", "64".into()),
    ];
    let slots = map(&[("v0", 0), ("v1", 1)]);

    let sshr = scalarize_v128(CodeOp::SshrV, &fields, &slots);
    // Both lanes sign-fill with `srai t0, t0, 63` — never a shift of 64.
    assert_eq!(
        sshr.iter()
            .filter(|i| i.op.mnemonic() == "asr_imm" && i.get("shift").as_deref() == Some("63"))
            .count(),
        2
    );
    assert!(sshr.iter().all(|i| i.get("shift").as_deref() != Some("64")));

    // Logical/left shift out every bit: store the `zero` register per lane.
    for op in [CodeOp::UshrV, CodeOp::ShlV] {
        let out = scalarize_v128(op, &fields, &slots);
        let zeroed = out
            .iter()
            .filter(|i| i.op.mnemonic() == "str_u64" && i.get("src").as_deref() == Some("zero"))
            .count();
        assert_eq!(zeroed, 2, "{} lanes must be zeroed", op.mnemonic());
        assert!(out.iter().all(|i| i.get("shift").as_deref() != Some("64")));
    }

    // In-range shifts are untouched.
    let three = vec![
        ("dst", "v0".into()),
        ("src", "v1".into()),
        ("shift", "3".into()),
    ];
    let out = scalarize_v128(CodeOp::UshrV, &three, &slots);
    assert_eq!(
        out.iter()
            .filter(|i| i.op.mnemonic() == "lsr_imm" && i.get("shift").as_deref() == Some("3"))
            .count(),
        2
    );
}

fn mir(op: crate::codegen::engine::mir::MirOp, fields: &[(&'static str, &str)]) -> MirInstruction {
    MirInstruction {
        op,
        fields: fields.iter().map(|(k, v)| (*k, v.into())).collect(),
        source: None,
    }
}

fn peak(map: &HashMap<String, usize>) -> usize {
    map.values().map(|s| s + 1).max().unwrap_or(0)
}

#[test]
fn slots_are_reused_across_disjoint_live_ranges() {
    use crate::codegen::engine::mir::MirOp;
    // Two independent lane-adds in straight-line code: the second op's values
    // recycle the first op's slots (their ranges do not overlap), so six
    // distinct values need only three concurrent slots.
    let inst = vec![
        mir(
            MirOp::FAddV,
            &[("dst", "%f0"), ("lhs", "%f1"), ("rhs", "%f2")],
        ),
        mir(
            MirOp::FAddV,
            &[("dst", "%f3"), ("lhs", "%f4"), ("rhs", "%f5")],
        ),
    ];
    let slots = build_slot_map(&inst);
    assert_eq!(slots.len(), 6, "all six values are mapped");
    assert_eq!(peak(&slots), 3, "but only three slots are live at once");
}

#[test]
fn loop_carried_values_never_share_a_slot() {
    use crate::codegen::engine::mir::MirOp;
    // The same two ops inside a loop (a backward branch to `top`): a value
    // defined late could be read early on the next iteration, so live ranges
    // are extended across the whole loop and no slot is recycled within it —
    // otherwise a loop-carried value would be silently clobbered.
    let inst = vec![
        mir(MirOp::Label, &[("name", "top")]),
        mir(
            MirOp::FAddV,
            &[("dst", "%f0"), ("lhs", "%f1"), ("rhs", "%f2")],
        ),
        mir(
            MirOp::FAddV,
            &[("dst", "%f3"), ("lhs", "%f4"), ("rhs", "%f5")],
        ),
        mir(
            MirOp::BranchEq,
            &[("lhs", "a0"), ("rhs", "a1"), ("target", "top")],
        ),
    ];
    let slots = build_slot_map(&inst);
    assert_eq!(slots.len(), 6);
    assert_eq!(
        peak(&slots),
        6,
        "loop extension keeps all six values distinct"
    );
}

/// plan-32-C: `build_vreg_map` reproduces the slot map's liveness as physical
/// vector registers — reuse across disjoint ranges packs into few registers,
/// loop-carried values stay distinct, and pressure past the `v1`–`v30` pool
/// falls back to `None` (scalar-arm-only).
#[test]
fn vreg_map_reuse_loop_distinctness_and_overflow() {
    use crate::codegen::engine::mir::MirOp;
    use std::collections::HashSet;

    // Reuse: six values across two disjoint straight-line ops → three regs.
    let straight = vec![
        mir(
            MirOp::FAddV,
            &[("dst", "%f0"), ("lhs", "%f1"), ("rhs", "%f2")],
        ),
        mir(
            MirOp::FAddV,
            &[("dst", "%f3"), ("lhs", "%f4"), ("rhs", "%f5")],
        ),
    ];
    let regs = build_vreg_map(&straight).expect("fits the pool");
    assert_eq!(regs.len(), 6, "all six values are assigned a register");
    let distinct: HashSet<u8> = regs.values().copied().collect();
    assert_eq!(distinct.len(), 3, "only three registers are live at once");
    assert!(
        regs.values().all(|&r| (1..=30).contains(&r)),
        "registers come from the v1..=v30 allocatable pool"
    );

    // Loop-carried: the same ops inside a loop keep all six distinct.
    let looped = vec![
        mir(MirOp::Label, &[("name", "top")]),
        mir(
            MirOp::FAddV,
            &[("dst", "%f0"), ("lhs", "%f1"), ("rhs", "%f2")],
        ),
        mir(
            MirOp::FAddV,
            &[("dst", "%f3"), ("lhs", "%f4"), ("rhs", "%f5")],
        ),
        mir(
            MirOp::BranchEq,
            &[("lhs", "a0"), ("rhs", "a1"), ("target", "top")],
        ),
    ];
    let looped_regs = build_vreg_map(&looped).expect("six fits the pool");
    let looped_distinct: HashSet<u8> = looped_regs.values().copied().collect();
    assert_eq!(
        looped_distinct.len(),
        6,
        "loop extension keeps all six distinct"
    );

    // Overflow: N values all live at once (loaded, then all stored). 30 fits
    // the pool; 31 exceeds it and falls back to scalar-only (`None`).
    let all_live = |n: usize| -> Vec<MirInstruction> {
        let mut v = Vec::new();
        for i in 0..n {
            v.push(mir(
                MirOp::LdrQ,
                &[("dst", "%f0"), ("base", "a0"), ("offset", "0")],
            ));
            // Overwrite dst with a distinct name each iteration.
            *v.last_mut()
                .unwrap()
                .fields
                .iter_mut()
                .find(|(k, _)| *k == "dst")
                .unwrap() = ("dst", format!("%f{i}").into());
        }
        for i in 0..n {
            v.push(mir(
                MirOp::StrQ,
                &[("src", "%f0"), ("base", "a0"), ("offset", "0")],
            ));
            *v.last_mut()
                .unwrap()
                .fields
                .iter_mut()
                .find(|(k, _)| *k == "src")
                .unwrap() = ("src", format!("%f{i}").into());
        }
        v
    };
    assert!(
        build_vreg_map(&all_live(30)).is_some(),
        "30 concurrent fits v1..=v30"
    );
    assert!(
        build_vreg_map(&all_live(31)).is_none(),
        "31 concurrent overflows the pool → scalar-arm-only"
    );
}

/// plan-32-C Phase 2: a vector-lowered op emits the runtime dual path — a
/// `_mfb_rt_has_rvv` guard, the RVV arm (`vsetivli` + the `vf*`/`v*` op), and
/// the unchanged scalar arm, joined by unique labels. With no vreg map (pool
/// overflow) it is scalar-only, byte-identical to `scalarize_v128`.
#[test]
fn dual_path_emits_guard_rvv_and_scalar_arms() {
    let fields = vec![
        ("dst", "%f0".into()),
        ("lhs", "%f1".into()),
        ("rhs", "%f2".into()),
    ];
    let slots = map(&[("%f0", 0), ("%f1", 1), ("%f2", 2)]);
    let vregs: HashMap<String, u8> = [("%f0", 1u8), ("%f1", 2), ("%f2", 3)]
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();

    let out = lower_v128_run(&[(CodeOp::FAddV, fields.clone())], &slots, &vregs, 0);
    let vop = |i: &CodeInstruction| i.get("vop");
    // Guard: load the flag and branch to the scalar label when it is clear.
    assert!(out.iter().any(|i| i.op.mnemonic() == "adrp"
        && i.get("symbol").as_deref()
            == Some(crate::codegen::error::constants::HAS_RVV_GLOBAL_SYMBOL)));
    assert!(out.iter().any(|i| i.op.mnemonic() == "rv.br"
        && i.get("target").as_deref() == Some("v128_scalar_0")
        && i.get("cond").as_deref() == Some("eq")));
    // RVV arm: vsetivli then vfadd.vv on the assigned registers.
    assert!(out.iter().any(|i| vop(i).as_deref() == Some("vsetivli")));
    assert!(out.iter().any(|i| vop(i).as_deref() == Some("vfadd.vv")
        && i.get("dst").as_deref() == Some("v1")
        && i.get("lhs").as_deref() == Some("v2")
        && i.get("rhs").as_deref() == Some("v3")));
    // Scalar arm (unchanged) is present behind the label, and both arms converge.
    assert!(out
        .iter()
        .any(|i| i.op.mnemonic() == "label" && i.get("name").as_deref() == Some("v128_scalar_0")));
    assert!(out.iter().any(|i| i.op.mnemonic() == "fadd_d"));
    assert!(out
        .iter()
        .any(|i| i.op.mnemonic() == "label" && i.get("name").as_deref() == Some("v128_done_0")));

    // A non-lowerable op (or overflow) is never accumulated into a run —
    // `rvv_lowerable` rejects it, so `select_riscv64` scalarizes it directly,
    // with no guard.
    assert!(!rvv_lowerable(
        CodeOp::FRintnV,
        &[("dst", "%f0".into()), ("src", "%f1".into())]
    ));
    assert!(!scalarize_v128(CodeOp::FAddV, &fields, &slots)
        .iter()
        .any(|i| i.op.mnemonic() == "adrp"));
}

/// plan-32-C Phase 3: the mask bridge. A float compare emits the ordered
/// `vmf*` into v0 then materializes the all-ones/all-zeros lane vector
/// (`vmv.v.i`+`vmerge.vim`); `BslV` is the `vxor`/`vand`/`vxor` bit-select over
/// those lanes; min/max are direct `vfmin`/`vfmax`.
#[test]
fn mask_bridge_and_minmax_rvv_arms() {
    let vregs: HashMap<String, u8> = [("%f0", 1u8), ("%f1", 2), ("%f2", 3)]
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
    let slots = map(&[("%f0", 0), ("%f1", 1), ("%f2", 2)]);
    let vop_seq = |out: &[CodeInstruction]| -> Vec<String> {
        out.iter().filter_map(|i| i.get("vop")).collect()
    };

    // FCmGtV → vmflt.vv into v0, then vmv.v.i / vmerge.vim into the dst reg.
    let cmp = rvv_arm(
        CodeOp::FCmGtV,
        &[
            ("dst", "%f0".into()),
            ("lhs", "%f1".into()),
            ("rhs", "%f2".into()),
        ],
        &slots,
        &vregs,
    )
    .expect("FCmGtV is vector-lowered");
    assert!(cmp
        .iter()
        .any(|i| i.get("vop").as_deref() == Some("vmflt.vv")
            && i.get("dst").as_deref() == Some("v0")));
    let seq = vop_seq(&cmp);
    let merge = seq
        .iter()
        .position(|v| v == "vmerge.vim")
        .expect("materializes lanes");
    assert_eq!(
        seq[merge - 1],
        "vmv.v.i",
        "zero the lane vector before merging -1"
    );

    // BslV → vxor;vand;vxor over the lane vectors.
    let bsl = rvv_arm(
        CodeOp::BslV,
        &[
            ("dst", "%f0".into()),
            ("lhs", "%f1".into()),
            ("rhs", "%f2".into()),
        ],
        &slots,
        &vregs,
    )
    .expect("BslV is vector-lowered");
    let bsl_seq = vop_seq(&bsl);
    assert_eq!(
        bsl_seq.iter().filter(|v| v.as_str() == "vxor.vv").count(),
        2,
        "bit-select is two xors around one and"
    );
    assert!(bsl_seq.iter().any(|v| v == "vand.vv"));

    // Min/max → direct vfmin/vfmax.
    for (op, mn) in [(CodeOp::FMinV, "vfmin.vv"), (CodeOp::FMaxV, "vfmax.vv")] {
        let out = rvv_arm(
            op,
            &[
                ("dst", "%f0".into()),
                ("lhs", "%f1".into()),
                ("rhs", "%f2".into()),
            ],
            &slots,
            &vregs,
        )
        .expect("min/max is vector-lowered");
        assert!(out.iter().any(|i| i.get("vop").as_deref() == Some(mn)));
    }

    // A deferred op (FRint*) still returns None → scalar-only.
    assert!(rvv_arm(
        CodeOp::FRintnV,
        &[("dst", "%f0".into()), ("src", "%f1".into())],
        &slots,
        &vregs,
    )
    .is_none());
}

/// plan-32-D: a multi-op run emits ONE guard + ONE vsetivli, and keeps a
/// value produced mid-run resident — the second op reads the first op's
/// result straight from its register, with no slot reload.
#[test]
fn per_run_single_guard_and_resident_chain() {
    // %f3 = %f0 + %f1 ; %f5 = %f3 * %f2  — %f3 is produced then consumed.
    let run = vec![
        (
            CodeOp::FAddV,
            vec![
                ("dst", "%f3".into()),
                ("lhs", "%f0".into()),
                ("rhs", "%f1".into()),
            ],
        ),
        (
            CodeOp::FMulV,
            vec![
                ("dst", "%f5".into()),
                ("lhs", "%f3".into()),
                ("rhs", "%f2".into()),
            ],
        ),
    ];
    let slots = map(&[("%f0", 0), ("%f1", 1), ("%f2", 2), ("%f3", 3), ("%f5", 4)]);
    let vregs: HashMap<String, u8> = [("%f0", 1u8), ("%f1", 2), ("%f2", 3), ("%f3", 4), ("%f5", 5)]
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();

    let out = lower_v128_run(&run, &slots, &vregs, 0);
    // One guard, one config for the whole run.
    assert_eq!(
        out.iter()
            .filter(
                |i| i.op.mnemonic() == "label" && i.get("name").as_deref() == Some("v128_scalar_0")
            )
            .count(),
        1,
        "one guard per run"
    );
    assert_eq!(
        out.iter()
            .filter(|i| i.get("vop").as_deref() == Some("vsetivli"))
            .count(),
        1,
        "one vsetivli per run"
    );
    // Residency: %f3 lives in v4; op2 must NOT reload it (no vle64 into v4).
    assert!(
        !out.iter()
            .any(|i| i.get("vop").as_deref() == Some("vle64.v")
                && i.get("dst").as_deref() == Some("v4")),
        "the mid-run value stays resident — its reload is elided"
    );
    // The multiply still reads it from its register.
    assert!(out
        .iter()
        .any(|i| i.get("vop").as_deref() == Some("vfmul.vv")
            && i.get("lhs").as_deref() == Some("v4")
            && i.get("rhs").as_deref() == Some("v3")));
}

#[test]
fn dup_broadcasts_both_lanes() {
    let fields = vec![("dst", "v3".into()), ("src", "a0".into())];
    let slots = map(&[("v3", 0)]);
    let out = scalarize_v128(CodeOp::DupVFromX, &fields, &slots);
    assert_eq!(
        out.iter().filter(|i| i.op.mnemonic() == "str_u64").count(),
        2
    );
    // src (a0, a GPR) passes through unslotted.
    assert!(out.iter().any(|i| i.get("src").as_deref() == Some("a0")));
}

// ---------- extended op-lowering coverage ----------

/// A generous slot map for the value names the op tests use.
fn big() -> HashMap<String, usize> {
    map(&[("v0", 0), ("v1", 1), ("v2", 2)])
}

/// Build `(field, String)` operand pairs.
fn fl(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, Operand)> {
    pairs.iter().map(|(k, v)| (*k, v.into())).collect()
}

fn count(out: &[CodeInstruction], mnemonic: &str) -> usize {
    out.iter().filter(|i| i.op.mnemonic() == mnemonic).count()
}

#[test]
fn scratch_token_operands_are_vector_values() {
    // plan-34-D VEC_SCRATCH/FP_SCRATCH token pools reach this pass unrealized.
    assert!(is_vector_operand("%vscratch0"));
    assert!(is_vector_operand("%vscratch7"));
    assert!(is_vector_operand("%fscratch3"));
    assert!(!is_vector_operand("%vscratch8")); // out of the 0..=7 pool
    assert!(!is_vector_operand("%fscratchZ"));
    assert!(is_vector_operand("%f42"));
    assert!(!is_vector_operand("%fq"));
}

#[test]
fn fp_three_same_ops_lower_per_lane() {
    let fields = fl(&[("dst", "v0"), ("lhs", "v1"), ("rhs", "v2")]);
    for (op, mn) in [
        (CodeOp::FAddV, "fadd_d"),
        (CodeOp::FSubV, "fsub_d"),
        (CodeOp::FMulV, "fmul_d"),
        (CodeOp::FDivV, "fdiv_d"),
    ] {
        let out = scalarize_v128(op, &fields, &big());
        assert_eq!(out[0].op.mnemonic(), "add_imm");
        assert_eq!(count(&out, mn), 2, "{} lanes", op.mnemonic());
    }
}

#[test]
fn fp_min_max_use_number_semantics() {
    let fields = fl(&[("dst", "v0"), ("lhs", "v1"), ("rhs", "v2")]);
    assert_eq!(
        count(&scalarize_v128(CodeOp::FMinV, &fields, &big()), "fminnm_d"),
        2
    );
    assert_eq!(
        count(&scalarize_v128(CodeOp::FMaxV, &fields, &big()), "fmaxnm_d"),
        2
    );
}

#[test]
fn fmla_fuses_and_fmls_subtracts() {
    let fields = fl(&[("dst", "v0"), ("lhs", "v1"), ("rhs", "v2")]);
    let mla = scalarize_v128(CodeOp::FMlaV, &fields, &big());
    assert_eq!(count(&mla, "fmadd_d"), 2);
    // bug-158: dst -= lhs*rhs must be a single fused rounding (`fnmsub_d` =
    // addend − lhs*rhs), not a fmul+fsub pair (two roundings).
    let mls = scalarize_v128(CodeOp::FMlsV, &fields, &big());
    assert_eq!(count(&mls, "fnmsub_d"), 2);
    assert_eq!(count(&mls, "fmul_d"), 0);
    assert_eq!(count(&mls, "fsub_d"), 0);
}

#[test]
fn fp_two_reg_misc_ops_lower_per_lane() {
    let fields = fl(&[("dst", "v0"), ("src", "v1")]);
    for (op, mn) in [
        (CodeOp::FAbsV, "fabs_d"),
        (CodeOp::FNegV, "fneg_d"),
        (CodeOp::FSqrtV, "fsqrt_d"),
    ] {
        assert_eq!(count(&scalarize_v128(op, &fields, &big()), mn), 2);
    }
}

#[test]
fn frint_directed_modes_and_ties_even() {
    let fields = fl(&[("dst", "v0"), ("src", "v1")]);
    // Directed modes emit their mode-specific fcvt (one per lane).
    for (op, cvt) in [
        (CodeOp::FRintmV, "fcvtms_x_from_d"),
        (CodeOp::FRintpV, "fcvtps_x_from_d"),
        (CodeOp::FRintzV, "fcvtzs_x_from_d"),
        (CodeOp::FRintaV, "fcvtas_x_from_d"),
    ] {
        let out = scalarize_v128(op, &fields, &big());
        assert_eq!(count(&out, cvt), 2, "{}", op.mnemonic());
        // Branchless magic-number mask select uses sltu per lane.
        assert_eq!(count(&out, "rv.sltu"), 2);
    }
    // Nearest-ties-even takes the 2^52 magic-number add/sub path (no fcvt).
    let n = scalarize_v128(CodeOp::FRintnV, &fields, &big());
    assert_eq!(count(&n, "fadd_d"), 2);
    assert_eq!(count(&n, "fsub_d"), 2);
    assert!(n.iter().all(|i| !i.op.mnemonic().starts_with("fcvt")));
}

#[test]
fn float_to_int_and_int_to_float_lane_converts() {
    let fields = fl(&[("dst", "v0"), ("src", "v1")]);
    assert_eq!(
        count(
            &scalarize_v128(CodeOp::FCvtzsV, &fields, &big()),
            "fcvtzs_x_from_d"
        ),
        2
    );
    assert_eq!(
        count(
            &scalarize_v128(CodeOp::FCvtasV, &fields, &big()),
            "fcvtas_x_from_d"
        ),
        2
    );
    assert_eq!(
        count(
            &scalarize_v128(CodeOp::ScvtfV, &fields, &big()),
            "scvtf_d_from_x"
        ),
        2
    );
}

#[test]
fn fp_lane_compares_build_masks() {
    let fields = fl(&[("dst", "v0"), ("lhs", "v1"), ("rhs", "v2")]);
    for op in [CodeOp::FCmGtV, CodeOp::FCmGeV, CodeOp::FCmEqV] {
        let out = scalarize_v128(op, &fields, &big());
        assert_eq!(count(&out, "rv.fcmp"), 2, "{}", op.mnemonic());
        assert_eq!(count(&out, "sub"), 2); // mask = -bool
    }
}

#[test]
fn fp_compare_against_zero_masks() {
    let fields = fl(&[("dst", "v0"), ("src", "v1")]);
    for op in [
        CodeOp::FCmGtZeroV,
        CodeOp::FCmGeZeroV,
        CodeOp::FCmEqZeroV,
        CodeOp::FCmLtZeroV,
        CodeOp::FCmLeZeroV,
    ] {
        let out = scalarize_v128(op, &fields, &big());
        assert_eq!(count(&out, "rv.fcmp"), 2, "{}", op.mnemonic());
        // +0.0 materialized per lane.
        assert_eq!(count(&out, "fmov_d_from_x"), 2);
    }
}

#[test]
fn integer_add_sub_lanes() {
    let fields = fl(&[("dst", "v0"), ("lhs", "v1"), ("rhs", "v2")]);
    assert_eq!(
        count(&scalarize_v128(CodeOp::AddV, &fields, &big()), "add"),
        2
    );
    assert_eq!(
        count(&scalarize_v128(CodeOp::SubV, &fields, &big()), "sub"),
        2
    );
}

#[test]
fn integer_lane_compares() {
    let fields = fl(&[("dst", "v0"), ("lhs", "v1"), ("rhs", "v2")]);
    let gt = scalarize_v128(CodeOp::CmGtV, &fields, &big());
    assert_eq!(count(&gt, "rv.slt"), 2);
    let ge = scalarize_v128(CodeOp::CmGeV, &fields, &big());
    assert_eq!(count(&ge, "rv.slt"), 2);
    assert_eq!(count(&ge, "sub_imm"), 2);
    let eq = scalarize_v128(CodeOp::CmEqV, &fields, &big());
    assert_eq!(count(&eq, "eor"), 2);
    assert_eq!(count(&eq, "rv.sltu"), 2);
}

#[test]
fn integer_neg_and_abs() {
    let fields = fl(&[("dst", "v0"), ("src", "v1")]);
    assert_eq!(
        count(&scalarize_v128(CodeOp::NegV, &fields, &big()), "sub"),
        2
    );
    let abs = scalarize_v128(CodeOp::AbsV, &fields, &big());
    assert_eq!(count(&abs, "asr_imm"), 2);
    assert_eq!(count(&abs, "eor"), 2);
}

#[test]
fn bitwise_ops_lower_per_lane() {
    let fields = fl(&[("dst", "v0"), ("lhs", "v1"), ("rhs", "v2")]);
    for (op, mn) in [
        (CodeOp::AndV, "and"),
        (CodeOp::OrrV, "orr"),
        (CodeOp::EorV, "eor"),
    ] {
        assert_eq!(
            count(&scalarize_v128(op, &fields, &big()), mn),
            2,
            "{}",
            op.mnemonic()
        );
    }
}

#[test]
fn bit_select_and_bit_insert() {
    let fields = fl(&[("dst", "v0"), ("lhs", "v1"), ("rhs", "v2")]);
    let bsl = scalarize_v128(CodeOp::BslV, &fields, &big());
    // Two `eor` per lane (a^b, then b^...) plus one `and`.
    assert_eq!(count(&bsl, "eor"), 4);
    assert_eq!(count(&bsl, "and"), 2);
    let bit = scalarize_v128(CodeOp::BitV, &fields, &big());
    assert_eq!(count(&bit, "eor"), 4);
    assert_eq!(count(&bit, "and"), 2);
}

#[test]
fn quad_load_store_move_sixteen_bytes() {
    let ld = scalarize_v128(
        CodeOp::LdrQ,
        &fl(&[("dst", "v0"), ("base", "a0"), ("offset", "0")]),
        &big(),
    );
    // Two 8-byte loads from the source pointer, two stores into the slot.
    assert_eq!(count(&ld, "ldr_u64"), 2);
    assert_eq!(count(&ld, "str_u64"), 2);
    // High lane reads source offset 8.
    assert!(ld.iter().any(|i| i.get("offset").as_deref() == Some("8")));

    let st = scalarize_v128(
        CodeOp::StrQ,
        &fl(&[("src", "v0"), ("base", "a0"), ("offset", "0")]),
        &big(),
    );
    assert_eq!(count(&st, "ldr_u64"), 2);
    assert_eq!(count(&st, "str_u64"), 2);
}

#[test]
fn umov_extracts_one_lane() {
    // dst is a GPR (passes through unslotted); index selects the lane half.
    let out = scalarize_v128(
        CodeOp::UmovXFromV,
        &fl(&[("dst", "a0"), ("src", "v1"), ("index", "1")]),
        &big(),
    );
    assert_eq!(count(&out, "ldr_u64"), 1);
    assert_eq!(out.last().unwrap().get("dst").as_deref(), Some("a0"));
    // Slot 1, high half → offset 1*16 + 8 = 24.
    assert_eq!(out.last().unwrap().get("offset").as_deref(), Some("24"));
}

#[test]
#[should_panic(expected = "not yet scalarized")]
fn unhandled_v128_op_panics() {
    // `SshlV` is a recognized v128 op with no scalarization arm (register-shift
    // by a vector, unused on rv64) — it must fail loud, not silently miscompile.
    let fields = fl(&[("dst", "v0"), ("lhs", "v1"), ("rhs", "v2")]);
    let _ = scalarize_v128(CodeOp::SshlV, &fields, &big());
}

// ---------- native-RVV arm (`rvv_arm`) per-op coverage ----------

/// A slot+vreg map for the value names the RVV-arm tests use.
fn slots_and_vregs() -> (HashMap<String, usize>, HashMap<String, u8>) {
    let slots = map(&[("v0", 0), ("v1", 1), ("v2", 2)]);
    let vregs: HashMap<String, u8> = [("v0", 1u8), ("v1", 2), ("v2", 3)]
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
    (slots, vregs)
}

/// Every vector-lowered op must produce a native-RVV body carrying the RVV
/// mnemonic its arm is responsible for. `rvv_arm` is exercised **directly**
/// (not through `lower_v128_run`, which inlines it and misattributes the arm
/// coverage): this pins each arm's `vf*`/`v*`/mask-bridge emission op-by-op,
/// mirroring the `scalarize_v128` per-op tests above for the scalar arm.
#[test]
fn rvv_arm_lowers_every_vector_op() {
    let (slots, vregs) = slots_and_vregs();
    let arm = |op: CodeOp, pairs: &[(&'static str, &str)]| -> Vec<CodeInstruction> {
        rvv_arm(op, &fl(pairs), &slots, &vregs)
            .unwrap_or_else(|| panic!("{} must be vector-lowered", op.mnemonic()))
    };
    let has = |out: &[CodeInstruction], vop: &str| {
        out.iter().any(|i| i.get("vop").as_deref() == Some(vop))
    };

    let ss = &[("dst", "v0"), ("src", "v1")]; // two-reg-misc
    let abc = &[("dst", "v0"), ("lhs", "v1"), ("rhs", "v2")]; // three-same

    // Float three-same and fused multiply-add/subtract.
    for (op, vop) in [
        (CodeOp::FAddV, "vfadd.vv"),
        (CodeOp::FSubV, "vfsub.vv"),
        (CodeOp::FMulV, "vfmul.vv"),
        (CodeOp::FDivV, "vfdiv.vv"),
        (CodeOp::FMlaV, "vfmacc.vv"),
        (CodeOp::FMlsV, "vfnmsac.vv"),
        (CodeOp::FMinV, "vfmin.vv"),
        (CodeOp::FMaxV, "vfmax.vv"),
    ] {
        assert!(has(&arm(op, abc), vop), "{} → {vop}", op.mnemonic());
    }

    // Float two-reg-misc: abs/neg via sign-injection, sqrt, and the two
    // f64<->i64 converters.
    for (op, vop) in [
        (CodeOp::FAbsV, "vfsgnjx.vv"),
        (CodeOp::FNegV, "vfsgnjn.vv"),
        (CodeOp::FSqrtV, "vfsqrt.v"),
        (CodeOp::FCvtzsV, "vfcvt.rtz.x.f.v"),
        (CodeOp::ScvtfV, "vfcvt.f.x.v"),
    ] {
        assert!(has(&arm(op, ss), vop), "{} → {vop}", op.mnemonic());
    }

    // Integer three-same and negate.
    for (op, vop) in [
        (CodeOp::AddV, "vadd.vv"),
        (CodeOp::SubV, "vsub.vv"),
        (CodeOp::AndV, "vand.vv"),
        (CodeOp::OrrV, "vor.vv"),
        (CodeOp::EorV, "vxor.vv"),
    ] {
        assert!(has(&arm(op, abc), vop), "{} → {vop}", op.mnemonic());
    }
    assert!(has(&arm(CodeOp::NegV, ss), "vrsub.vx"), "NegV → vrsub.vx");

    // Immediate lane shifts (encodable amount → `.vi`).
    for (op, vop) in [
        (CodeOp::ShlV, "vsll.vi"),
        (CodeOp::SshrV, "vsra.vi"),
        (CodeOp::UshrV, "vsrl.vi"),
    ] {
        let out = arm(op, &[("dst", "v0"), ("src", "v1"), ("shift", "3")]);
        assert!(has(&out, vop), "{} → {vop}", op.mnemonic());
        // An amount the uimm5 cannot encode falls back to the scalar arm.
        assert!(
            rvv_arm(
                op,
                &fl(&[("dst", "v0"), ("src", "v1"), ("shift", "40")]),
                &slots,
                &vregs
            )
            .is_none(),
            "{} shift ≥32 must defer to scalar",
            op.mnemonic()
        );
    }

    // Lane broadcast (GPR source) and both lane extracts (element 0 direct,
    // element 1 via the v31 slidedown scratch).
    assert!(
        has(
            &arm(CodeOp::DupVFromX, &[("dst", "v0"), ("src", "a0")]),
            "vmv.v.x"
        ),
        "DupVFromX → vmv.v.x"
    );
    let umov0 = arm(
        CodeOp::UmovXFromV,
        &[("dst", "a0"), ("src", "v1"), ("index", "0")],
    );
    assert!(
        has(&umov0, "vmv.x.s") && !has(&umov0, "vslidedown.vi"),
        "umov lane 0"
    );
    let umov1 = arm(
        CodeOp::UmovXFromV,
        &[("dst", "a0"), ("src", "v1"), ("index", "1")],
    );
    assert!(
        has(&umov1, "vslidedown.vi") && has(&umov1, "vmv.x.s"),
        "umov lane 1"
    );

    // 128-bit unit-stride load/store from an arbitrary base+offset.
    assert!(
        has(
            &arm(
                CodeOp::LdrQ,
                &[("dst", "v0"), ("base", "a0"), ("offset", "0")]
            ),
            "vle64.v"
        ),
        "LdrQ → vle64.v"
    );
    assert!(
        has(
            &arm(
                CodeOp::StrQ,
                &[("src", "v0"), ("base", "a0"), ("offset", "0")]
            ),
            "vse64.v"
        ),
        "StrQ → vse64.v"
    );

    // The mask bridge: float and integer compares write a `vm*` mask into v0,
    // then `vmv.v.i`/`vmerge.vim` materialize the all-ones/all-zeros lanes.
    for (op, vop) in [
        (CodeOp::FCmGtV, "vmflt.vv"),
        (CodeOp::FCmGeV, "vmfle.vv"),
        (CodeOp::FCmEqV, "vmfeq.vv"),
        (CodeOp::CmGtV, "vmslt.vv"),
        (CodeOp::CmGeV, "vmsle.vv"),
        (CodeOp::CmEqV, "vmseq.vv"),
    ] {
        let out = arm(op, abc);
        assert!(has(&out, vop), "{} → {vop}", op.mnemonic());
        assert!(
            has(&out, "vmerge.vim"),
            "{} materializes lanes",
            op.mnemonic()
        );
    }
    // Compare-against-zero (the `+0.0` register is `vmv.v.i v31,0`).
    for (op, vop) in [
        (CodeOp::FCmGtZeroV, "vmflt.vv"),
        (CodeOp::FCmGeZeroV, "vmfle.vv"),
        (CodeOp::FCmEqZeroV, "vmfeq.vv"),
        (CodeOp::FCmLtZeroV, "vmflt.vv"),
        (CodeOp::FCmLeZeroV, "vmfle.vv"),
    ] {
        let out = arm(op, ss);
        assert!(has(&out, vop), "{} → {vop}", op.mnemonic());
        assert!(
            has(&out, "vmerge.vim"),
            "{} materializes lanes",
            op.mnemonic()
        );
    }

    // Bit-select and bit-insert are the same `vxor`/`vand`/`vxor` bit algebra.
    for op in [CodeOp::BslV, CodeOp::BitV] {
        let out = arm(op, abc);
        assert_eq!(
            out.iter()
                .filter(|i| i.get("vop").as_deref() == Some("vxor.vv"))
                .count(),
            2,
            "{} is two xors around one and",
            op.mnemonic()
        );
        assert!(has(&out, "vand.vv"), "{} uses vand", op.mnemonic());
    }

    // Deferred ops (the rounding/ties-away conversions, wide shifts) always
    // return `None` so the caller emits the scalar arm.
    for op in [
        CodeOp::FRintnV,
        CodeOp::FRintmV,
        CodeOp::FCvtasV,
        CodeOp::AbsV,
    ] {
        assert!(
            rvv_arm(op, &fl(ss), &slots, &vregs).is_none(),
            "{} must defer to the scalar arm",
            op.mnemonic()
        );
    }
}

/// `rvv_lowerable` mirrors `rvv_arm`'s membership so `select_riscv64` can decide
/// run membership without emitting: lowerable ops are accepted, deferred ops and
/// unencodable shift amounts are rejected.
#[test]
fn rvv_lowerable_matches_the_emitter() {
    assert!(rvv_lowerable(
        CodeOp::FAddV,
        &fl(&[("dst", "v0"), ("lhs", "v1"), ("rhs", "v2")])
    ));
    assert!(rvv_lowerable(
        CodeOp::CmEqV,
        &fl(&[("dst", "v0"), ("lhs", "v1"), ("rhs", "v2")])
    ));
    // Shifts are lowerable only when the amount fits the `.vi` uimm5.
    assert!(rvv_lowerable(
        CodeOp::ShlV,
        &fl(&[("dst", "v0"), ("src", "v1"), ("shift", "5")])
    ));
    assert!(!rvv_lowerable(
        CodeOp::UshrV,
        &fl(&[("dst", "v0"), ("src", "v1"), ("shift", "40")])
    ));
    // Deferred ops are never lowerable.
    assert!(!rvv_lowerable(
        CodeOp::FRintnV,
        &fl(&[("dst", "v0"), ("src", "v1")])
    ));
    assert!(!rvv_lowerable(
        CodeOp::AbsV,
        &fl(&[("dst", "v0"), ("src", "v1")])
    ));
}

/// `drop_redundant_reloads` elides a reload of a slot a register already holds,
/// but keeps reloads after the register is overwritten by a compute and keeps a
/// reload of a *different* slot.
#[test]
fn residency_elides_only_a_proven_redundant_reload() {
    let (slots, vregs) = slots_and_vregs();
    // v0 = v1 + v2 ; v0 = v0 + v2  — the second op re-reads v2 (already resident
    // from op 1) and re-reads/rewrites v0; the v2 reload must be elided, but the
    // reload feeding the recomputed v0 chain is preserved.
    let run = vec![
        (
            CodeOp::FAddV,
            fl(&[("dst", "v0"), ("lhs", "v1"), ("rhs", "v2")]),
        ),
        (
            CodeOp::FAddV,
            fl(&[("dst", "v0"), ("lhs", "v0"), ("rhs", "v2")]),
        ),
    ];
    let out = lower_v128_run(&run, &slots, &vregs, 7);
    // v2 lives in v3; its slot (offset 32) is loaded once, not twice.
    let v2_loads = out
        .iter()
        .filter(|i| {
            i.get("vop").as_deref() == Some("vle64.v") && i.get("dst").as_deref() == Some("v3")
        })
        .count();
    assert_eq!(
        v2_loads, 1,
        "the second v2 reload is elided (register-resident)"
    );
    // Unique labels per run.
    assert!(out
        .iter()
        .any(|i| i.op.mnemonic() == "label" && i.get("name").as_deref() == Some("v128_scalar_7")));
}
