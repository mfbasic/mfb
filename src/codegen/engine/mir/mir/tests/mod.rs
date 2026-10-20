use super::*;
use crate::codegen::error::constants::ARENA_STATE_REGISTER;

fn assert_round_trips(original: &[CodeInstruction]) {
    let round_tripped = crate::arch::aarch64::select::select_aarch64(lower_to_mir(original));
    assert_eq!(
        round_tripped.len(),
        original.len(),
        "expand∘fuse changed instruction count"
    );
    for (after, before) in round_tripped.iter().zip(original.iter()) {
        assert_eq!(after.op, before.op);
        assert_eq!(after.fields, before.fields);
    }
}

/// Every `CodeOp` round-trips through `MirOp` unchanged — the property the
/// byte-identical gate rests on. `from_code`/`to_code` are exhaustive
/// matches, so a coverage gap is a compile error rather than a test miss.
#[test]
fn code_op_round_trips_through_mir() {
    // Mirror ops: the MIR op carries the same mnemonic as its `CodeOp`.
    for op in [
        CodeOp::Label,
        CodeOp::Mov,
        CodeOp::Add,
        CodeOp::BranchVs,
        CodeOp::BranchMi,
        CodeOp::Ret,
        CodeOp::LdrD,
        CodeOp::FMaddD, // scalar fused multiply-add — stays a mirror op
        CodeOp::Clz,    // already-neutral exotic int op — stays a mirror
        CodeOp::Rbit,   // (clz/rbit/msub keep their semantic AArch64 name)
        CodeOp::MSub,
    ] {
        assert_eq!(MirOp::from_code(op).to_code(), Some(op));
        assert_eq!(MirOp::from_code(op).mnemonic(), op.mnemonic());
    }
}

/// Renamed ops (plan-00-C Phases 3 & 4) still convert 1:1 to their `CodeOp`
/// — so selection/encoding stay byte-identical — but the MIR mnemonic is the
/// ISA-neutral semantic name, *not* the AArch64 instruction. This is the
/// `mir.md §4` / validation §5 requirement: no `smulh`/`umulh`/`rorv`/`adc`/
/// `rev`/`fcvt*`/`scvtf` mnemonics in the MIR.
#[test]
fn renamed_ops_are_neutral_but_select_back_identically() {
    for (code, neutral) in [
        (CodeOp::SMulH, "mulhi_s"),
        (CodeOp::UMulH, "mulhi_u"),
        (CodeOp::AddCarry, "addc"),
        (CodeOp::SubBorrow, "subc"),
        (CodeOp::Rorv, "rotr"),
        (CodeOp::RorvW, "rotr_w"),
        (CodeOp::RevW, "bswap_w"),
        (CodeOp::RevX, "bswap"),
        (CodeOp::FCvtzsXFromD, "f2i_trunc"),
        (CodeOp::FCvtmsXFromD, "f2i_floor"),
        (CodeOp::FCvtpsXFromD, "f2i_ceil"),
        (CodeOp::FCvtasXFromD, "f2i_nearest"),
        (CodeOp::SCvtfDFromX, "i2f"),
        (CodeOp::FMovDFromX, "fmov_i2f"),
        (CodeOp::FMovXFromD, "fmov_f2i"),
        // §7 machine-y call/syscall vocabulary (plan-00-F): the helpers'
        // `bl`/`blr`/`svc` neutralize so the helper MIR has no AArch64 op.
        (CodeOp::BranchLink, "call"),
        (CodeOp::BranchLinkRegister, "call_indirect"),
        (CodeOp::Svc, "syscall"),
    ] {
        let mir = MirOp::from_code(code);
        // 1:1 selection back to the same CodeOp — byte-identical encoding.
        assert_eq!(mir.to_code(), Some(code));
        // Neutral MIR mnemonic, and it no longer names the AArch64 op.
        assert_eq!(mir.mnemonic(), neutral);
        assert_ne!(mir.mnemonic(), code.mnemonic());
    }
}

/// The whole `v128` SIMD vocabulary (plan-00-E): each NEON `CodeOp` maps 1:1
/// to its `MirOp` (so selection/encoding stay byte-identical) but carries a
/// neutral `v128.*` mnemonic — no `*_v` NEON mnemonic survives in the MIR
/// (`mir.md §6`, validation §5). Covers every renamed NEON op exhaustively.
#[test]
fn v128_ops_are_neutral_but_select_back_identically() {
    let cases = [
        (CodeOp::LdrQ, "v128.load"),
        (CodeOp::StrQ, "v128.store"),
        (CodeOp::FAddV, "v128.fadd"),
        (CodeOp::FSubV, "v128.fsub"),
        (CodeOp::FMulV, "v128.fmul"),
        (CodeOp::FDivV, "v128.fdiv"),
        (CodeOp::FMlaV, "v128.fma"),
        (CodeOp::FMlsV, "v128.fms"),
        (CodeOp::FMinV, "v128.fmin"),
        (CodeOp::FMaxV, "v128.fmax"),
        (CodeOp::FCmGtV, "v128.fcmp_gt"),
        (CodeOp::FCmGeV, "v128.fcmp_ge"),
        (CodeOp::FCmEqV, "v128.fcmp_eq"),
        (CodeOp::FAbsV, "v128.fabs"),
        (CodeOp::FNegV, "v128.fneg"),
        (CodeOp::FSqrtV, "v128.fsqrt"),
        (CodeOp::FRintpV, "v128.fround_ceil"),
        (CodeOp::FRintmV, "v128.fround_floor"),
        (CodeOp::FRintaV, "v128.fround_nearest"),
        (CodeOp::FRintnV, "v128.fround_even"),
        (CodeOp::FRintzV, "v128.fround_trunc"),
        (CodeOp::FCvtzsV, "v128.f2i_trunc"),
        (CodeOp::FCvtasV, "v128.f2i_nearest"),
        (CodeOp::ScvtfV, "v128.i2f"),
        (CodeOp::FCmGtZeroV, "v128.fcmp_gt_zero"),
        (CodeOp::FCmGeZeroV, "v128.fcmp_ge_zero"),
        (CodeOp::FCmEqZeroV, "v128.fcmp_eq_zero"),
        (CodeOp::FCmLtZeroV, "v128.fcmp_lt_zero"),
        (CodeOp::FCmLeZeroV, "v128.fcmp_le_zero"),
        (CodeOp::AddV, "v128.add"),
        (CodeOp::SubV, "v128.sub"),
        (CodeOp::CmGtV, "v128.icmp_gt"),
        (CodeOp::CmGeV, "v128.icmp_ge"),
        (CodeOp::CmEqV, "v128.icmp_eq"),
        (CodeOp::SshlV, "v128.sshl"),
        (CodeOp::UshlV, "v128.ushl"),
        (CodeOp::NegV, "v128.neg"),
        (CodeOp::AbsV, "v128.abs"),
        (CodeOp::Cnt8bV, "v128.cnt8b"),
        (CodeOp::Addv8bV, "v128.addv8b"),
        (CodeOp::AndV, "v128.and"),
        (CodeOp::OrrV, "v128.or"),
        (CodeOp::EorV, "v128.xor"),
        (CodeOp::BslV, "v128.bsl"),
        (CodeOp::BitV, "v128.bit"),
        (CodeOp::ShlV, "v128.shl_imm"),
        (CodeOp::SshrV, "v128.sshr_imm"),
        (CodeOp::UshrV, "v128.ushr_imm"),
        (CodeOp::DupVFromX, "v128.dup_from_gpr"),
        (CodeOp::UmovXFromV, "v128.umov_to_gpr"),
    ];
    for (code, neutral) in cases {
        let mir = MirOp::from_code(code);
        assert_eq!(mir.to_code(), Some(code), "v128 op must select 1:1");
        assert_eq!(mir.mnemonic(), neutral);
        assert!(mir.is_v128(), "{neutral} should be classed as a v128 op");
        // Neutral name: in the `v128.` namespace, never the AArch64 `*_v`.
        assert!(mir.mnemonic().starts_with("v128."));
        assert_ne!(mir.mnemonic(), code.mnemonic());
        assert!(!mir.mnemonic().ends_with("_v"));
    }
}

/// **The `v128` lane-semantics contract** (plan-00-E Phase 4, `mir.md §6`).
///
/// This is the contract the x86_64 (SSE2+FMA3+SSE4.1) and rv64 (scalarized)
/// backends are judged against — the silent-bug surface where a wrong lane
/// op breaks the ≤1-ULP kernels without a crash. On AArch64 it is realized
/// by the (unchanged) NEON encoder; pinning it here makes it an executable
/// acceptance the new backends must reproduce. Each entry fixes the op's
/// **lane shape** and its **edge-case semantics** (the parts that genuinely
/// differ across ISAs); the executable golden vectors are the runtime ULP
/// harness (`tools/math-kernels/runtime_ulp.py`, the transcendental kernels)
/// and the `vector::` / `math::`-array acceptance fixtures, which exercise
/// these ops observably and round-trip unchanged through the MIR.
#[test]
fn v128_lane_semantics_contract() {
    // (op, lane shape, the cross-ISA-significant semantic to reproduce)
    let contract = [
        (CodeOp::LdrQ, "mem128", "load 16 bytes, no lane interpretation"),
        (CodeOp::StrQ, "mem128", "store 16 bytes, no lane interpretation"),
        (CodeOp::FAddV, "2xf64", "IEEE add per lane"),
        (CodeOp::FSubV, "2xf64", "IEEE sub per lane"),
        (CodeOp::FMulV, "2xf64", "IEEE mul per lane"),
        (CodeOp::FDivV, "2xf64", "IEEE div per lane"),
        (CodeOp::FMlaV, "2xf64", "fused dst += lhs*rhs, single rounding (needs x86 FMA3)"),
        (CodeOp::FMlsV, "2xf64", "fused dst -= lhs*rhs, single rounding (needs x86 FMA3)"),
        (CodeOp::FMinV, "2xf64", "NaN-PROPAGATING min (NEON fmin); x86 must match minpd+NaN fixup, NOT bare minpd"),
        (CodeOp::FMaxV, "2xf64", "NaN-PROPAGATING max; x86 maxpd+NaN fixup"),
        (CodeOp::FCmGtV, "2xf64", "lane mask: all-ones if a>b (ordered, NaN→0), else all-zeros"),
        (CodeOp::FCmGeV, "2xf64", "lane mask: all-ones if a>=b (ordered)"),
        (CodeOp::FCmEqV, "2xf64", "lane mask: all-ones if a==b (ordered)"),
        (CodeOp::FAbsV, "2xf64", "clear sign bit per lane"),
        (CodeOp::FNegV, "2xf64", "flip sign bit per lane"),
        (CodeOp::FSqrtV, "2xf64", "IEEE sqrt per lane"),
        (CodeOp::FRintpV, "2xf64", "round to integral, toward +inf"),
        (CodeOp::FRintmV, "2xf64", "round to integral, toward -inf"),
        (CodeOp::FRintaV, "2xf64", "round to integral, nearest, ties AWAY from zero"),
        (CodeOp::FRintnV, "2xf64", "round to integral, nearest, ties to EVEN (x86 roundpd mode 0)"),
        (CodeOp::FRintzV, "2xf64", "round to integral, toward zero (truncate)"),
        (CodeOp::FCvtzsV, "f64->i64 x2", "convert toward zero, saturating to i64"),
        (CodeOp::FCvtasV, "f64->i64 x2", "convert nearest ties-away, saturating to i64"),
        (CodeOp::ScvtfV, "i64->f64 x2", "signed i64 → f64 per lane"),
        (CodeOp::FCmGtZeroV, "2xf64", "lane mask vs +0.0: all-ones if a>0"),
        (CodeOp::FCmGeZeroV, "2xf64", "lane mask vs +0.0: all-ones if a>=0"),
        (CodeOp::FCmEqZeroV, "2xf64", "lane mask vs +0.0: all-ones if a==0 (±0 both match)"),
        (CodeOp::FCmLtZeroV, "2xf64", "lane mask vs +0.0: all-ones if a<0"),
        (CodeOp::FCmLeZeroV, "2xf64", "lane mask vs +0.0: all-ones if a<=0"),
        (CodeOp::AddV, "2xi64", "wrapping add per lane"),
        (CodeOp::SubV, "2xi64", "wrapping sub per lane"),
        (CodeOp::CmGtV, "2xi64", "lane mask: all-ones if signed a>b"),
        (CodeOp::CmGeV, "2xi64", "lane mask: all-ones if signed a>=b"),
        (CodeOp::CmEqV, "2xi64", "lane mask: all-ones if a==b"),
        (CodeOp::SshlV, "2xi64", "signed variable shift: rhs>=0 left, rhs<0 arithmetic right"),
        (CodeOp::UshlV, "2xi64", "unsigned variable shift: rhs>=0 left, rhs<0 logical right"),
        (CodeOp::NegV, "2xi64", "two's-complement negate per lane"),
        (CodeOp::AbsV, "2xi64", "absolute value per lane"),
        (CodeOp::Cnt8bV, "8xi8", "population count per byte (low 64 bits)"),
        (CodeOp::Addv8bV, "8xi8", "horizontal unsigned add of the low 8 bytes into lane 0"),
        (CodeOp::AndV, "16xi8", "bitwise and over all 128 bits"),
        (CodeOp::OrrV, "16xi8", "bitwise or over all 128 bits"),
        (CodeOp::EorV, "16xi8", "bitwise xor over all 128 bits"),
        (CodeOp::BslV, "16xi8", "bit-select: result = (dst & a) | (~dst & b); MASK lives in dst (≠ x86 blendv sign-bit)"),
        (CodeOp::BitV, "16xi8", "bit-insert-if-true: dst = (dst & ~mask) | (lhs & mask), mask in rhs"),
        (CodeOp::ShlV, "2xi64", "shift-left by immediate per lane"),
        (CodeOp::SshrV, "2xi64", "arithmetic shift-right by immediate per lane"),
        (CodeOp::UshrV, "2xi64", "logical shift-right by immediate per lane"),
        (CodeOp::DupVFromX, "i64->2xi64", "broadcast a GPR into both lanes"),
        (CodeOp::UmovXFromV, "lane->i64", "extract one lane into a GPR (zero-extend)"),
    ];
    // Completeness: exactly one contract row per `v128` op, and every row is
    // a v128 op with a `2xf64`/`2xi64`/`16xi8`/conversion/mem lane shape. The
    // count must match the neutrality test's 50-op coverage, so a newly added
    // v128 op cannot slip in without a pinned lane-semantics contract.
    assert_eq!(
        contract.len(),
        50,
        "every v128 op needs a lane-semantics contract row"
    );
    let valid_shapes = [
        "mem128",
        "2xf64",
        "2xi64",
        "16xi8",
        "8xi8",
        "f64->i64 x2",
        "i64->f64 x2",
        "i64->2xi64",
        "lane->i64",
    ];
    for (op, shape, semantics) in contract {
        let mir = MirOp::from_code(op);
        assert!(mir.is_v128(), "{} must be a v128 op", mir.mnemonic());
        assert!(
            valid_shapes.contains(&shape),
            "{}: unknown lane shape {shape}",
            mir.mnemonic()
        );
        assert!(!semantics.is_empty(), "{}: empty contract", mir.mnemonic());
    }
    // No two rows name the same op (the table is a function, total over v128).
    let mut seen = std::collections::HashSet::new();
    for (op, _, _) in contract {
        assert!(
            seen.insert(op.mnemonic()),
            "duplicate contract row for {}",
            op.mnemonic()
        );
    }
}

/// `select_aarch64 ∘ lower_to_mir` is the identity on a non-fusing stream.
#[test]
fn lower_then_select_is_identity() {
    assert_round_trips(&[
        CodeInstruction::new("mov")
            .physical_field("dst", "%v0")
            .physical_field("src", "x0"),
        CodeInstruction::new("add")
            .physical_field("dst", "%v1")
            .physical_field("lhs", "%v0")
            .physical_field("rhs", "%v0"),
        CodeInstruction::new("ret"),
    ]);
}

/// A flag-setter + flag-reading branch fuses to one flagless op and expands
/// back byte-for-byte: integer compare-and-branch (incl. an extra `reason`
/// field), float compare-and-branch, compare-vs-zero, and overflow arith.
#[test]
fn flag_pairs_fuse_and_expand_identically() {
    // cmp_imm; b.eq (carries the debug `reason` field)
    assert_round_trips(&[
        CodeInstruction::new("cmp_imm")
            .physical_field("lhs", "x0")
            .physical_field("rhs", "2"),
        CodeInstruction::new("b.eq")
            .field("target", "if_else_0")
            .field("reason", "ifFalse"),
    ]);
    // cmp; b.lt (register compare)
    assert_round_trips(&[
        CodeInstruction::new("cmp")
            .physical_field("lhs", "%v0")
            .physical_field("rhs", "%v1"),
        CodeInstruction::new("b.lt").field("target", "L0"),
    ]);
    // adds; b.vc (integer add overflow trap)
    assert_round_trips(&[
        CodeInstruction::new("adds")
            .physical_field("dst", "%v6")
            .physical_field("lhs", "%v4")
            .physical_field("rhs", "%v5"),
        CodeInstruction::new("b.vc").field("target", "overflow_ok_2"),
    ]);
    // fcmp_d; b.mi (IEEE float `<`, plan-17) and fcmp_d; b.vs (finiteness)
    assert_round_trips(&[
        CodeInstruction::new("fcmp_d")
            .physical_field("lhs", "%f0")
            .physical_field("rhs", "%f1"),
        CodeInstruction::new("b.mi").field("target", "Lt"),
    ]);
    assert_round_trips(&[
        CodeInstruction::new("fcmp_d")
            .physical_field("lhs", "%f0")
            .physical_field("rhs", "%f1"),
        CodeInstruction::new("b.vs").field("target", "Lnan"),
    ]);
    // fcmp_zero_d; b.ge (compare vs zero)
    assert_round_trips(&[
        CodeInstruction::new("fcmp_zero_d").physical_field("src", "%f2"),
        CodeInstruction::new("b.ge").field("target", "Lge"),
    ]);
}

/// The fused op is genuinely flagless: no `cmp`/`b.cc` remains in the MIR.
#[test]
fn fused_op_carries_operands_and_condition() {
    let mir = lower_to_mir(&[
        CodeInstruction::new("cmp_imm")
            .physical_field("lhs", "x0")
            .physical_field("rhs", "255"),
        CodeInstruction::new("b.hi").field("target", "range_err"),
    ]);
    assert_eq!(mir.len(), 1);
    assert_eq!(mir[0].op, MirOp::BrCcImm);
    assert_eq!(mir[0].op.mnemonic(), "br_cc_imm");
    let get = |k: &str| {
        mir[0]
            .fields
            .iter()
            .find(|(f, _)| *f == k)
            .map(|(_, v)| v.render())
    };
    assert_eq!(get("lhs").as_deref(), Some("x0"));
    assert_eq!(get("rhs").as_deref(), Some("255"));
    assert_eq!(get("cond").as_deref(), Some("b.hi"));
    assert_eq!(get("target").as_deref(), Some("range_err"));
}

/// The macOS syscall error idiom `svc; b.<carry>` fuses into the flagless
/// `syscall_br` and expands back byte-for-byte (plan-00-F) — the last
/// flag-reading branch in the helper MIR. A `svc` *not* followed by a flag
/// branch (e.g. the exit syscall before `branch_self`) stays the neutral
/// `syscall` op.
#[test]
fn syscall_carry_check_fuses_and_expands_identically() {
    // svc; b.lo (carry-clear → error path) fuses to one flagless op.
    let checked = [
        CodeInstruction::new("svc"),
        CodeInstruction::new("b.lo").field("target", "encoding_error"),
    ];
    let mir = lower_to_mir(&checked);
    assert_eq!(mir.len(), 1);
    assert_eq!(mir[0].op, MirOp::SyscallBr);
    assert_eq!(mir[0].op.mnemonic(), "syscall_br");
    let get = |k: &str| {
        mir[0]
            .fields
            .iter()
            .find(|(f, _)| *f == k)
            .map(|(_, v)| v.render())
    };
    assert_eq!(get("cond").as_deref(), Some("b.lo"));
    assert_eq!(get("target").as_deref(), Some("encoding_error"));
    // No `svc`/`b.lo` mnemonic survives — the MIR is flagless.
    assert!(!mir
        .iter()
        .any(|m| matches!(m.op, MirOp::Syscall | MirOp::BranchLo)));
    assert_round_trips(&checked);

    // A bare syscall (no following flag branch) stays the neutral `syscall`.
    let bare = [
        CodeInstruction::new("svc"),
        CodeInstruction::new("branch_self"),
    ];
    let mir = lower_to_mir(&bare);
    assert_eq!(mir.len(), 2);
    assert_eq!(mir[0].op, MirOp::Syscall);
    assert_eq!(mir[0].op.mnemonic(), "syscall");
    assert_round_trips(&bare);
}

/// The explicit-carry 128-bit add chain (plan-00-G §4): each limb is a single
/// `add_carry` op whose carry-in/out are *values*, so nothing fuses and the
/// carry survives register allocation. The op neutral-renames to `addc` and
/// round-trips 1:1. A plain `adds` not followed by a flag-branch stays a
/// mirror op (it is not mistaken for an overflow fusion).
#[test]
fn explicit_carry_chain_round_trips() {
    let original = [
        // lo = a_lo + b_lo, carry → x15
        CodeInstruction::new("add_carry")
            .physical_field("dst", "x9")
            .physical_field("carry_out", "x15")
            .physical_field("lhs", "x13")
            .physical_field("rhs", "x11")
            .physical_field("carry_in", "xzr"),
        // hi = a_hi + b_hi + x15, carry-out discarded
        CodeInstruction::new("add_carry")
            .physical_field("dst", "x10")
            .physical_field("carry_out", "xzr")
            .physical_field("lhs", "x14")
            .physical_field("rhs", "x12")
            .physical_field("carry_in", "x15"),
    ];
    let mir = lower_to_mir(&original);
    assert_eq!(mir.len(), 2);
    assert_eq!(mir[0].op, MirOp::AddC);
    assert_eq!(mir[0].op.mnemonic(), "addc");
    assert_round_trips(&original);

    // A lone `adds` (no following flag-branch) stays a mirror op.
    let lone_adds = [
        CodeInstruction::new("adds")
            .physical_field("dst", "x9")
            .physical_field("lhs", "x9")
            .physical_field("rhs", "x1"),
        CodeInstruction::new("mov")
            .physical_field("dst", "x10")
            .physical_field("src", "x9"),
    ];
    let mir = lower_to_mir(&lone_adds);
    assert_eq!(mir[0].op, MirOp::Adds);
    assert_round_trips(&lone_adds);
}

/// A 3-way `cmp; b.lo; b.hi` (the string-ordering pattern) becomes two
/// flagless ops: the first owns the compare, the second shares it. Both are
/// self-contained (carry operands), and they expand back to the single
/// shared `cmp` byte-for-byte.
#[test]
fn multi_branch_compare_fuses_with_share() {
    let original = [
        CodeInstruction::new("cmp")
            .physical_field("lhs", "x16")
            .physical_field("rhs", "x11"),
        CodeInstruction::new("b.lo").field("target", "less"),
        CodeInstruction::new("b.hi").field("target", "greater"),
    ];
    let mir = lower_to_mir(&original);
    assert_eq!(mir.len(), 2);
    assert_eq!(mir[0].op, MirOp::BrCc); // owns the compare
    assert_eq!(mir[1].op, MirOp::BrCc); // shares it — still flagless
    let shared = |m: &MirInstruction| m.fields.iter().any(|(k, _)| *k == "share");
    assert!(!shared(&mir[0]));
    assert!(shared(&mir[1]));
    // the shared branch still carries the compare operands (self-contained)
    fn get(m: &MirInstruction, k: &str) -> Option<String> {
        m.fields
            .iter()
            .find(|(f, _)| *f == k)
            .map(|(_, v)| v.render())
    }
    assert_eq!(get(&mir[1], "lhs").as_deref(), Some("x16"));
    assert_eq!(get(&mir[1], "rhs").as_deref(), Some("x11"));
    assert_eq!(get(&mir[1], "cond").as_deref(), Some("b.hi"));
    assert_round_trips(&original);
}

/// An `adrp; add_pageoff` page pair fuses into one neutral `addr_of` and
/// expands back to the exact same two instructions (plan-00-C Phase 1). The
/// fused op carries only `[dst, symbol]`; the expansion reconstructs
/// `add_pageoff`'s `src == dst`.
#[test]
fn addr_of_fuses_and_expands_identically() {
    let original = [
        CodeInstruction::new("adrp")
            .physical_field("dst", "x9")
            .field("symbol", "str.0"),
        CodeInstruction::new("add_pageoff")
            .physical_field("dst", "x9")
            .physical_field("src", "x9")
            .field("symbol", "str.0"),
    ];
    let mir = lower_to_mir(&original);
    assert_eq!(mir.len(), 1);
    assert_eq!(mir[0].op, MirOp::AddrOf);
    assert_eq!(mir[0].op.mnemonic(), "addr_of");
    let get = |k: &str| {
        mir[0]
            .fields
            .iter()
            .find(|(f, _)| *f == k)
            .map(|(_, v)| v.render())
    };
    assert_eq!(get("dst").as_deref(), Some("x9"));
    assert_eq!(get("symbol").as_deref(), Some("str.0"));
    // No `adrp`/`add_pageoff` mnemonic survives in the MIR (validation §5).
    assert!(!mir
        .iter()
        .any(|m| matches!(m.op, MirOp::Adrp | MirOp::AddPageOff)));
    assert_round_trips(&original);
}

/// A lone `adrp` (no following `add_pageoff`), or a pair whose registers do
/// not line up, is *not* fused — it stays a mirror op so the stream still
/// round-trips. Guards against an over-eager fusion mistaking an unrelated
/// page load for the address-of sequence.
#[test]
fn unpaired_or_mismatched_adrp_is_not_fused() {
    // Lone adrp followed by an unrelated op.
    let lone = [
        CodeInstruction::new("adrp")
            .physical_field("dst", "x9")
            .field("symbol", "g"),
        CodeInstruction::new("ret"),
    ];
    let mir = lower_to_mir(&lone);
    assert_eq!(mir[0].op, MirOp::Adrp);
    assert_round_trips(&lone);
    // add_pageoff on a *different* register than the adrp — not an addr_of.
    let mismatch = [
        CodeInstruction::new("adrp")
            .physical_field("dst", "x9")
            .field("symbol", "g"),
        CodeInstruction::new("add_pageoff")
            .physical_field("dst", "x10")
            .physical_field("src", "x10")
            .field("symbol", "g"),
    ];
    let mir = lower_to_mir(&mismatch);
    assert_eq!(mir.len(), 2);
    assert_eq!(mir[0].op, MirOp::Adrp);
    assert_eq!(mir[1].op, MirOp::AddPageOff);
    assert_round_trips(&mismatch);
}

/// The round-trip identity, in miniature: a 39-fixture sweep with at least
/// one instruction from **every** builder op family — moves & immediates,
/// the universal ALU, the neutral-renamed exotic integer ops, the structural
/// `addr_of` pair, every load/store width (one against the abstract
/// `arena_base`, plan-00-D), scalar float arith + the renamed float↔int
/// conversions & bit-reinterprets, the NEON `v128` ops, the machine-y
/// call/syscall vocabulary (plan-00-F), and the fused flagless control-flow
/// ops. `select_aarch64 ∘ lower_to_mir` must be the identity on the whole
/// stream — the property that lets the MIR be the sole code path (plan-00-G)
/// without perturbing the AArch64 output.
#[test]
fn round_trip_sweep_over_every_op_family() {
    let fixtures: [CodeInstruction; 39] = [
        // — moves & immediates —
        CodeInstruction::new("mov")
            .physical_field("dst", "%v0")
            .physical_field("src", "x1"),
        CodeInstruction::new("mov_imm")
            .physical_field("dst", "%v1")
            .field("value", "4294967296"),
        // — universal ALU (incl. the immediate forms that keep small imms) —
        CodeInstruction::new("add")
            .physical_field("dst", "%v2")
            .physical_field("lhs", "%v0")
            .physical_field("rhs", "%v1"),
        CodeInstruction::new("sub")
            .physical_field("dst", "%v3")
            .physical_field("lhs", "%v2")
            .physical_field("rhs", "%v0"),
        CodeInstruction::new("mul")
            .physical_field("dst", "%v4")
            .physical_field("lhs", "%v2")
            .physical_field("rhs", "%v3"),
        CodeInstruction::new("and")
            .physical_field("dst", "%v5")
            .physical_field("lhs", "%v4")
            .physical_field("rhs", "%v0"),
        CodeInstruction::new("orr")
            .physical_field("dst", "%v6")
            .physical_field("lhs", "%v5")
            .physical_field("rhs", "%v1"),
        CodeInstruction::new("eor")
            .physical_field("dst", "%v7")
            .physical_field("lhs", "%v6")
            .physical_field("rhs", "%v2"),
        CodeInstruction::new("mvn")
            .physical_field("dst", "%v8")
            .physical_field("src", "%v7"),
        CodeInstruction::new("sdiv")
            .physical_field("dst", "%v9")
            .physical_field("lhs", "%v8")
            .physical_field("rhs", "%v0"),
        CodeInstruction::new("udiv")
            .physical_field("dst", "%v10")
            .physical_field("lhs", "%v9")
            .physical_field("rhs", "%v1"),
        CodeInstruction::new("lslv")
            .physical_field("dst", "%v11")
            .physical_field("lhs", "%v10")
            .physical_field("rhs", "%v0"),
        CodeInstruction::new("add_imm")
            .physical_field("dst", "%v12")
            .physical_field("src", "%v11")
            .field("imm", "8"),
        CodeInstruction::new("lsl_imm")
            .physical_field("dst", "%v13")
            .physical_field("src", "%v12")
            .field("shift", "3"),
        // — neutral-renamed "exotic" integer ops (Phase 3) —
        CodeInstruction::new("smulh")
            .physical_field("dst", "%v14")
            .physical_field("lhs", "%v0")
            .physical_field("rhs", "%v1"),
        CodeInstruction::new("umulh")
            .physical_field("dst", "%v15")
            .physical_field("lhs", "%v0")
            .physical_field("rhs", "%v1"),
        CodeInstruction::new("add_carry")
            .physical_field("dst", "%v16")
            .physical_field("carry_out", "%v26")
            .physical_field("lhs", "%v14")
            .physical_field("rhs", "%v15")
            .physical_field("carry_in", "xzr"),
        CodeInstruction::new("rorv")
            .physical_field("dst", "%v17")
            .physical_field("lhs", "%v16")
            .physical_field("rhs", "%v0"),
        CodeInstruction::new("rev_x")
            .physical_field("dst", "%v18")
            .physical_field("src", "%v17"),
        CodeInstruction::new("clz")
            .physical_field("dst", "%v19")
            .physical_field("src", "%v18"),
        CodeInstruction::new("msub")
            .physical_field("dst", "%v20")
            .physical_field("lhs", "%v0")
            .physical_field("rhs", "%v1")
            .physical_field("minuend", "%v2"),
        // — structural addr_of page pair (Phase 1): fuses to one op —
        CodeInstruction::new("adrp")
            .physical_field("dst", "%v21")
            .field("symbol", "pool"),
        CodeInstruction::new("add_pageoff")
            .physical_field("dst", "%v21")
            .physical_field("src", "%v21")
            .field("symbol", "pool"),
        // — loads/stores, every width; the u8 store addresses the abstract
        //   arena base (the pinned x19), exercising the plan-00-D rename —
        CodeInstruction::new("ldr_u64")
            .physical_field("dst", "%v22")
            .physical_field("base", "%v21")
            .field("offset", "0"),
        CodeInstruction::new("str_u8")
            .physical_field("src", "%v22")
            .physical_field("base", crate::arch::aarch64::regmodel::ARENA_BASE_REGISTER)
            .field("offset", "16"),
        CodeInstruction::new("ldr_d")
            .physical_field("dst", "%f0")
            .physical_field("base", "%v21")
            .field("offset", "24"),
        // — scalar float arith + renamed conversions / bit-reinterprets (Phase 4) —
        CodeInstruction::new("fadd_d")
            .physical_field("dst", "%f1")
            .physical_field("lhs", "%f0")
            .physical_field("rhs", "%f0"),
        CodeInstruction::new("fmadd_d")
            .physical_field("dst", "%f2")
            .physical_field("lhs", "%f1")
            .physical_field("rhs", "%f0")
            .field("acc", "%f1"),
        CodeInstruction::new("scvtf_d_from_x")
            .physical_field("dst", "%f3")
            .physical_field("src", "%v0"),
        CodeInstruction::new("fcvtzs_x_from_d")
            .physical_field("dst", "%v23")
            .physical_field("src", "%f3"),
        CodeInstruction::new("fcvtms_x_from_d")
            .physical_field("dst", "%v24")
            .physical_field("src", "%f3"),
        CodeInstruction::new("fmov_d_from_x")
            .physical_field("dst", "%f4")
            .physical_field("src", "%v0"),
        CodeInstruction::new("fmov_x_from_d")
            .physical_field("dst", "%v25")
            .physical_field("src", "%f4"),
        // — NEON `v128` op (plan-00-E): the `fmla_v` MAC neutralizes to
        //   `v128.fma`; no `*_v` NEON mnemonic survives —
        CodeInstruction::new("fmla_v")
            .physical_field("dst", "%f5")
            .physical_field("lhs", "%f3")
            .physical_field("rhs", "%f4"),
        // — machine-y call/syscall vocabulary (plan-00-F): the helpers'
        //   `bl`/`blr`/`svc` neutralize to `call`/`call_indirect`/`syscall` —
        CodeInstruction::new("bl").field("target", "_mfb_arena_alloc"),
        CodeInstruction::new("blr").physical_field("register", "x16"),
        CodeInstruction::new("svc"),
        // — fused flagless control flow (Phases B/C neighbours) —
        CodeInstruction::new("cmp")
            .physical_field("lhs", "%v0")
            .physical_field("rhs", "%v1"),
        CodeInstruction::new("b.lt").field("target", "Lbody"),
    ];
    assert_round_trips(&fixtures);

    // And the resulting MIR names nothing AArch64-specific from the
    // neutralized families (validation §5): the exotic ints, the scalar
    // float↔int conversions, the `adrp` page pair, and the NEON `*_v` tail.
    let mir = lower_to_mir(&fixtures);
    let banned = [
        "adrp",
        "add_pageoff",
        "smulh",
        "umulh",
        "add_carry",
        "sub_borrow",
        "rorv",
        "rorv_w",
        "rev_w",
        "rev_x",
        "fcvtzs_x_from_d",
        "fcvtms_x_from_d",
        "fcvtps_x_from_d",
        "fcvtas_x_from_d",
        "scvtf_d_from_x",
        "fmov_d_from_x",
        "fmov_x_from_d",
        // NEON-tail mnemonics — none may appear (now `v128.*`, plan-00-E).
        "ldr_q",
        "str_q",
        "fadd_v",
        "fmla_v",
        "fcmgt_v",
        "frintn_v",
        "fcvtzs_v",
        "scvtf_v",
        "add_v",
        "sshl_v",
        "shl_v",
        "bsl_v",
        "bit_v",
        "dup_v_from_x",
        "umov_x_from_v",
        // Machine-y mnemonics — now `call`/`call_indirect`/`syscall` (plan-00-F).
        "bl",
        "blr",
        "svc",
    ];
    let mut saw_v128 = false;
    let mut saw_call = false;
    let mut saw_syscall = false;
    for instruction in &mir {
        let mnemonic = instruction.op.mnemonic();
        assert!(
            !banned.contains(&mnemonic),
            "MIR still names an AArch64-specific op: {mnemonic}"
        );
        if mnemonic.starts_with("v128.") {
            saw_v128 = true;
        }
        saw_call |= mnemonic == "call" || mnemonic == "call_indirect";
        saw_syscall |= mnemonic == "syscall";
    }
    assert!(saw_v128, "the NEON fixture did not lower to a `v128.*` op");
    assert!(
        saw_call,
        "the bl/blr fixtures did not lower to `call`/`call_indirect`"
    );
    assert!(saw_syscall, "the svc fixture did not lower to `syscall`");

    // The pinned arena register never appears in the MIR: the `str_u8`
    // fixture's `base` is the abstract `arena_base`, not `x19` (plan-00-D
    // §2). Selection realizes it back to `x19` byte-for-byte (proved by the
    // `assert_round_trips` above).
    let realization = arena_base_realization();
    let mut saw_arena_base = false;
    for instruction in &mir {
        for (_, value) in &instruction.fields {
            assert_ne!(
                value, realization,
                "MIR still names the pinned arena register {realization}"
            );
            if value == ARENA_BASE {
                saw_arena_base = true;
            }
        }
    }
    assert!(
        saw_arena_base,
        "the arena-base fixture did not lower to `arena_base`"
    );
}

/// `arena_base` is the identity through the MIR: an arena load/store names
/// `arena_base` in the neutral stream and selects back to the pinned `x19`
/// byte-for-byte (plan-00-D §2). Non-arena register operands are untouched.
#[test]
fn arena_base_renames_and_realizes_identically() {
    let realization = arena_base_realization();
    let original = [
        // load the free-list head from the arena, store it back via a vreg.
        CodeInstruction::new("ldr_u64")
            .physical_field("dst", "%v0")
            .physical_field("base", realization)
            .field("offset", "48"),
        CodeInstruction::new("str_u64")
            .physical_field("src", "%v0")
            .physical_field("base", realization)
            .field("offset", "48"),
    ];
    let mir = lower_to_mir(&original);
    // Both base fields are renamed to the neutral `arena_base`; the vreg is not.
    let base = |i: usize| {
        mir[i]
            .fields
            .iter()
            .find(|(k, _)| *k == "base")
            .map(|(_, v)| v.render())
    };
    assert_eq!(base(0).as_deref(), Some(ARENA_BASE));
    assert_eq!(base(1).as_deref(), Some(ARENA_BASE));
    assert!(!mir
        .iter()
        .any(|m| m.fields.iter().any(|(_, v)| v == realization)));
    // …and selection restores the pinned register exactly.
    assert_round_trips(&original);
}

/// plan-34-A: the three invariant registers — zero, the link register, and the
/// arena base — are named by neutral tokens in shared lowering, never by an
/// AArch64 register number. This guards the rename against reintroduction of a
/// physical `"x19"`/`"x30"`/`"x31"` (the seed of plan-34-C's stream invariant):
/// a stray physical name that reached `x86_64/select.rs::map_scratch_register`
/// would be realized as the wrong register (`x19` → `rbp`, not the arena base
/// `r15`) — a silent miscompile.
#[test]
fn invariant_registers_are_neutral_tokens() {
    use crate::target::shared::abi;
    // The tokens are neutral, never an AArch64 register number.
    assert_eq!(
        abi::ZERO,
        crate::target::shared::abi::Reg::test_named("xzr")
    );
    assert_eq!(abi::LR, crate::target::shared::abi::Reg::test_named("lr"));
    assert_eq!(abi::ARENA.to_string(), ARENA_BASE);
    for token in [abi::ZERO, abi::LR, abi::ARENA] {
        assert!(
            !matches!(token.to_string().as_str(), "x19" | "x30" | "x31"),
            "invariant-register token must not be an AArch64 register number: {token}"
        );
    }
    // A shared stream that names zero (store source + negate), the link
    // register (frame save), and the arena base (address base) with the abi
    // helpers carries no physical x19/x30/x31 into the MIR — `arena_base` is
    // only realized to `x19` later, in `select_aarch64`.
    let stream = [
        abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, 0),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        CodeInstruction::new("sub")
            .physical_field("dst", "x9")
            .field("lhs", abi::ZERO)
            .physical_field("rhs", "x9"),
    ];
    let mir = lower_to_mir(&stream);
    for inst in &mir {
        for (_, value) in &inst.fields {
            assert!(
                !matches!(value.render().as_str(), "x19" | "x30" | "x31"),
                "MIR field leaked a physical invariant register: {value}"
            );
        }
    }
}

/// Relocation intents are neutral: a `CodeRelocation` carries a
/// [`RelocIntent`], its `-mir` name never an AArch64 reloc kind, and the
/// AArch64 table realizes it back to today's `branch26`/`page21`/`pageoff12`
/// (plan-00-D §1).
#[test]
fn reloc_intents_are_neutral_and_realize_to_aarch64_kinds() {
    use crate::arch::aarch64::reloc::reloc_kind;
    for (intent, neutral, concrete) in [
        (RelocIntent::Call, "call", "branch26"),
        (RelocIntent::DataAddrHi, "data_addr_hi", "page21"),
        (RelocIntent::DataAddrLo, "data_addr_lo", "pageoff12"),
        (RelocIntent::GotLoadHi, "got_load_hi", "page21"),
        (RelocIntent::GotLoadLo, "got_load_lo", "pageoff12"),
    ] {
        assert_eq!(intent.name(), neutral);
        assert_ne!(
            intent.name(),
            concrete,
            "the -mir name must not be an AArch64 kind"
        );
        assert_eq!(reloc_kind(intent), concrete);
    }
}
