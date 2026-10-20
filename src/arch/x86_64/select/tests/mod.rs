use super::*;
use crate::codegen::engine::mir::lower_to_mir;

/// Build one aarch64-form `CodeInstruction`.
fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    let mut ins = CodeInstruction::new(op);
    for (k, v) in fields {
        ins = ins.physical_field(k, v);
    }
    ins
}

/// Select a stream from aarch64-form instructions.
fn sel(instructions: &[CodeInstruction]) -> Vec<CodeInstruction> {
    select_x86(lower_to_mir(instructions), X86Abi::SysV)
}

/// Every field value in the selected stream, flattened.
fn values(out: &[CodeInstruction]) -> Vec<String> {
    out.iter()
        .flat_map(|inst| inst.fields.iter().map(|(_, v)| v.render()))
        .collect()
}

#[test]
fn addr_of_becomes_lea_and_pageoff_drops() {
    // adrp; add_pageoff (same reg + symbol) fuses to addr_of, selected as Adrp.
    let out = sel(&[
        ci("adrp", &[("dst", "x9"), ("symbol", "g")]),
        ci(
            "add_pageoff",
            &[("dst", "x9"), ("src", "x9"), ("symbol", "g")],
        ),
    ]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].op, CodeOp::Adrp);
    // x9 scratch mapped to an x86 GPR.
    assert!(out[0].fields.iter().any(|(k, v)| *k == "dst" && v != "x9"));
}

#[test]
fn sp_zero_and_fp_register_mapping() {
    let out = sel(&[
        ci("mov", &[("dst", "x9"), ("src", "sp")]),
        ci("mov", &[("dst", "x10"), ("src", "x31")]),
        ci("fmov_d_from_d", &[("dst", "d0"), ("src", "d3")]),
        ci("fadd_v", &[("dst", "v1"), ("lhs", "v2"), ("rhs", "q3")]),
        ci("ret", &[]),
    ]);
    let vals = values(&out);
    assert!(vals.contains(&"rsp".to_string()));
    // The legacy `x31` zero spelling now maps to the neutral zero token (which
    // the encoder emits as an immediate zero), not the freed r14.
    assert!(vals.contains(&"xzr".to_string()));
    assert!(vals.contains(&"xmm0".to_string()));
    assert!(vals.contains(&"xmm3".to_string()));
    assert!(vals.contains(&"xmm1".to_string()));
    assert!(vals.contains(&"xmm2".to_string()));
}

#[test]
fn scratch_register_pool_wraps() {
    // High scratch registers land on the pool; x20 → rbx, x27 → r12, x28 → r13.
    assert_eq!(map_scratch_register(20), "rbx");
    assert_eq!(map_scratch_register(27), "r12");
    assert_eq!(map_scratch_register(28), "r13");
    assert_eq!(map_scratch_register(19), "rbp");
    assert_eq!(map_scratch_register(9), "rbx");
}

#[test]
fn x30_link_register_is_dropped() {
    // A frame save of x30 (link register) is removed entirely.
    let out = sel(&[
        ci(
            "str_u64",
            &[("src", "x30"), ("base", "sp"), ("offset", "0")],
        ),
        ci("ret", &[]),
    ]);
    assert!(!values(&out).iter().any(|v| v == "x30"));
}

#[test]
fn float_compare_branch_rewrites() {
    // Each fcmp_d + b.cc pair rewrites into the x86 IEEE branch sequence.
    // b.gt → ja ; b.ge → jae (single branch).
    let out = sel(&[
        ci("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
        ci("b.gt", &[("target", "L")]),
        ci("ret", &[]),
    ]);
    assert!(out.iter().any(|i| i.op.mnemonic() == "x86.ja"));

    for (cond, expect) in [
        ("b.ge", "x86.jae"),
        ("b.lt", "x86.jb"),
        ("b.le", "x86.jbe"),
        ("b.vs", "x86.jp"),
        ("b.vc", "x86.jnp"),
    ] {
        let out = sel(&[
            ci("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
            ci(cond, &[("target", "L")]),
            ci("ret", &[]),
        ]);
        assert!(
            out.iter().any(|i| i.op.mnemonic() == expect),
            "cond {cond} should emit {expect}"
        );
    }
}

#[test]
fn float_compare_ordered_only_and_multi_branch() {
    // b.mi / b.ls / b.eq emit `jp skip; jcc target; skip:` (3 instructions).
    for (cond, cc) in [("b.mi", "x86.jb"), ("b.ls", "x86.jbe"), ("b.eq", "x86.je")] {
        let out = sel(&[
            ci("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
            ci(cond, &[("target", "L")]),
            ci("ret", &[]),
        ]);
        assert!(out.iter().any(|i| i.op.mnemonic() == "x86.jp"));
        assert!(out.iter().any(|i| i.op.mnemonic() == cc));
        assert!(out.iter().any(|i| i.op.mnemonic() == "label"));
    }
    // b.ne → jp target; jne target ; b.hi → jp target; ja target.
    let ne = sel(&[
        ci("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
        ci("b.ne", &[("target", "L")]),
        ci("ret", &[]),
    ]);
    assert!(ne.iter().any(|i| i.op.mnemonic() == "x86.jne"));
    let hi = sel(&[
        ci("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
        ci("b.hi", &[("target", "L")]),
        ci("ret", &[]),
    ]);
    assert!(hi.iter().any(|i| i.op.mnemonic() == "x86.ja"));
    // b.lo → ordered-only jb.
    let lo = sel(&[
        ci("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
        ci("b.lo", &[("target", "L")]),
        ci("ret", &[]),
    ]);
    assert!(lo.iter().any(|i| i.op.mnemonic() == "x86.jb"));
}

#[test]
fn ordered_only_skip_labels_are_unique_per_branch_site() {
    // bug-15: two ordered-only float branches to the SAME target (e.g.
    // `IF a < b OR c < d THEN GOTO L`) once emitted two labels both named
    // `L__x86ford`. The encoder's label map is last-writer-wins, so the first
    // `jp` resolved to the second label and a NaN first operand skipped the
    // second comparison entirely.
    let out = sel(&[
        ci("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
        ci("b.mi", &[("target", "L")]),
        ci("fcmp_d", &[("lhs", "d2"), ("rhs", "d3")]),
        ci("b.mi", &[("target", "L")]),
        ci("label", &[("name", "L")]),
        ci("ret", &[]),
    ]);
    let labels: Vec<String> = out
        .iter()
        .filter(|i| i.op == CodeOp::Label)
        .map(|i| i.fields[0].1.render())
        .collect();
    assert_eq!(
        labels.len(),
        3,
        "two skip labels + the shared target: {labels:?}"
    );
    let skips: Vec<String> = labels
        .iter()
        .filter(|n| n.contains("__x86ford"))
        .cloned()
        .collect();
    assert_eq!(skips.len(), 2);
    assert_ne!(skips[0], skips[1], "skip labels collide: {skips:?}");
    // Each `jp` targets its own skip label, which sits right after its `jb`.
    let jps: Vec<String> = out
        .iter()
        .filter(|i| i.op.mnemonic() == "x86.jp")
        .map(|i| i.fields[0].1.render())
        .collect();
    assert_eq!(jps, skips);
}

#[test]
fn fcmp_zero_branch_rewrite() {
    // A compare-against-zero fused branch also takes the float remap.
    let out = sel(&[
        ci("fcmp_zero_d", &[("src", "d0")]),
        ci("b.mi", &[("target", "L")]),
        ci("ret", &[]),
    ]);
    assert_eq!(out[0].op, CodeOp::FCmpZeroD);
    assert!(out.iter().any(|i| i.op.mnemonic().starts_with("x86.")));
}

#[test]
#[should_panic(expected = "unmapped x86 float-compare branch condition")]
fn float_branch_unmapped_condition_panics() {
    // A non-flag condition reaching x86_float_branch panics.
    x86_float_branch("b.pl", "L", 0);
}

#[test]
fn integer_compare_branch_not_remapped() {
    // An integer cmp + branch keeps the standard b.cc → jcc path (no x86.*).
    let out = sel(&[
        ci("cmp", &[("lhs", "x9"), ("rhs", "x10")]),
        ci("b.eq", &[("target", "L")]),
        ci("ret", &[]),
    ]);
    assert!(out.iter().any(|i| i.op == CodeOp::BranchEq));
    assert!(!out.iter().any(|i| i.op.mnemonic().starts_with("x86.")));
}

#[test]
fn arena_base_realizes_to_r15() {
    // The AArch64 arena-base realization register, once lowered to the neutral
    // `arena_base` and selected, becomes the x86 pin r15.
    let realization = crate::codegen::engine::mir::arena_base_realization();
    let out = sel(&[
        ci(
            "ldr_u64",
            &[("dst", "x9"), ("base", realization), ("offset", "0")],
        ),
        ci("ret", &[]),
    ]);
    assert!(values(&out).iter().any(|v| v == "r15"));
}

#[test]
fn realize_abi_operand_maps_to_aligned_registers() {
    // plan-85-A §2 aligned SysV realization: MFB args/results and C args all
    // draw from the aligned call-argument bank; %retC keeps rax:rdx; the
    // syscall file is unchanged.
    for (n, reg) in ["rdi", "rsi", "rdx", "rcx", "r8", "r9", "rax", "rbp"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            realize_abi_operand(AbiConvention::Mfb, AbiRole::Arg, n, X86Abi::SysV),
            reg
        );
        assert_eq!(
            realize_abi_operand(AbiConvention::C, AbiRole::Arg, n, X86Abi::SysV),
            reg
        );
    }
    // %retMFB is the byte-CHANGING choice: aligned [rdi,rsi,rdx,rcx], NOT the
    // old rax-first result bank [rax,rdx,rcx,rsi].
    for (n, reg) in ["rdi", "rsi", "rdx", "rcx"].into_iter().enumerate() {
        assert_eq!(
            realize_abi_operand(AbiConvention::Mfb, AbiRole::Ret, n, X86Abi::SysV),
            reg
        );
    }
    // %retC keeps the genuine C return bank rax:rdx.
    assert_eq!(
        realize_abi_operand(AbiConvention::C, AbiRole::Ret, 0, X86Abi::SysV),
        "rax"
    );
    assert_eq!(
        realize_abi_operand(AbiConvention::C, AbiRole::Ret, 1, X86Abi::SysV),
        "rdx"
    );
    // Syscalls.
    for (n, reg) in ["rdi", "rsi", "rdx", "r10", "r8", "r9"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            realize_abi_operand(AbiConvention::Sys, AbiRole::Arg, n, X86Abi::SysV),
            reg
        );
    }
    assert_eq!(
        realize_abi_operand(AbiConvention::Sys, AbiRole::Ret, 0, X86Abi::SysV),
        "rax"
    );
    // Win64: MFB arg AND result share the aligned Win64 call bank
    // (rcx,rdx,r8,r9); a variable-shifted result landing on rcx is handled by
    // the encoder, not by de-aligning. %retC still rax:rdx.
    for (n, reg) in ["rcx", "rdx", "r8", "r9"].into_iter().enumerate() {
        assert_eq!(
            realize_abi_operand(AbiConvention::Mfb, AbiRole::Arg, n, X86Abi::Win64),
            reg
        );
        assert_eq!(
            realize_abi_operand(AbiConvention::Mfb, AbiRole::Ret, n, X86Abi::Win64),
            reg
        );
    }
    assert_eq!(
        realize_abi_operand(AbiConvention::C, AbiRole::Ret, 0, X86Abi::Win64),
        "rax"
    );
    assert_eq!(
        realize_abi_operand(AbiConvention::C, AbiRole::Ret, 1, X86Abi::Win64),
        "rdx"
    );
}

#[test]
fn explicit_abi_token_realizes_to_aligned_register() {
    use crate::target::shared::abi;
    // A typed `Operand::Abi` is realized directly by the plan-85-A seam to its
    // ALIGNED register. Proof: `%retMFB0` realizes to `rdi` (aligned
    // CALL_ARGS[0]); the old rax-first result bank would give `rax`. If `rax`
    // appeared, the aligned realization was not applied.
    let inst = CodeInstruction::new("mov")
        .physical_field("dst", abi::mfb_return(0))
        .physical_field("src", abi::mfb_arg(1));
    let out = sel(&[inst]);
    let vals = values(&out);
    assert!(
        vals.iter().any(|v| v == "rdi"),
        "%retMFB0 must realize aligned to rdi, got {vals:?}"
    );
    assert!(
        vals.iter().any(|v| v == "rsi"),
        "%argMFB1 must realize aligned to rsi, got {vals:?}"
    );
    assert!(
        !vals.iter().any(|v| v == "rax"),
        "no rax expected — the old rax-first result bank must not appear, got {vals:?}"
    );
}
