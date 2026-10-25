//! RISC-V 64 instruction selection (plan-99): neutral MIR → RV64GC machine ops.
//!
//! The rv64 counterpart of `arch::aarch64::select` / `arch::x86_64::select`. It
//! consumes the shared neutral [`MirInstruction`] stream (via
//! `mir::Backend::select`) and produces [`CodeInstruction`]s with RISC-V lp64d
//! register names, using the shared MIR primitives.
//!
//! Two jobs, like the other backends:
//!
//! 1. **Expand the flagless fused ops.** RISC-V has *no condition flags*, so a
//!    fused compare-and-branch is a single native B-type instruction
//!    (`b<cond> rs1, rs2, label`); a float compare is `feq/flt/fle.d` into a GPR
//!    the branch then tests; an overflow-checked add/sub computes the sum then a
//!    sign-comparison. `addr_of` becomes the `auipc; addi` (`Adrp`+`AddPageOff`)
//!    pair the encoder realizes as PC-relative. This is where the flagless MIR
//!    earns its keep (plan-99 §1).
//!
//! 2. **Remap the residual physical registers.** The neutral MIR still carries
//!    AArch64 physical names for ABI boundaries and the hand-written helpers.
//!    Unlike x86 — where argument, return, and syscall registers are three
//!    disjoint files, forcing a control-flow role analysis — RISC-V reuses the
//!    `a0`–`a7` bank for arguments *and* results *and* syscall arguments, exactly
//!    as AArch64 reuses `x0`–`x7`. So the remap is a simple **positional**
//!    substitution (`xN → aN`) with a handful of fixed cases (`x8` → the syscall
//!    number register `a7`, `x30` → `ra`, `x31`/`xzr` → the hardware zero `zero`,
//!    the scratch pool for `x9`–`x29`, and the FP `dN` → the FP ABI role).

use crate::arch::ops::CodeOp;
use crate::codegen::engine::mir::{
    fused_setter_codeop, rename_operand_field_values, MirInstruction, MirOp, ARENA_BASE,
    FUSED_COND_FIELD, FUSED_SHARE_FIELD,
};
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::CodeInstruction;

use super::regmodel::ARENA_BASE_REGISTER;

/// The fixed lowering-scratch integer registers, reserved from allocation
/// (`regmodel`): `t0`–`t2` stage immediate materialization, overflow detection,
/// and the float-compare boolean.
///
/// **Spill-safe scratch (bug-749).** The allocator runs AFTER selection, so any
/// instruction naming a virtual register may get a reload before it, a store after
/// it, and an eviction save/restore around it. The encoder builds the address of an
/// `sp` store (and of any FP load/store) whose offset exceeds the 12-bit immediate
/// in `t0` (`encode::emitter::Encoder::emit_store`), so in a frame over 2 KiB spill
/// code overwrites `t0` — and only `t0`. A value the selector writes in one emitted
/// instruction and reads in a later one, with a virtual register named in between
/// (or by the reader itself, or by the writer — an eviction restore lands after
/// it, and an FP restore is an `fld` that builds its address in `t0`), must
/// therefore live in `t1`/`t2`, never `t0`.
const T0: &str = "t0";
const T1: &str = "t1";
const T2: &str = "t2";
/// Fixed FP scratch for the compare-vs-zero constant.
const FT1: &str = "ft1";
/// The hardware zero register.
const ZERO: &str = "zero";
/// The flag register (plan-99): a bare `cmp`/`cmp_imm` whose flag-reading branch
/// is NOT adjacent (fusion missed it — the flags outlive intervening loads in a
/// few hand-written net/link helpers, and, under register pressure, a spill/reload
/// the allocator threads between a compare and its branch) saves its **left**
/// operand here at the compare and the standalone branch re-derives the condition
/// from it. `gp` (x3) is never used by the codegen (no gp-relative addressing) and
/// is preserved across calls, so it survives the whole compare→branch span.
///
/// bug-381: the **right** operand cannot share a second such register — rv64 has
/// no free one (`tp`/x4 faults a dynamically-linked binary via TLS, and shrinking
/// the allocatable pool to free a temporary destabilizes the allocator) — so it is
/// spilled to a reserved per-thread memory word ([`ARENA_FLAG_RHS_OFFSET`]) at the
/// compare and reloaded at the branch. That keeps the compared *value* live across
/// any spill/reload or redefinition of the original operand register, which a
/// flagless target must do. See [`store_flag_rhs`] / [`load_flag_rhs`].
const GP: &str = "gp";

/// The value of a named field (empty string if absent).
fn field_value(fields: &[(&'static str, Operand)], name: &str) -> String {
    fields
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.render())
        .unwrap_or_default()
}

/// The typed operand of a named register field (an empty operand if absent).
///
/// bug-738: a register operand must stay typed through selection — rendering a
/// convention token (`Operand::Abi`, e.g. `%retMFB0`) to text strands it, because
/// `remap_riscv_abi` realizes only the typed form and the encoder then rejects the
/// literal token. Use this, not [`field_value`], for any register the selector
/// re-emits (the fused path's `get_op` in [`expand_fused`] is the same rule).
fn field_operand(fields: &[(&'static str, Operand)], name: &str) -> Operand {
    fields
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| Operand::from(""))
}

/// Build a `CodeInstruction` from a mnemonic and `(field, value)` pairs.
fn ci(mnemonic: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    let mut inst = CodeInstruction::new(mnemonic);
    for (k, v) in fields {
        inst = inst.physical_field(k, v);
    }
    inst
}

/// Map an AArch64 integer compare condition (the branch mnemonic after `cmp`) to
/// the RISC-V branch that reproduces it: `(rs1, rs2, riscv_cond)` where the
/// branch is `b<riscv_cond> rs1, rs2, target`. `lhs`/`rhs` are the compare's two
/// operands (`lhs CMP rhs`); some relations swap them (RISC-V has only `lt`/`ge`,
/// signed and unsigned, so `>`/`<=` swap operands).
fn int_branch<'a, T: ?Sized>(cond: &str, lhs: &'a T, rhs: &'a T) -> (&'a T, &'a T, &'static str) {
    match cond {
        "b.eq" => (lhs, rhs, "eq"),
        "b.ne" => (lhs, rhs, "ne"),
        "b.ge" => (lhs, rhs, "ge"),           // signed lhs >= rhs
        "b.lt" => (lhs, rhs, "lt"),           // signed lhs <  rhs
        "b.gt" => (rhs, lhs, "lt"),           // signed lhs >  rhs  ⇔ rhs < lhs
        "b.le" => (rhs, lhs, "ge"),           // signed lhs <= rhs  ⇔ rhs >= lhs
        "b.hi" => (rhs, lhs, "ltu"),          // unsigned lhs >  rhs ⇔ rhs <u lhs
        "b.lo" => (lhs, rhs, "ltu"),          // unsigned lhs <  rhs
        "b.ls" => (rhs, lhs, "geu"),          // unsigned lhs <= rhs ⇔ rhs >=u lhs
        "b.hs" | "b.cs" => (lhs, rhs, "geu"), // unsigned lhs >= rhs
        other => panic!("rv64: unmapped integer compare-branch condition '{other}'"),
    }
}

/// Emit the RISC-V float compare-and-branch for one AArch64 `fcmp`+`b.cc` pair
/// (plan-99). RISC-V float comparisons (`feq.d`/`flt.d`/`fle.d`) write a 0/1 GPR
/// with **ordered** (NaN ⇒ 0) semantics; the branch then tests that GPR. Each
/// relation below reproduces the exact truth set the AArch64 `b.cc` mnemonic
/// carries after `fcmp` (the same contract `x86_float_branch` documents):
/// `>`/`>=`/`<`(`b.mi`/`b.lo`)/`<=`(`b.ls`)/`=` are ordered-only; `<>`(`b.ne`),
/// `b.lt`/`b.le`/`b.hi` and the finiteness checks `b.vs`/`b.vc` include the
/// unordered (NaN) case.
fn float_branch(cond: &str, lhs: &Operand, rhs: &Operand, target: &str) -> Vec<CodeInstruction> {
    // `feq/flt/fle.d dst, a, b` then branch `dst <bcond> zero`. `a`/`b` are the
    // compare operands (possibly typed `Operand::Abi` tokens), preserved through the
    // instruction so nothing renders a convention token to a `Raw` string (plan-85-D).
    let cmp = |dst: &str, cmp: &str, a: &Operand, b: &Operand| {
        CodeInstruction::new("rv.fcmp")
            .physical_field("dst", dst)
            .physical_field("lhs", a)
            .physical_field("rhs", b)
            .field("cmp", cmp)
    };
    let br = |a: &str, bcond: &str| {
        ci(
            "rv.br",
            &[
                ("lhs", a),
                ("rhs", ZERO),
                ("cond", bcond),
                ("target", target),
            ],
        )
    };
    match cond {
        // The bool lives in `t1`, never `t0` (bug-749's spill-safe rule): the
        // compare names the operands' FP registers, so the FP allocator may put an
        // eviction restore — an `fld` whose out-of-range `sp` offset the encoder
        // builds in `t0` — between the compare and the branch.
        // Ordered-only relations: the compare already excludes NaN, so branch when true.
        "b.gt" => vec![cmp(T1, "lt", rhs, lhs), br(T1, "ne")], // rhs < lhs
        "b.ge" => vec![cmp(T1, "le", rhs, lhs), br(T1, "ne")], // rhs <= lhs
        "b.mi" | "b.lo" => vec![cmp(T1, "lt", lhs, rhs), br(T1, "ne")], // lhs < rhs (ordered)
        "b.ls" => vec![cmp(T1, "le", lhs, rhs), br(T1, "ne")], // lhs <= rhs (ordered)
        "b.eq" => vec![cmp(T1, "eq", lhs, rhs), br(T1, "ne")], // lhs == rhs (ordered)
        // Unordered-including relations: branch when the ordered complement is false.
        "b.ne" => vec![cmp(T1, "eq", lhs, rhs), br(T1, "eq")], // NOT(ordered ==) = ≠ or NaN
        "b.hi" => vec![cmp(T1, "le", lhs, rhs), br(T1, "eq")], // NOT(lhs<=rhs) = > or NaN
        "b.lt" => vec![cmp(T1, "le", rhs, lhs), br(T1, "eq")], // NOT(rhs<=lhs) = < or NaN
        "b.le" => vec![cmp(T1, "lt", rhs, lhs), br(T1, "eq")], // NOT(rhs<lhs) = <= or NaN
        // Finiteness: unordered iff either operand is not self-equal (NaN).
        "b.vs" => vec![
            cmp(T1, "eq", lhs, lhs),
            cmp(T2, "eq", rhs, rhs),
            ci("and", &[("dst", T0), ("lhs", T1), ("rhs", T2)]),
            br(T0, "eq"), // both self-equal ⇒ ordered; branch when unordered
        ],
        "b.vc" => vec![
            cmp(T1, "eq", lhs, lhs),
            cmp(T2, "eq", rhs, rhs),
            ci("and", &[("dst", T0), ("lhs", T1), ("rhs", T2)]),
            br(T0, "ne"), // both self-equal ⇒ ordered
        ],
        other => panic!("rv64: unmapped float compare-branch condition '{other}'"),
    }
}

/// The `cond` value carried by an integer compare-branch, plus the branch target.
fn cond_and_target(fields: &[(&'static str, Operand)]) -> (String, String) {
    let split = fields
        .iter()
        .position(|(key, _)| *key == FUSED_COND_FIELD)
        .expect("fused MIR op carries a cond field");
    let cond = fields[split].1.render();
    let target = fields[split + 1..]
        .iter()
        .find(|(key, _)| *key == "target")
        .map(|(_, v)| v.render())
        .expect("fused compare-branch carries a target");
    (cond, target)
}

/// Whether the fused op's branch reuses the preceding comparison. On RISC-V a
/// compare-and-branch is self-contained (no shared flags), so a shared branch
/// simply re-emits the whole compare — but the operands are the same, so it is
/// correct and cheap.
fn is_shared(fields: &[(&'static str, Operand)]) -> bool {
    fields.iter().any(|(key, _)| *key == FUSED_SHARE_FIELD)
}

/// Expand one fused MIR op into RISC-V machine ops (plan-99). `setter_op` is the
/// AArch64 flag-setter the fusion recorded; the fields are
/// `[<setter operands>, cond, <branch fields>]`.
fn expand_fused(
    op: MirOp,
    setter_op: CodeOp,
    fields: &[(&'static str, Operand)],
) -> Vec<CodeInstruction> {
    let get = |name: &str| -> String {
        fields
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, v)| v.render())
            .unwrap_or_default()
    };
    // Register-operand lookup that PRESERVES the typed `Operand` (plan-85-D): a
    // compare/overflow operand that is a convention token (`Operand::Abi`) must not
    // be rendered to a `Raw` string here, or it would reach the realization seam as a
    // string and force the string-token realizer to exist. Physical temps and
    // immediates still flow as strings via `get`.
    let get_op = |name: &str| -> Operand {
        fields
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| Operand::from(""))
    };
    let (cond, target) = cond_and_target(fields);
    match setter_op {
        // Integer register-register compare-and-branch.
        CodeOp::Cmp => {
            let lhs = get_op("lhs");
            let rhs = get_op("rhs");
            let (a, b, rvcond) = int_branch(&cond, &lhs, &rhs);
            vec![CodeInstruction::new("rv.br")
                .physical_field("lhs", a)
                .physical_field("rhs", b)
                .field("cond", rvcond)
                .field("target", &target)]
        }
        // Integer register-immediate compare-and-branch. RISC-V branches take
        // registers only, so a non-zero immediate is materialized into `t1` —
        // NOT `t0`: the allocator inserts `lhs`'s spill code between the two
        // instructions, and in a >2 KiB frame a spill store builds its address in
        // `t0` (bug-749; see the spill-safe scratch rule on [`T0`]).
        CodeOp::CmpImm => {
            let lhs = get_op("lhs");
            let imm = get("rhs");
            let mut out = Vec::new();
            let imm_reg: Operand = if imm == "0" {
                Operand::from(ZERO)
            } else {
                out.push(ci("mov_imm", &[("dst", T1), ("value", &imm)]));
                Operand::from(T1)
            };
            let (a, b, rvcond) = int_branch(&cond, &lhs, &imm_reg);
            out.push(
                CodeInstruction::new("rv.br")
                    .physical_field("lhs", a)
                    .physical_field("rhs", b)
                    .field("cond", rvcond)
                    .field("target", &target),
            );
            out
        }
        // Float register-register compare-and-branch.
        CodeOp::FCmpD => {
            let lhs = get_op("lhs");
            let rhs = get_op("rhs");
            float_branch(&cond, &lhs, &rhs, &target)
        }
        // Float compare against +0.0: materialize +0.0 into `ft1` (`fmv.d.x`),
        // then the same compare-and-branch against it.
        CodeOp::FCmpZeroD => {
            let src = get_op("src");
            let ft1 = Operand::from(FT1);
            let mut out = vec![ci("fmov_d_from_x", &[("dst", FT1), ("src", ZERO)])];
            out.extend(float_branch(&cond, &src, &ft1, &target));
            out
        }
        // Overflow-checked add: `dst = lhs + rhs`, branch on signed overflow.
        // Overflow iff the inputs share a sign that differs from the result's:
        //   sum = lhs + rhs
        //   ovf = (~(lhs ^ rhs) & (sum ^ lhs)) < 0
        CodeOp::Adds => {
            let dst = get_op("dst");
            let lhs = get_op("lhs");
            let rhs = get_op("rhs");
            let branch_cond = overflow_branch_cond(&cond);
            // The result must be written into `dst` BEFORE the branch: the fused
            // `adds; b.vc` writes the destination and *then* branches, and the
            // branch jumps away on the no-overflow path — so a `mov dst, t2` after
            // it would be skipped, leaving `dst` undefined.
            //
            // bug-749: the sum (`t2`) and the sign word (`t1`) live across
            // instructions that name virtual registers, so they must not be in
            // `t0`; `t0` carries only `sum ^ lhs`, consumed by the very next
            // register-free `and` after `mvn`.
            vec![
                CodeInstruction::new("add")
                    .physical_field("dst", T2)
                    .physical_field("lhs", &lhs)
                    .physical_field("rhs", &rhs),
                CodeInstruction::new("eor")
                    .physical_field("dst", T1)
                    .physical_field("lhs", &lhs)
                    .physical_field("rhs", &rhs),
                CodeInstruction::new("eor")
                    .physical_field("dst", T0)
                    .physical_field("lhs", T2)
                    .physical_field("rhs", &lhs),
                ci("mvn", &[("dst", T1), ("src", T1)]),
                ci("and", &[("dst", T1), ("lhs", T1), ("rhs", T0)]),
                CodeInstruction::new("mov")
                    .physical_field("dst", &dst)
                    .physical_field("src", T2),
                ci(
                    "rv.br",
                    &[
                        ("lhs", T1),
                        ("rhs", ZERO),
                        ("cond", branch_cond),
                        ("target", &target),
                    ],
                ),
            ]
        }
        // Overflow-checked subtract: `dst = lhs - rhs`, branch on signed overflow.
        //   diff = lhs - rhs
        //   ovf = ((lhs ^ rhs) & (lhs ^ diff)) < 0
        CodeOp::Subs => {
            let dst = get_op("dst");
            let lhs = get_op("lhs");
            let rhs = get_op("rhs");
            let branch_cond = overflow_branch_cond(&cond);
            // Write `dst` before the branch, and keep `t0` out of every value
            // held across a virtual register (see the `Adds` case above).
            vec![
                CodeInstruction::new("sub")
                    .physical_field("dst", T2)
                    .physical_field("lhs", &lhs)
                    .physical_field("rhs", &rhs),
                CodeInstruction::new("eor")
                    .physical_field("dst", T1)
                    .physical_field("lhs", &lhs)
                    .physical_field("rhs", &rhs),
                CodeInstruction::new("eor")
                    .physical_field("dst", T0)
                    .physical_field("lhs", &lhs)
                    .physical_field("rhs", T2),
                ci("and", &[("dst", T1), ("lhs", T1), ("rhs", T0)]),
                CodeInstruction::new("mov")
                    .physical_field("dst", &dst)
                    .physical_field("src", T2),
                ci(
                    "rv.br",
                    &[
                        ("lhs", T1),
                        ("rhs", ZERO),
                        ("cond", branch_cond),
                        ("target", &target),
                    ],
                ),
            ]
        }
        CodeOp::Svc => {
            // `svc; b.<carry>` is the macOS syscall-error idiom (carry set on
            // error). Linux syscalls return `-errno` and the builders check the
            // result with an ordinary `cmp`/branch, so this fused form should not
            // reach a Linux rv64 build. Fail loud rather than silently miscompile.
            let _ = (op, get("cond"));
            panic!("rv64: SyscallBr (macOS carry idiom) is unexpected on Linux");
        }
        other => panic!("rv64: unexpected fused setter {}", other.mnemonic()),
    }
}

/// The RISC-V branch condition for an overflow check: the fused `add_ovf`/
/// `sub_ovf` carries `b.vc` (branch when NO overflow) or `b.vs` (branch on
/// overflow). The detection word's sign bit is set iff overflow occurred, so
/// `b.vs` → `lt` (word < 0) and `b.vc` → `ge` (word >= 0).
fn overflow_branch_cond(cond: &str) -> &'static str {
    match cond {
        "b.vs" => "lt",
        "b.vc" => "ge",
        other => panic!("rv64: unexpected overflow branch condition '{other}'"),
    }
}

/// The right-hand side of a pending (non-fused) integer compare, saved so a
/// later standalone flag-reading branch can re-derive its condition (plan-99).
enum FlagRhs {
    /// Compare against the hardware zero register.
    Zero,
    /// Compare against an immediate, re-materialized at the branch.
    Imm(String),
    /// Compare against a register operand. bug-381: its value is spilled to the
    /// per-thread [`ARENA_FLAG_RHS_OFFSET`] word at the compare, so the branch
    /// reloads the compared *value* rather than re-reading a register the
    /// allocator may have overwritten in the span. No payload — the value lives in
    /// memory, not a named register.
    Reg,
}

/// Materialize the address of the per-thread flag-rhs snapshot word
/// (`arena_base + ARENA_FLAG_RHS_OFFSET`) into `dst`. The offset is past the
/// 12-bit load/store immediate, so it is built and added to the pinned arena base
/// (`ARENA_BASE` is renamed to the physical arena register at the end of
/// selection). `dst` must be a lowering-scratch register free at the call site.
fn flag_rhs_address(dst: &str) -> Vec<CodeInstruction> {
    let offset = crate::codegen::error::constants::ARENA_FLAG_RHS_OFFSET.to_string();
    vec![
        ci("mov_imm", &[("dst", dst), ("value", &offset)]),
        ci("add", &[("dst", dst), ("lhs", ARENA_BASE), ("rhs", dst)]),
    ]
}

/// bug-381: spill a bare compare's rhs *value* to the per-thread snapshot word.
/// Uses `t1` as the address scratch (reserved lowering scratch, free at a bare
/// compare): the address is held across the store of `rhs`, a virtual register
/// whose spill code the allocator places in between, so it must not be `t0`
/// (bug-749, the spill-safe scratch rule on [`T0`]).
fn store_flag_rhs(rhs: &Operand) -> Vec<CodeInstruction> {
    let mut out = flag_rhs_address(T1);
    out.push(
        CodeInstruction::new("str_u64")
            .physical_field("src", rhs)
            .physical_field("base", T1)
            .physical_field("offset", "0"),
    );
    out
}

/// bug-381: reload the snapshotted rhs value into `dst` at the branch. `dst`
/// doubles as the address scratch (materialize address into `dst`, then load
/// through it), so a single lowering-scratch register suffices.
fn load_flag_rhs(dst: &str) -> Vec<CodeInstruction> {
    let mut out = flag_rhs_address(dst);
    out.push(ci(
        "ldr_u64",
        &[("dst", dst), ("base", dst), ("offset", "0")],
    ));
    out
}

/// Save a bare compare's lhs into the flag register `gp`, keeping the operand
/// typed so a convention token realizes to its `aN` home (bug-738).
fn snapshot_lhs(lhs: &Operand) -> CodeInstruction {
    CodeInstruction::new("mov")
        .physical_field("dst", GP)
        .physical_field("src", lhs)
}

/// Select neutral MIR into RV64GC machine ops (plan-99).
pub(crate) fn select_riscv64(instructions: Vec<MirInstruction>) -> Vec<CodeInstruction> {
    let mut out = Vec::with_capacity(instructions.len());
    // Assign a memory slot to every distinct `v128` value the function uses, so
    // the SIMD ops can be scalarized onto the slot region (plan-99 §6).
    let v128_slots = super::v128::build_slot_map(&instructions);
    // plan-32-C: physical vector-register assignment for the native-RVV arm of the
    // dual-path v128 lowering. `None` (register pressure past the v1..=v30 pool)
    // means this function emits the scalar arm only. `v128_seq` gives each v128
    // site a unique pair of arm labels.
    let v128_vregs = super::v128::build_vreg_map(&instructions);
    let mut v128_seq = 0usize;
    // plan-32-D: the currently-accumulating maximal run of consecutive
    // RVV-lowerable v128 ops (flushed as one dual-path dispatch).
    let mut v128_run: Vec<(CodeOp, Vec<(&'static str, Operand)>)> = Vec::new();
    // Slots are recycled across non-overlapping live ranges, so the region size
    // is the peak concurrent slot count (highest index + 1), not the number of
    // distinct values.
    let peak_slots = v128_slots.values().map(|s| s + 1).max().unwrap_or(0);
    assert!(
        peak_slots <= super::v128::SLOT_COUNT,
        "rv64 v128: function needs {peak_slots} concurrent vector slots, exceeding the {}-slot region",
        super::v128::SLOT_COUNT
    );
    // A bare `cmp`/`cmp_imm` whose flag-reading branch is not adjacent (fusion
    // missed it) saves its left operand into `gp` (and, for a register rhs, spills
    // the right operand's value to the per-thread snapshot word — bug-381); the
    // standalone branch that follows consumes `pending` to build a native `rv.br`.
    // Multiple branches may read one compare (`gp`/the snapshot persist until the
    // next compare, a call, or a label).
    let mut pending: Option<FlagRhs> = None; // lhs is always GP; a register rhs is in the snapshot word
    for instruction in instructions {
        // plan-32-D: accumulate a maximal run of consecutive RVV-lowerable v128
        // ops into one dual-path dispatch. Any other instruction — or a v128 op
        // that isn't vector-lowered / a function that overflowed the vreg pool —
        // first flushes the pending run so the two arms reconcile at the slots.
        let v128_here = instruction
            .op
            .to_code()
            .filter(|c| super::v128::is_v128(*c));
        if let Some(code_op) = v128_here {
            if v128_vregs.is_some() && super::v128::rvv_lowerable(code_op, &instruction.fields) {
                v128_run.push((code_op, instruction.fields.clone()));
                continue;
            }
        }
        if !v128_run.is_empty() {
            if let Some(vregs) = v128_vregs.as_ref() {
                out.extend(super::v128::lower_v128_run(
                    &v128_run,
                    &v128_slots,
                    vregs,
                    v128_seq,
                ));
                v128_seq += 1;
            }
            v128_run.clear();
        }
        // A v128 op the run rejected (deferred op, or vreg-pool overflow):
        // scalarize it on its own (correctness-preserving; still bit-identical).
        if let Some(code_op) = v128_here {
            out.extend(super::v128::scalarize_v128(
                code_op,
                &instruction.fields,
                &v128_slots,
            ));
            continue;
        }
        // Bare (non-fused) integer compare: save the left operand into the flag
        // register `gp`. RISC-V has no flags, so the comparison itself emits
        // nothing yet — the following standalone branch reconstructs it.
        match instruction.op.to_code() {
            Some(CodeOp::Cmp) => {
                // Both operands are registers: keep them typed (bug-738).
                let lhs = field_operand(&instruction.fields, "lhs");
                let rhs = field_operand(&instruction.fields, "rhs");
                // Snapshot BOTH compared values at the compare (bug-381): lhs into
                // the reserved `gp`, rhs into the per-thread snapshot word. Keeping
                // the rhs by register name (the plan-99 original) stranded the
                // branch when the allocator reloaded that register in the span; the
                // by-value snapshots survive any intervening spill/reload or
                // redefinition of the original operands.
                out.push(snapshot_lhs(&lhs));
                out.extend(store_flag_rhs(&rhs));
                pending = Some(FlagRhs::Reg);
                continue;
            }
            Some(CodeOp::CmpImm) => {
                // The lhs is a register (typed, bug-738); the rhs an immediate.
                let lhs = field_operand(&instruction.fields, "lhs");
                let rhs = field_value(&instruction.fields, "rhs");
                out.push(snapshot_lhs(&lhs));
                pending = Some(if rhs == "0" {
                    FlagRhs::Zero
                } else {
                    FlagRhs::Imm(rhs)
                });
                continue;
            }
            Some(
                op @ (CodeOp::BranchEq
                | CodeOp::BranchNe
                | CodeOp::BranchGe
                | CodeOp::BranchLt
                | CodeOp::BranchGt
                | CodeOp::BranchLe
                | CodeOp::BranchHi
                | CodeOp::BranchLo
                | CodeOp::BranchLs),
            ) => {
                // A standalone integer flag-reading branch consuming a bare
                // compare. Re-derive the native compare-and-branch from `pending`.
                let target = field_value(&instruction.fields, "target");
                let rhs = pending.as_ref().unwrap_or_else(|| {
                    panic!(
                        "rv64: standalone flag branch without a preceding compare (to `{target}`)"
                    )
                });
                let cond = op.mnemonic();
                match rhs {
                    FlagRhs::Zero => {
                        let (a, b, rvcond) = int_branch(cond, GP, ZERO);
                        out.push(ci(
                            "rv.br",
                            &[
                                ("lhs", a),
                                ("rhs", b),
                                ("cond", rvcond),
                                ("target", &target),
                            ],
                        ));
                    }
                    FlagRhs::Imm(v) => {
                        out.push(ci("mov_imm", &[("dst", T0), ("value", v)]));
                        let (a, b, rvcond) = int_branch(cond, GP, T0);
                        out.push(ci(
                            "rv.br",
                            &[
                                ("lhs", a),
                                ("rhs", b),
                                ("cond", rvcond),
                                ("target", &target),
                            ],
                        ));
                    }
                    FlagRhs::Reg => {
                        // Reload the snapshotted rhs value (bug-381) into t0, then
                        // branch on gp vs t0 — reading the compared values, not the
                        // originals the allocator may have overwritten.
                        out.extend(load_flag_rhs(T0));
                        let (a, b, rvcond) = int_branch(cond, GP, T0);
                        out.push(ci(
                            "rv.br",
                            &[
                                ("lhs", a),
                                ("rhs", b),
                                ("cond", rvcond),
                                ("target", &target),
                            ],
                        ));
                    }
                }
                continue;
            }
            _ => {}
        }
        // bug-126.2 / bug-284 C3 / bug-381: a bare (non-fused) integer compare
        // defers its operands — the lhs snapshotted into `gp`, a register rhs
        // spilled to the per-thread snapshot word (bug-381) — for a later
        // standalone flag branch to re-read. Two situations still invalidate the
        // pending compare, because on NO architecture do flags survive them, so any
        // codegen reading them across one would be wrong regardless of the snapshot:
        //
        //  - A `label`: control flow can merge at the label, reaching the branch
        //    without having executed the compare, so the snapshots are stale on the
        //    merged-in path.
        //  - A `call`: a compare whose flag-reading branch sits across a call has no
        //    meaning on any target (a call clobbers flags), and the callee's own
        //    flag emulation would overwrite the shared per-thread snapshot word, so
        //    the snapshot does NOT survive a call. Detect it rather than miscompile.
        //
        // What is NO LONGER invalidated (bug-381): a redefinition of an original
        // operand *register* — a `dst`, an `add_carry` `carry_out`, a `sub_borrow`
        // `borrow_out`, or an allocator-inserted spill/reload of the operand. On the
        // flag machines that schedule is legal (redefining a GPR does not disturb
        // the flags), and rv64 must reproduce it correctly, not panic. It does: both
        // compared values were captured *by value* at the compare (lhs in `gp`, rhs
        // in the snapshot word), so a later redefinition of the original register
        // cannot touch them. This is the case bug-284 C3 could only *detect* (it
        // panicked); bug-381 makes it *recoverable*.
        if instruction.op.to_code() == Some(CodeOp::Label) {
            pending = None;
        } else if matches!(instruction.op, MirOp::Call | MirOp::CallIndirect) {
            pending = None;
        }
        if instruction.op == MirOp::AddrOf {
            // PC-relative symbol address as the `auipc; addi` pair (the encoder
            // realizes `adrp` as `auipc rd, %pcrel_hi` and `add_pageoff` as
            // `addi rd, rd, %pcrel_lo`). Emitting the pair (rather than a single
            // op) means a re-lowering pass re-fuses it to `addr_of` and this
            // expansion is a fixed point.
            let dst = instruction
                .fields
                .iter()
                .find(|(k, _)| *k == "dst")
                .map(|(_, v)| v.clone())
                .expect("addr_of carries a dst");
            let symbol = instruction
                .fields
                .iter()
                .find(|(k, _)| *k == "symbol")
                .map(|(_, v)| v.clone())
                .expect("addr_of carries a symbol");
            // Preserve the typed `dst` operand (a convention token stays
            // `Operand::Abi`, realized positionally by `remap_riscv_abi`, never a Raw
            // string — plan-85-D); only `symbol` is a name, so render it.
            out.push(
                CodeInstruction::new("adrp")
                    .physical_field("dst", &dst)
                    .field("symbol", symbol.render()),
            );
            out.push(
                CodeInstruction::new("add_pageoff")
                    .physical_field("dst", &dst)
                    .physical_field("src", &dst)
                    .field("symbol", symbol.render()),
            );
            continue;
        }
        if let Some(setter_op) = fused_setter_codeop(instruction.op) {
            // A shared branch re-emits the whole setter (RISC-V has no flags to
            // reuse). That is correct and cheap for a COMPARE (idempotent — it
            // does not write its operands), but an arithmetic overflow setter
            // (`adds`/`subs`) writes `dst`, so re-emitting it for a second consumer
            // with `dst` aliasing `lhs` would double-apply the add/sub (bug-126).
            // No builder emits a shared arithmetic setter today; fail loud rather
            // than silently miscompile if one ever does (the correct handling would
            // materialize the result + overflow word once and reuse them).
            if is_shared(&instruction.fields) && matches!(setter_op, CodeOp::Adds | CodeOp::Subs) {
                panic!(
                    "rv64: shared fused arithmetic setter '{}' would re-apply its \
                     add/subtract for the second branch; the flag-less RISC-V \
                     expansion is only idempotent for compares",
                    setter_op.mnemonic()
                );
            }
            out.extend(expand_fused(instruction.op, setter_op, &instruction.fields));
            continue;
        }
        // Non-fused MIR ops map 1:1 to a CodeOp via `to_code` (applying the
        // neutral→concrete renames, e.g. `call`→`bl`, `mulhi_u`→`umulh`); the
        // rv64 encoder realizes each CodeOp as RISC-V bytes. MOVE the field bag
        // instead of `code_fields_from_mir`'s `to_vec` clone (plan-84 Phase 2).
        let source = instruction.source;
        let op = instruction
            .op
            .to_code()
            .expect("non-fused MIR op maps to a single CodeOp");
        out.push(CodeInstruction {
            source,
            op,
            fields: instruction.fields,
        });
    }
    // plan-32-D: flush a v128 run that ends the function (no trailing non-v128 op).
    if !v128_run.is_empty() {
        if let Some(vregs) = v128_vregs.as_ref() {
            out.extend(super::v128::lower_v128_run(
                &v128_run,
                &v128_slots,
                vregs,
                v128_seq,
            ));
        }
    }
    // Realize the neutral arena base as the pinned `s11`.
    for instruction in &mut out {
        rename_operand_field_values(&mut instruction.fields, ARENA_BASE, ARENA_BASE_REGISTER);
    }
    remap_riscv_abi(&mut out);
    // plan-71-B Phase 1: the Category-2 self-move probe, after `remap_riscv_abi`
    // has realized the ABI role tokens to their lp64d homes (`%argK`/`%retK` →
    // the same `aN`). A same-index staging move would read as a `mov aN,aN`
    // no-op here. Report those under `MFB_BUG387_SELFMOVE`; unset, this reads
    // nothing and every emitted byte is unchanged (the scan never mutates `out`).
    if std::env::var_os("MFB_BUG387_SELFMOVE").is_some() {
        for line in crate::codegen::compiler::opt::bug387_selfmove_lines(&out, "riscv64") {
            eprintln!("{line}");
        }
    }
    out
}

/// The scratch register an AArch64 physical `xN` (N ≥ 9, N ≠ 30) maps to in a
/// hand-written helper (routed through `route_function_through_mir`, post-
/// allocation, so these are fixed physicals with no allocator to place them).
///
/// AArch64 treats `x9`–`x17` as caller-saved scratch and `x19`–`x28` as
/// callee-saved; a value parked across a `call` must survive, so `x19`–`x28` map
/// to the RISC-V callee-saved `s1`–`s10` (1:1). The caller scratch maps to the
/// four caller-saved temporaries `t3`–`t6` plus callee-saved substitutes
/// (over-preserving is safe). RISC-V has fewer non-ABI registers than AArch64
/// (15 vs 20 after reserving `a0`–`a7`, the `t0`–`t2` lowering scratch, and the
/// `s11` arena base), so a fully distinct 20-register mapping is impossible — but
/// no machine-floor helper uses more than 13 distinct scratch registers, and the
/// homes are arranged so every helper's *co-occurring* registers stay distinct:
/// `x9`–`x11` (used with the full callee set in `_main`) avoid all `s*`, and
/// `x12`–`x17` (used with at most `x19`/`x20` in the formatter/arena helpers)
/// avoid `s1`/`s2`. Shared homes (e.g. `x14`↔`x21` both `s3`) never appear in the
/// same function.
fn map_scratch_register(n: usize) -> String {
    match n {
        9 => "t3".to_string(),
        10 => "t4".to_string(),
        11 => "t5".to_string(),
        12 => "t6".to_string(),
        13 => "s0".to_string(),
        14 => "s3".to_string(),
        15 => "s4".to_string(),
        16 => "s5".to_string(),
        17 => "s6".to_string(),
        18 => "s7".to_string(),
        19..=28 => format!("s{}", n - 18), // x19→s1 … x28→s10
        _ => "t6".to_string(),             // x29 (fp) / stragglers — rare
    }
}

/// Map an AArch64 FP physical `dN` to its RISC-V FP ABI home. `d0`–`d7` are the
/// FP argument/return registers (`fa0`–`fa7`, matching AArch64); `d8`–`d15` are
/// callee-saved (`fs0`–`fs7`); higher numbers were the kernels' physical file,
/// now virtual registers, so they should not appear here.
fn map_fp_register(n: usize) -> String {
    match n {
        0..=7 => format!("fa{n}"),
        8..=15 => format!("fs{}", n - 8),
        // The math/SIMD kernels use FP virtual registers, so a physical d16+ is
        // unexpected in the selected stream. Map to the caller-saved ft bank
        // (skipping ft0/ft1/ft2, which the regmodel reserves for float-compare
        // staging and scalarized-v128 FMA lanes) if one ever appears — d16..d24 →
        // ft3..ft11 (bug-218).
        16..=24 => format!("ft{}", n - 16 + 3),
        other => panic!("rv64: unexpected physical FP register d{other}"),
    }
}

/// Remap the residual AArch64 physical registers the selected stream still
/// carries to their RISC-V lp64d homes. Virtual registers (`%vN`/`%fN`),
/// immediates, labels, and already-RISC-V names (`a0`, `t0`, `s11`, `sp`, …) pass
/// through. The mapping is purely positional (see the module comment): `xN → aN`,
/// with the fixed exceptions handled here.
fn remap_riscv_abi(instructions: &mut [CodeInstruction]) {
    for instruction in instructions.iter_mut() {
        for (_, value) in instruction.fields.iter_mut() {
            // A convention-explicit ABI token (plan-85-A) realizes positionally to
            // `x{index}` first (args and results coincide on the positional ABIs),
            // then the `remap_register` below maps that `xN`→`aN` exactly as it
            // does the legacy tokens — byte-identical.
            if let Operand::Abi { index, .. } = value {
                *value = Operand::from(crate::target::shared::abi::realize_abi_positional(*index));
            }
            // plan-34-B Phase-3b seam: realize a role token to its AArch64
            // spelling first (`%arg3` → `x3`), then the positional remap maps that
            // to the RISC-V home (`x3` → `a3`) exactly as today — byte-identical.
            if let Some(reg) = crate::target::shared::abi::realize_abi_token(&value.render()) {
                *value = Operand::from(reg);
            }
            if let Some(mapped) = remap_register(&value.render()) {
                *value = Operand::from(mapped);
            }
        }
    }
}

/// The RISC-V home for one operand value, or `None` if it is not an AArch64
/// physical register name (so it passes through unchanged).
pub(crate) fn remap_register(value: &str) -> Option<String> {
    match value {
        "sp" | "raw_sp" => return Some("sp".to_string()),
        // Neutral invariant-register tokens (plan-34-A). The legacy AArch64
        // spellings `"x31"`/`"x30"` are no longer accepted here: shared lowering
        // and the RISC-V platform backend both emit the tokens, and on RISC-V
        // `x31` is a *real* register (`t6`) — silently realizing a stray `"x31"`
        // as `t6` instead of `zero` is exactly the miscompile this narrowing
        // forecloses.
        "xzr" => return Some(ZERO.to_string()),
        "lr" => return Some("ra".to_string()),
        _ => {}
    }
    // Integer `xN`/`wN`.
    if let Some(n) = value
        .strip_prefix('x')
        .or_else(|| value.strip_prefix('w'))
        .and_then(|rest| rest.parse::<usize>().ok())
    {
        return Some(if n <= 7 {
            format!("a{n}") // x0–x7: argument / return / syscall-arg (all a-bank)
        } else if n == 8 {
            "a7".to_string() // syscall number register
        } else if n <= 29 {
            map_scratch_register(n)
        } else {
            // n >= 30: the link register and zero register are named by the
            // neutral `lr`/`xzr` tokens now (plan-34-A), matched above; a bare
            // `x30`/`x31` here is a stray physical name — deliberately mapped to
            // an invalid `a{n}` so the encoder rejects it loudly rather than
            // realizing `x31` as the real register `t6`.
            format!("a{n}")
        });
    }
    // FP `dN` (the AArch64 scalar-double physical bank).
    if let Some(n) = value
        .strip_prefix('d')
        .and_then(|rest| rest.parse::<usize>().ok())
    {
        return Some(map_fp_register(n));
    }
    None
}

#[cfg(test)]
mod tests;
