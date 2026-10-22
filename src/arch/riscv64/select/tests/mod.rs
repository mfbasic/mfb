use super::*;
use crate::codegen::engine::mir::lower_to_mir;

mod v128_tail;

fn build(mnemonic: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    ci(mnemonic, fields)
}

fn sel(instructions: &[CodeInstruction]) -> Vec<CodeInstruction> {
    select_riscv64(lower_to_mir(instructions))
}

fn values(out: &[CodeInstruction]) -> Vec<String> {
    out.iter()
        .flat_map(|inst| inst.fields.iter().map(|(_, v)| v.render()))
        .collect()
}

#[test]
fn positional_abi_remap() {
    let out = sel(&[
        build("mov", &[("dst", "x0"), ("src", "x1")]),
        build("mov", &[("dst", "x7"), ("src", "sp")]),
        build("mov", &[("dst", "x9"), ("src", "xzr")]),
        build("ret", &[]),
    ]);
    let vals = values(&out);
    assert!(vals.contains(&"a0".to_string()));
    assert!(vals.contains(&"a1".to_string()));
    assert!(vals.contains(&"a7".to_string()));
    assert!(vals.contains(&"sp".to_string()));
    assert!(vals.contains(&"zero".to_string()));
}

#[test]
fn explicit_abi_tokens_realize_to_positional_a_registers() {
    use crate::target::shared::abi;
    // plan-85-A: a typed `Operand::Abi` realizes positionally (`x{index}`) then
    // remaps `xN`→`aN`, byte-identically to the legacy tokens — every
    // convention/role collapses. %retC1 → a1, %argSys3 → a3, %retSys → a0.
    let out = sel(&[
        CodeInstruction::new("mov")
            .physical_field("dst", abi::mfb_return(0))
            .physical_field("src", abi::mfb_arg(2)),
        CodeInstruction::new("mov")
            .physical_field("dst", abi::c_return(1))
            .physical_field("src", abi::sys_arg(3)),
        CodeInstruction::new("mov")
            .physical_field("dst", abi::sys_return())
            .physical_field("src", abi::c_arg(7)),
    ]);
    let vals = values(&out);
    assert!(
        vals.contains(&"a0".to_string()),
        "%retMFB0/%retSys → a0: {vals:?}"
    );
    assert!(vals.contains(&"a2".to_string()), "%argMFB2 → a2: {vals:?}");
    assert!(vals.contains(&"a1".to_string()), "%retC1 → a1: {vals:?}");
    assert!(vals.contains(&"a3".to_string()), "%argSys3 → a3: {vals:?}");
    assert!(vals.contains(&"a7".to_string()), "%argC7 → a7: {vals:?}");
}

#[test]
fn link_register_maps_to_ra() {
    let out = sel(&[
        build("str_u64", &[("src", "lr"), ("base", "sp"), ("offset", "0")]),
        build("ret", &[]),
    ]);
    assert!(values(&out).iter().any(|v| v == "ra"));
    assert!(!values(&out).iter().any(|v| v == "lr"));
}

#[test]
fn integer_compare_branch_native() {
    // cmp x9, x10 ; b.lt L  →  rv.br a?/s?, cond=lt
    let out = sel(&[
        build("cmp", &[("lhs", "x9"), ("rhs", "x10")]),
        build("b.lt", &[("target", "L")]),
        build("ret", &[]),
    ]);
    assert!(out.iter().any(|i| i.op == CodeOp::RvBr));
    let br = out.iter().find(|i| i.op == CodeOp::RvBr).unwrap();
    assert_eq!(br.get("cond").as_deref(), Some("lt"));
    assert_eq!(br.get("target").as_deref(), Some("L"));
}

#[test]
fn compare_branch_swaps_for_gt() {
    // cmp x9,x10 ; b.gt L  →  blt x10,x9  (swap)
    let out = sel(&[
        build("cmp", &[("lhs", "x9"), ("rhs", "x10")]),
        build("b.gt", &[("target", "L")]),
        build("ret", &[]),
    ]);
    let br = out.iter().find(|i| i.op == CodeOp::RvBr).unwrap();
    assert_eq!(br.get("cond").as_deref(), Some("lt"));
    // Operands swapped: lhs is the original rhs.
    assert_eq!(br.get("lhs"), br.get("lhs")); // (mapped, both scratch)
}

#[test]
fn compare_imm_materializes_nonzero() {
    let out = sel(&[
        build("cmp_imm", &[("lhs", "x9"), ("rhs", "5")]),
        build("b.eq", &[("target", "L")]),
        build("ret", &[]),
    ]);
    // A non-zero immediate is materialized into t1 (bug-749: t0 is not
    // spill-safe across the rv.br's own reload).
    assert!(out
        .iter()
        .any(|i| i.op == CodeOp::MovImm && i.get("dst").as_deref() == Some("t1")));
    assert!(out.iter().any(|i| i.op == CodeOp::RvBr));
}

#[test]
fn compare_imm_zero_uses_zero_register() {
    let out = sel(&[
        build("cmp_imm", &[("lhs", "x9"), ("rhs", "0")]),
        build("b.ne", &[("target", "L")]),
        build("ret", &[]),
    ]);
    assert!(!out.iter().any(|i| i.op == CodeOp::MovImm));
    let br = out.iter().find(|i| i.op == CodeOp::RvBr).unwrap();
    assert_eq!(br.get("rhs").as_deref(), Some("zero"));
}

#[test]
fn float_compare_branch_uses_fcmp_and_br() {
    let out = sel(&[
        build("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
        build("b.mi", &[("target", "L")]),
        build("ret", &[]),
    ]);
    assert!(out.iter().any(|i| i.op == CodeOp::RvFcmp));
    assert!(out.iter().any(|i| i.op == CodeOp::RvBr));
    // d0/d1 mapped to fa0/fa1.
    let vals = values(&out);
    assert!(vals.contains(&"fa0".to_string()));
    assert!(vals.contains(&"fa1".to_string()));
}

#[test]
fn overflow_add_expands_with_detection() {
    let out = sel(&[
        build("adds", &[("dst", "x9"), ("lhs", "x10"), ("rhs", "x11")]),
        build("b.vc", &[("target", "ok")]),
        build("ret", &[]),
    ]);
    // The add result goes to a scratch, detection uses eor/mvn/and, then br.
    assert!(out.iter().any(|i| i.op == CodeOp::Add));
    assert!(out.iter().any(|i| i.op == CodeOp::Eor));
    assert!(out.iter().any(|i| i.op == CodeOp::Mvn));
    let br = out.iter().find(|i| i.op == CodeOp::RvBr).unwrap();
    assert_eq!(br.get("cond").as_deref(), Some("ge")); // b.vc = no overflow = word >= 0
}

#[test]
fn addr_of_expands_to_auipc_addi_pair() {
    let out = sel(&[
        build("adrp", &[("dst", "x9"), ("symbol", "g")]),
        build(
            "add_pageoff",
            &[("dst", "x9"), ("src", "x9"), ("symbol", "g")],
        ),
    ]);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].op, CodeOp::Adrp);
    assert_eq!(out[1].op, CodeOp::AddPageOff);
}

#[test]
fn arena_base_realizes_to_s11() {
    let realization = crate::codegen::engine::mir::arena_base_realization();
    let out = sel(&[
        build(
            "ldr_u64",
            &[("dst", "x9"), ("base", realization), ("offset", "0")],
        ),
        build("ret", &[]),
    ]);
    assert!(values(&out).iter().any(|v| v == "s11"));
}

#[test]
fn fp_registers_map_to_abi_roles() {
    let out = sel(&[
        build("fadd_d", &[("dst", "d0"), ("lhs", "d8"), ("rhs", "d1")]),
        build("ret", &[]),
    ]);
    let vals = values(&out);
    assert!(vals.contains(&"fa0".to_string()));
    assert!(vals.contains(&"fa1".to_string()));
    assert!(vals.contains(&"fs0".to_string())); // d8 → fs0
}

// ---------- branch-condition mapping helpers ----------

fn fl(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, Operand)> {
    pairs.iter().map(|(k, v)| (*k, Operand::from(*v))).collect()
}

#[test]
fn integer_branch_condition_table() {
    // Every AArch64 compare-branch mnemonic maps to a RISC-V branch relation,
    // swapping operands where RISC-V lacks the direct form.
    assert_eq!(int_branch("b.eq", "a", "b"), ("a", "b", "eq"));
    assert_eq!(int_branch("b.ne", "a", "b"), ("a", "b", "ne"));
    assert_eq!(int_branch("b.ge", "a", "b"), ("a", "b", "ge"));
    assert_eq!(int_branch("b.lt", "a", "b"), ("a", "b", "lt"));
    assert_eq!(int_branch("b.gt", "a", "b"), ("b", "a", "lt")); // swap
    assert_eq!(int_branch("b.le", "a", "b"), ("b", "a", "ge")); // swap
    assert_eq!(int_branch("b.hi", "a", "b"), ("b", "a", "ltu")); // swap
    assert_eq!(int_branch("b.lo", "a", "b"), ("a", "b", "ltu"));
    assert_eq!(int_branch("b.ls", "a", "b"), ("b", "a", "geu")); // swap
    assert_eq!(int_branch("b.hs", "a", "b"), ("a", "b", "geu"));
    assert_eq!(int_branch("b.cs", "a", "b"), ("a", "b", "geu"));
}

#[test]
#[should_panic(expected = "unmapped integer compare-branch")]
fn integer_branch_unknown_condition_panics() {
    int_branch("b.zz", "a", "b");
}

#[test]
fn float_branch_condition_table() {
    // Ordered-only relations emit one compare then branch-on-true.
    for cond in ["b.gt", "b.ge", "b.mi", "b.lo", "b.ls", "b.eq"] {
        let out = float_branch(cond, &Operand::from("fa0"), &Operand::from("fa1"), "L");
        assert_eq!(out.len(), 2, "{cond}");
        assert_eq!(out[0].op.mnemonic(), "rv.fcmp");
        assert_eq!(out[1].get("cond").as_deref(), Some("ne"));
    }
    // Unordered-including relations branch when the ordered complement is false.
    for cond in ["b.ne", "b.hi", "b.lt", "b.le"] {
        let out = float_branch(cond, &Operand::from("fa0"), &Operand::from("fa1"), "L");
        assert_eq!(out.len(), 2, "{cond}");
        assert_eq!(out[1].get("cond").as_deref(), Some("eq"));
    }
    // Finiteness checks compare each operand with itself (NaN detection).
    let vs = float_branch("b.vs", &Operand::from("fa0"), &Operand::from("fa1"), "L");
    assert_eq!(vs.len(), 4);
    assert_eq!(vs[3].get("cond").as_deref(), Some("eq"));
    let vc = float_branch("b.vc", &Operand::from("fa0"), &Operand::from("fa1"), "L");
    assert_eq!(vc.len(), 4);
    assert_eq!(vc[3].get("cond").as_deref(), Some("ne"));
}

#[test]
#[should_panic(expected = "unmapped float compare-branch")]
fn float_branch_unknown_condition_panics() {
    float_branch("b.zz", &Operand::from("fa0"), &Operand::from("fa1"), "L");
}

// ---------- fused-op expansion ----------

#[test]
fn float_compare_against_zero_materializes_plus_zero() {
    let out = expand_fused(
        MirOp::AddrOf, // `op` is unused for this setter
        CodeOp::FCmpZeroD,
        &fl(&[("src", "d0"), ("cond", "b.mi"), ("target", "L")]),
    );
    // First materialize +0.0 into ft1, then a float compare-and-branch.
    assert_eq!(out[0].op.mnemonic(), "fmov_d_from_x");
    assert_eq!(out[0].get("dst").as_deref(), Some("ft1"));
    assert!(out.iter().any(|i| i.op.mnemonic() == "rv.fcmp"));
}

#[test]
fn overflow_subtract_expands_with_detection() {
    let out = expand_fused(
        MirOp::AddrOf,
        CodeOp::Subs,
        &fl(&[
            ("dst", "x9"),
            ("lhs", "x10"),
            ("rhs", "x11"),
            ("cond", "b.vs"),
            ("target", "ov"),
        ]),
    );
    assert!(out.iter().any(|i| i.op.mnemonic() == "sub"));
    // dst is written before the branch (see the Adds/Subs comments).
    assert!(out
        .iter()
        .any(|i| i.op.mnemonic() == "mov" && i.get("dst").as_deref() == Some("x9")));
    let br = out.iter().find(|i| i.op.mnemonic() == "rv.br").unwrap();
    assert_eq!(br.get("cond").as_deref(), Some("lt")); // b.vs = overflow = word < 0
}

#[test]
#[should_panic(expected = "SyscallBr")]
fn fused_syscall_carry_idiom_is_rejected() {
    expand_fused(
        MirOp::AddrOf,
        CodeOp::Svc,
        &fl(&[("cond", "b.hs"), ("target", "e")]),
    );
}

#[test]
#[should_panic(expected = "unexpected fused setter")]
fn fused_unexpected_setter_panics() {
    expand_fused(
        MirOp::AddrOf,
        CodeOp::Add,
        &fl(&[("cond", "b.eq"), ("target", "L")]),
    );
}

#[test]
fn overflow_branch_condition_mapping() {
    assert_eq!(overflow_branch_cond("b.vs"), "lt");
    assert_eq!(overflow_branch_cond("b.vc"), "ge");
}

#[test]
#[should_panic(expected = "unexpected overflow branch")]
fn overflow_branch_unknown_condition_panics() {
    overflow_branch_cond("b.eq");
}

// ---------- bare (non-fused) compare + standalone flag branch ----------

#[test]
fn bare_compare_register_defers_to_standalone_branch() {
    // A non-adjacent cmp/branch (fusion misses it): the cmp snapshots its lhs
    // into `gp` and its rhs *value* into the per-thread snapshot word
    // (bug-381), and the standalone branch reloads that value and re-derives
    // the native compare-branch.
    let out = sel(&[
        build("cmp", &[("lhs", "x9"), ("rhs", "x10")]),
        build("mov", &[("dst", "x12"), ("src", "x13")]),
        build("b.lt", &[("target", "L")]),
        build("ret", &[]),
    ]);
    // lhs saved into the flag register gp.
    assert!(out
        .iter()
        .any(|i| i.op == CodeOp::Mov && i.get("dst").as_deref() == Some("gp")));
    // rhs value spilled to and reloaded from the snapshot word: the address is
    // built off the arena base (s11) — into t1 for the store (bug-749: held
    // across the rhs's spill code) and into t0 for the reload — then the
    // value is stored/loaded through it.
    assert!(out
        .iter()
        .any(|i| i.op.mnemonic() == "add" && i.get("lhs").as_deref() == Some("s11")));
    assert!(out
        .iter()
        .any(|i| i.op.mnemonic() == "str_u64" && i.get("base").as_deref() == Some("t1")));
    assert!(out
        .iter()
        .any(|i| i.op.mnemonic() == "ldr_u64" && i.get("dst").as_deref() == Some("t0")));
    let br = out.iter().find(|i| i.op == CodeOp::RvBr).unwrap();
    assert_eq!(br.get("lhs").as_deref(), Some("gp"));
    assert_eq!(br.get("rhs").as_deref(), Some("t0")); // reloaded rhs value
    assert_eq!(br.get("cond").as_deref(), Some("lt"));
}

#[test]
fn bare_compare_imm_zero_standalone_branch() {
    let out = sel(&[
        build("cmp_imm", &[("lhs", "x9"), ("rhs", "0")]),
        build("mov", &[("dst", "x12"), ("src", "x13")]),
        build("b.ne", &[("target", "L")]),
        build("ret", &[]),
    ]);
    let br = out.iter().find(|i| i.op == CodeOp::RvBr).unwrap();
    assert_eq!(br.get("lhs").as_deref(), Some("gp"));
    assert_eq!(br.get("rhs").as_deref(), Some("zero"));
    assert!(!out.iter().any(|i| i.op == CodeOp::MovImm));
}

/// bug-738: a bare compare whose operand is a typed convention token
/// (`Operand::Abi`, e.g. `emit_stdin_next_byte`'s trailing
/// `cmp_imm %retMFB0, 0`) must keep that operand typed through the `gp`
/// snapshot and the rhs spill, so `remap_riscv_abi` realizes it to its `aN`
/// home. Rendered to text it reached the encoder as the literal
/// `%retMFB0` and the whole build failed.
#[test]
fn bare_compare_on_abi_tokens_realizes_them() {
    use crate::target::shared::abi;
    let out = sel(&[
        CodeInstruction::new("cmp_imm")
            .physical_field("lhs", abi::mfb_return(0))
            .field("rhs", "0"),
        build("mov", &[("dst", "x12"), ("src", "x13")]),
        build("b.eq", &[("target", "L")]),
        CodeInstruction::new("cmp")
            .physical_field("lhs", abi::c_return(1))
            .physical_field("rhs", abi::mfb_arg(2)),
        build("mov", &[("dst", "x12"), ("src", "x13")]),
        build("b.lt", &[("target", "L")]),
        build("ret", &[]),
    ]);
    let vals = values(&out);
    assert!(
        !vals.iter().any(|v| v.starts_with('%')),
        "an ABI token survived selection unrealized: {vals:?}"
    );
    let snapshots: Vec<String> = out
        .iter()
        .filter(|i| i.op == CodeOp::Mov && i.get("dst").as_deref() == Some("gp"))
        .filter_map(|i| i.get("src"))
        .collect();
    assert_eq!(snapshots, ["a0", "a1"], "gp snapshots the realized lhs");
    let spilled: Vec<String> = out
        .iter()
        .filter(|i| i.op.mnemonic() == "str_u64" && i.get("base").as_deref() == Some("t1"))
        .filter_map(|i| i.get("src"))
        .collect();
    assert_eq!(spilled, ["a2"], "the rhs snapshot stores the realized rhs");
}

/// bug-749: the first index at which `t0` holds a value the selector wrote
/// and still reads later while an instruction naming a virtual register sits
/// in between — the allocator inserts that operand's spill code there, and in
/// a >2 KiB frame the encoder builds a spill store's address in `t0`.
fn t0_held_across_a_vreg(out: &[CodeInstruction]) -> Option<usize> {
    use crate::codegen::engine::regalloc::parse_vreg;
    let names = |i: &CodeInstruction| -> Vec<(&'static str, String)> {
        i.fields.iter().map(|(k, v)| (*k, v.render())).collect()
    };
    let writes_t0 = |i: &CodeInstruction| names(i).iter().any(|(k, v)| *k == "dst" && v == "t0");
    let reads_t0 = |i: &CodeInstruction| names(i).iter().any(|(k, v)| *k != "dst" && v == "t0");
    let has_vreg = |i: &CodeInstruction| names(i).iter().any(|(_, v)| parse_vreg(v).is_some());
    for (w, inst) in out.iter().enumerate() {
        if !writes_t0(inst) {
            continue;
        }
        let mut crossed = false;
        for later in &out[w + 1..] {
            if reads_t0(later) {
                if crossed {
                    return Some(w);
                }
                // The reader itself may carry a vreg: its spill code lands
                // BEFORE it, i.e. inside the window, too.
                if has_vreg(later) {
                    return Some(w);
                }
                break;
            }
            if writes_t0(later) {
                break;
            }
            crossed |= has_vreg(later);
        }
    }
    None
}

/// bug-749: no selector expansion may hold `t0` across an instruction that
/// names a virtual register (the allocator's spill code for it can clobber
/// `t0`). Covers every multi-instruction scratch sequence the selector emits
/// for integer operands: the fused immediate compare, the overflow-checked
/// add/subtract, and the bare register compare's rhs snapshot.
#[test]
fn no_selected_sequence_holds_t0_across_a_virtual_register() {
    use crate::codegen::engine::regalloc::vreg_name;
    let (a, b, d) = (vreg_name(1), vreg_name(2), vreg_name(3));
    let cases: Vec<(&str, Vec<CodeInstruction>)> = vec![
        (
            "cmp_imm + b.ne (fused)",
            vec![
                build("cmp_imm", &[("lhs", &a), ("rhs", "45")]),
                build("b.ne", &[("target", "L")]),
            ],
        ),
        (
            "adds + b.vs (fused)",
            vec![
                build("adds", &[("dst", &d), ("lhs", &a), ("rhs", &b)]),
                build("b.vs", &[("target", "L")]),
            ],
        ),
        (
            "subs + b.vc (fused)",
            vec![
                build("subs", &[("dst", &d), ("lhs", &a), ("rhs", &b)]),
                build("b.vc", &[("target", "L")]),
            ],
        ),
        (
            "bare cmp (rhs snapshot)",
            vec![
                build("cmp", &[("lhs", &a), ("rhs", &b)]),
                build("mov", &[("dst", "x12"), ("src", "x13")]),
                build("b.lt", &[("target", "L")]),
            ],
        ),
    ];
    for (name, mut stream) in cases {
        stream.push(build("label", &[("name", "L")]));
        stream.push(build("ret", &[]));
        let out = sel(&stream);
        if let Some(at) = t0_held_across_a_vreg(&out) {
            let dump: Vec<String> = out
                .iter()
                .map(|i| format!("{} {:?}", i.op.mnemonic(), values(std::slice::from_ref(i))))
                .collect();
            panic!(
                "{name}: t0 written at {at} is held across a virtual register:\n{}",
                dump.join("\n")
            );
        }
    }
}

/// bug-760 (found on 2229): a float compare-and-branch whose compare names FP
/// virtual registers must not leave its bool in `t0`. The FP allocator may put
/// an eviction restore right after the compare — an `fld` whose out-of-range
/// `sp` offset the encoder builds in `t0` — so a `t0` bool was overwritten by
/// an address before the branch read it (`nbody` at -O1 raised a spurious
/// "overflowed to infinity" from its finiteness check).
#[test]
fn float_compare_branch_keeps_its_bool_out_of_t0() {
    use crate::codegen::engine::regalloc::{fp_vreg_name, parse_fp_vreg};
    let (a, b) = (fp_vreg_name(1), fp_vreg_name(2));
    for cond in [
        "b.gt", "b.ge", "b.mi", "b.lo", "b.ls", "b.eq", "b.ne", "b.hi", "b.lt", "b.le", "b.vs",
        "b.vc",
    ] {
        let out = sel(&[
            build("fcmp_d", &[("lhs", &a), ("rhs", &b)]),
            build(cond, &[("target", "L")]),
            build("label", &[("name", "L")]),
            build("ret", &[]),
        ]);
        let compares: Vec<&CodeInstruction> =
            out.iter().filter(|i| i.op == CodeOp::RvFcmp).collect();
        assert!(!compares.is_empty(), "{cond}: no rv.fcmp");
        for compare in compares {
            let names_fp_vreg = compare
                .fields
                .iter()
                .any(|(_, v)| parse_fp_vreg(&v.render()).is_some());
            assert!(names_fp_vreg, "{cond}: the compare reads the FP vregs");
            assert_ne!(
                compare.get("dst").as_deref(),
                Some("t0"),
                "{cond}: the compare's bool must not live in t0"
            );
        }
    }
}

#[test]
fn bare_compare_imm_nonzero_standalone_branch() {
    let out = sel(&[
        build("cmp_imm", &[("lhs", "x9"), ("rhs", "7")]),
        build("mov", &[("dst", "x12"), ("src", "x13")]),
        build("b.eq", &[("target", "L")]),
        build("ret", &[]),
    ]);
    // The immediate is re-materialized into t0 at the branch.
    assert!(out
        .iter()
        .any(|i| i.op == CodeOp::MovImm && i.get("dst").as_deref() == Some("t0")));
    assert!(out.iter().any(|i| i.op == CodeOp::RvBr));
}

#[test]
fn pending_compare_invalidated_by_label() {
    // A label between the compare and any consumer strands the pending compare
    // (bug-126.2): control flow can merge at the label, so the gp/snapshot are
    // stale on the merged-in path. No branch reads it here, so selection must
    // not panic, and the lhs snapshot mov is still emitted.
    let out = sel(&[
        build("cmp", &[("lhs", "x9"), ("rhs", "x10")]),
        build("label", &[("name", "mid")]),
        build("ret", &[]),
    ]);
    assert!(out
        .iter()
        .any(|i| i.op == CodeOp::Mov && i.get("dst").as_deref() == Some("gp")));
}

/// bug-381: the compare snapshots BOTH operands by value (lhs → gp, rhs → the
/// per-thread word), so a redefinition of an original operand register between
/// the compare and its branch is now *recoverable* — the branch reads the
/// snapshots, not the since-clobbered originals. This is the exact schedule
/// bug-284 C3 could only detect (it panicked). `add_carry`'s `carry_out`
/// writing the compare's rhs register no longer strands the branch.
#[test]
fn pending_compare_recovers_across_a_carry_out_def_of_its_rhs() {
    let out = sel(&[
        build("cmp", &[("lhs", "x9"), ("rhs", "x10")]),
        build(
            "add_carry",
            &[
                ("dst", "x11"),
                ("carry_out", "x10"),
                ("lhs", "x12"),
                ("rhs", "x13"),
            ],
        ),
        build("b.lt", &[("target", "L")]),
        build("ret", &[]),
    ]);
    // The branch reads gp and the reloaded snapshot (t0), not x10 (whose home
    // the carry_out redefined), so it is correct.
    let br = out.iter().find(|i| i.op == CodeOp::RvBr).unwrap();
    assert_eq!(br.get("lhs").as_deref(), Some("gp"));
    assert_eq!(br.get("rhs").as_deref(), Some("t0"));
    assert_eq!(br.get("cond").as_deref(), Some("lt"));
}

#[test]
fn pending_compare_recovers_across_a_borrow_out_def_of_its_rhs() {
    let out = sel(&[
        build("cmp", &[("lhs", "x9"), ("rhs", "x10")]),
        build(
            "sub_borrow",
            &[
                ("dst", "x11"),
                ("borrow_out", "x10"),
                ("lhs", "x12"),
                ("rhs", "x13"),
            ],
        ),
        build("b.lt", &[("target", "L")]),
        build("ret", &[]),
    ]);
    let br = out.iter().find(|i| i.op == CodeOp::RvBr).unwrap();
    assert_eq!(br.get("lhs").as_deref(), Some("gp"));
    assert_eq!(br.get("rhs").as_deref(), Some("t0"));
    assert_eq!(br.get("cond").as_deref(), Some("lt"));
}

/// bug-381, the observed failure shape: under register pressure the allocator
/// reloads a compare operand (`ldr_u64 dst=<operand>, [sp+..]`) into the
/// compare→branch span, redefining it. Before the fix the branch was stranded
/// (`pending` cleared, then a panic); now the rhs value lives in the snapshot
/// word, so the reload is harmless and a correct branch is emitted.
#[test]
fn pending_compare_recovers_across_a_reload_of_its_operand() {
    let out = sel(&[
        build("cmp", &[("lhs", "x9"), ("rhs", "x10")]),
        build(
            "ldr_u64",
            &[("dst", "x10"), ("base", "sp"), ("offset", "80")],
        ),
        build("b.lo", &[("target", "L")]),
        build("ret", &[]),
    ]);
    let br = out.iter().find(|i| i.op == CodeOp::RvBr).unwrap();
    assert_eq!(br.get("lhs").as_deref(), Some("gp"));
    assert_eq!(br.get("rhs").as_deref(), Some("t0"));
    assert_eq!(br.get("cond").as_deref(), Some("ltu")); // b.lo → unsigned lhs < rhs
}

/// bug-284 C3 / bug-381: a call still invalidates the pending compare. The
/// callee's own flag emulation would overwrite the shared per-thread snapshot
/// word, and a call clobbers flags on every target anyway, so a compare whose
/// branch sits across a call is a codegen bug we detect rather than miscompile.
#[test]
#[should_panic(expected = "standalone flag branch without a preceding compare")]
fn pending_compare_invalidated_by_an_intervening_call() {
    sel(&[
        build("cmp", &[("lhs", "x9"), ("rhs", "x10")]),
        build("bl", &[("target", "some_function")]),
        build("b.lt", &[("target", "L")]),
        build("ret", &[]),
    ]);
}

/// The control case: with nothing in between, the compare still reaches its
/// branch and fuses, so the invalidations above are not over-broad.
#[test]
fn an_undisturbed_pending_compare_still_reaches_its_branch() {
    let out = sel(&[
        build("cmp", &[("lhs", "x9"), ("rhs", "x10")]),
        build("b.lt", &[("target", "L")]),
        build("ret", &[]),
    ]);
    assert!(
        out.iter().any(|i| i.op.mnemonic() == "rv.br"),
        "an undisturbed compare must still fuse into rv.br"
    );
}

// ---------- v128 scalarization through selection ----------

#[test]
fn v128_op_scalarizes_during_selection() {
    let out = sel(&[
        build("fadd_v", &[("dst", "d0"), ("lhs", "d1"), ("rhs", "d2")]),
        build("ret", &[]),
    ]);
    // Scalarized to the per-thread slot base plus two lane fadd_d.
    assert!(out.iter().any(|i| i.op.mnemonic() == "add_imm"));
    assert_eq!(
        out.iter().filter(|i| i.op.mnemonic() == "fadd_d").count(),
        2
    );
}

// ---------- residual physical-register remapping ----------

#[test]
fn scratch_register_homes() {
    assert_eq!(map_scratch_register(9), "t3");
    assert_eq!(map_scratch_register(10), "t4");
    assert_eq!(map_scratch_register(11), "t5");
    assert_eq!(map_scratch_register(12), "t6");
    assert_eq!(map_scratch_register(13), "s0");
    assert_eq!(map_scratch_register(14), "s3");
    assert_eq!(map_scratch_register(15), "s4");
    assert_eq!(map_scratch_register(16), "s5");
    assert_eq!(map_scratch_register(17), "s6");
    assert_eq!(map_scratch_register(18), "s7");
    assert_eq!(map_scratch_register(19), "s1"); // x19 → s1
    assert_eq!(map_scratch_register(28), "s10"); // x28 → s10
    assert_eq!(map_scratch_register(29), "t6"); // fp / straggler default
    assert_eq!(map_scratch_register(31), "t6");
}

#[test]
fn fp_register_homes() {
    assert_eq!(map_fp_register(0), "fa0");
    assert_eq!(map_fp_register(7), "fa7");
    assert_eq!(map_fp_register(8), "fs0");
    assert_eq!(map_fp_register(15), "fs7");
    // bug-218 narrowed this to d16..d24 → ft3..ft11 so d16 stops aliasing
    // ft2, which the regmodel reserves alongside ft0/ft1.
    assert_eq!(map_fp_register(16), "ft3"); // skips reserved ft0/ft1/ft2
    assert_eq!(map_fp_register(24), "ft11");
}

#[test]
#[should_panic(expected = "unexpected physical FP register")]
fn fp_register_out_of_range_panics() {
    // d24 is the last mapped register since bug-218, so d25 is the first
    // out-of-range one — testing the boundary itself, rather than a number
    // safely past it, is what catches the range widening back over ft2.
    map_fp_register(25);
}

#[test]
fn remap_register_cases() {
    assert_eq!(remap_register("sp").as_deref(), Some("sp"));
    assert_eq!(remap_register("raw_sp").as_deref(), Some("sp"));
    assert_eq!(remap_register("xzr").as_deref(), Some("zero"));
    assert_eq!(remap_register("lr").as_deref(), Some("ra"));
    assert_eq!(remap_register("x0").as_deref(), Some("a0"));
    assert_eq!(remap_register("x7").as_deref(), Some("a7"));
    assert_eq!(remap_register("x8").as_deref(), Some("a7")); // syscall number reg
    assert_eq!(remap_register("x9").as_deref(), Some("t3"));
    assert_eq!(remap_register("w5").as_deref(), Some("a5")); // wN alias
                                                             // n >= 30: a stray physical is mapped to an invalid `aN` the encoder rejects.
    assert_eq!(remap_register("x30").as_deref(), Some("a30"));
    assert_eq!(remap_register("d0").as_deref(), Some("fa0"));
    assert_eq!(remap_register("d16").as_deref(), Some("ft3")); // bug-218
                                                               // Non-physical operands pass through unchanged.
    assert_eq!(remap_register("a0"), None);
    assert_eq!(remap_register("%f3"), None);
    assert_eq!(remap_register("t0"), None);
}
