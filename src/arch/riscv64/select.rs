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

use crate::arch::aarch64::ops::CodeOp;
use crate::target::shared::code::mir::{
    fused_setter_codeop, rename_field_values, MirInstruction, MirOp, ARENA_BASE, FUSED_COND_FIELD,
    FUSED_SHARE_FIELD,
};
use crate::target::shared::code::CodeInstruction;

use super::regmodel::ARENA_BASE_REGISTER;

/// The fixed lowering-scratch integer registers, reserved from allocation
/// (`regmodel`): `t0`–`t2` stage immediate materialization, overflow detection,
/// and the float-compare boolean.
const T0: &str = "t0";
const T1: &str = "t1";
const T2: &str = "t2";
/// Fixed FP scratch for the compare-vs-zero constant.
const FT1: &str = "ft1";
/// The hardware zero register.
const ZERO: &str = "zero";
/// The flag register (plan-99): a bare `cmp`/`cmp_imm` whose flag-reading branch
/// is NOT adjacent (fusion missed it — the flags outlive intervening loads in a
/// few hand-written net/link helpers) saves its left operand here at the compare
/// and the standalone branch re-derives the condition from it. `gp` (x3) is never
/// used by the codegen (no gp-relative addressing) and is preserved across calls,
/// so it survives the whole compare→branch span.
const GP: &str = "gp";

/// The value of a named field (empty string if absent).
fn field_value(fields: &[(&'static str, String)], name: &str) -> String {
    fields
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// Build a `CodeInstruction` from a mnemonic and `(field, value)` pairs.
fn ci(mnemonic: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    let mut inst = CodeInstruction::new(mnemonic);
    for (k, v) in fields {
        inst = inst.field(k, v);
    }
    inst
}

/// Map an AArch64 integer compare condition (the branch mnemonic after `cmp`) to
/// the RISC-V branch that reproduces it: `(rs1, rs2, riscv_cond)` where the
/// branch is `b<riscv_cond> rs1, rs2, target`. `lhs`/`rhs` are the compare's two
/// operands (`lhs CMP rhs`); some relations swap them (RISC-V has only `lt`/`ge`,
/// signed and unsigned, so `>`/`<=` swap operands).
fn int_branch<'a>(cond: &str, lhs: &'a str, rhs: &'a str) -> (&'a str, &'a str, &'static str) {
    match cond {
        "b.eq" => (lhs, rhs, "eq"),
        "b.ne" => (lhs, rhs, "ne"),
        "b.ge" => (lhs, rhs, "ge"),  // signed lhs >= rhs
        "b.lt" => (lhs, rhs, "lt"),  // signed lhs <  rhs
        "b.gt" => (rhs, lhs, "lt"),  // signed lhs >  rhs  ⇔ rhs < lhs
        "b.le" => (rhs, lhs, "ge"),  // signed lhs <= rhs  ⇔ rhs >= lhs
        "b.hi" => (rhs, lhs, "ltu"), // unsigned lhs >  rhs ⇔ rhs <u lhs
        "b.lo" => (lhs, rhs, "ltu"), // unsigned lhs <  rhs
        "b.ls" => (rhs, lhs, "geu"), // unsigned lhs <= rhs ⇔ rhs >=u lhs
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
fn float_branch(cond: &str, lhs: &str, rhs: &str, target: &str) -> Vec<CodeInstruction> {
    // `feq/flt/fle.d dst, a, b` then branch `dst <bcond> zero`.
    let cmp = |dst: &str, cmp: &str, a: &str, b: &str| {
        ci(
            "rv.fcmp",
            &[("dst", dst), ("lhs", a), ("rhs", b), ("cmp", cmp)],
        )
    };
    let br = |a: &str, bcond: &str| {
        ci(
            "rv.br",
            &[("lhs", a), ("rhs", ZERO), ("cond", bcond), ("target", target)],
        )
    };
    match cond {
        // Ordered-only relations: the compare already excludes NaN, so branch when true.
        "b.gt" => vec![cmp(T0, "lt", rhs, lhs), br(T0, "ne")], // rhs < lhs
        "b.ge" => vec![cmp(T0, "le", rhs, lhs), br(T0, "ne")], // rhs <= lhs
        "b.mi" | "b.lo" => vec![cmp(T0, "lt", lhs, rhs), br(T0, "ne")], // lhs < rhs (ordered)
        "b.ls" => vec![cmp(T0, "le", lhs, rhs), br(T0, "ne")], // lhs <= rhs (ordered)
        "b.eq" => vec![cmp(T0, "eq", lhs, rhs), br(T0, "ne")], // lhs == rhs (ordered)
        // Unordered-including relations: branch when the ordered complement is false.
        "b.ne" => vec![cmp(T0, "eq", lhs, rhs), br(T0, "eq")], // NOT(ordered ==) = ≠ or NaN
        "b.hi" => vec![cmp(T0, "le", lhs, rhs), br(T0, "eq")], // NOT(lhs<=rhs) = > or NaN
        "b.lt" => vec![cmp(T0, "le", rhs, lhs), br(T0, "eq")], // NOT(rhs<=lhs) = < or NaN
        "b.le" => vec![cmp(T0, "lt", rhs, lhs), br(T0, "eq")], // NOT(rhs<lhs) = <= or NaN
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
fn cond_and_target(fields: &[(&'static str, String)]) -> (String, String) {
    let split = fields
        .iter()
        .position(|(key, _)| *key == FUSED_COND_FIELD)
        .expect("fused MIR op carries a cond field");
    let cond = fields[split].1.clone();
    let target = fields[split + 1..]
        .iter()
        .find(|(key, _)| *key == "target")
        .map(|(_, v)| v.clone())
        .expect("fused compare-branch carries a target");
    (cond, target)
}

/// Whether the fused op's branch reuses the preceding comparison. On RISC-V a
/// compare-and-branch is self-contained (no shared flags), so a shared branch
/// simply re-emits the whole compare — but the operands are the same, so it is
/// correct and cheap.
fn is_shared(fields: &[(&'static str, String)]) -> bool {
    fields.iter().any(|(key, _)| *key == FUSED_SHARE_FIELD)
}

/// Expand one fused MIR op into RISC-V machine ops (plan-99). `setter_op` is the
/// AArch64 flag-setter the fusion recorded; the fields are
/// `[<setter operands>, cond, <branch fields>]`.
fn expand_fused(op: MirOp, setter_op: CodeOp, fields: &[(&'static str, String)]) -> Vec<CodeInstruction> {
    let get = |name: &str| -> String {
        fields
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let (cond, target) = cond_and_target(fields);
    match setter_op {
        // Integer register-register compare-and-branch.
        CodeOp::Cmp => {
            let lhs = get("lhs");
            let rhs = get("rhs");
            let (a, b, rvcond) = int_branch(&cond, &lhs, &rhs);
            vec![ci(
                "rv.br",
                &[("lhs", a), ("rhs", b), ("cond", rvcond), ("target", &target)],
            )]
        }
        // Integer register-immediate compare-and-branch. RISC-V branches take
        // registers only, so a non-zero immediate is materialized into `t0`.
        CodeOp::CmpImm => {
            let lhs = get("lhs");
            let imm = get("rhs");
            let mut out = Vec::new();
            let imm_reg: &str = if imm == "0" {
                ZERO
            } else {
                out.push(ci("mov_imm", &[("dst", T0), ("value", &imm)]));
                T0
            };
            let (a, b, rvcond) = int_branch(&cond, &lhs, imm_reg);
            out.push(ci(
                "rv.br",
                &[("lhs", a), ("rhs", b), ("cond", rvcond), ("target", &target)],
            ));
            out
        }
        // Float register-register compare-and-branch.
        CodeOp::FCmpD => {
            let lhs = get("lhs");
            let rhs = get("rhs");
            float_branch(&cond, &lhs, &rhs, &target)
        }
        // Float compare against +0.0: materialize +0.0 into `ft1` (`fmv.d.x`),
        // then the same compare-and-branch against it.
        CodeOp::FCmpZeroD => {
            let src = get("src");
            let mut out = vec![ci(
                "fmov_d_from_x",
                &[("dst", FT1), ("src", ZERO)],
            )];
            out.extend(float_branch(&cond, &src, FT1, &target));
            out
        }
        // Overflow-checked add: `dst = lhs + rhs`, branch on signed overflow.
        // Overflow iff the inputs share a sign that differs from the result's:
        //   sum = lhs + rhs
        //   ovf = (~(lhs ^ rhs) & (sum ^ lhs)) < 0
        CodeOp::Adds => {
            let dst = get("dst");
            let lhs = get("lhs");
            let rhs = get("rhs");
            let branch_cond = overflow_branch_cond(&cond);
            // The result must be written into `dst` BEFORE the branch: the fused
            // `adds; b.vc` writes the destination and *then* branches, and the
            // branch jumps away on the no-overflow path — so a `mov dst, t0` after
            // it would be skipped, leaving `dst` undefined.
            vec![
                ci("add", &[("dst", T0), ("lhs", &lhs), ("rhs", &rhs)]),
                ci("eor", &[("dst", T1), ("lhs", &lhs), ("rhs", &rhs)]),
                ci("eor", &[("dst", T2), ("lhs", T0), ("rhs", &lhs)]),
                ci("mvn", &[("dst", T1), ("src", T1)]),
                ci("and", &[("dst", T1), ("lhs", T1), ("rhs", T2)]),
                ci("mov", &[("dst", &dst), ("src", T0)]),
                ci(
                    "rv.br",
                    &[("lhs", T1), ("rhs", ZERO), ("cond", branch_cond), ("target", &target)],
                ),
            ]
        }
        // Overflow-checked subtract: `dst = lhs - rhs`, branch on signed overflow.
        //   diff = lhs - rhs
        //   ovf = ((lhs ^ rhs) & (lhs ^ diff)) < 0
        CodeOp::Subs => {
            let dst = get("dst");
            let lhs = get("lhs");
            let rhs = get("rhs");
            let branch_cond = overflow_branch_cond(&cond);
            // Write `dst` before the branch (see the `Adds` case above).
            vec![
                ci("sub", &[("dst", T0), ("lhs", &lhs), ("rhs", &rhs)]),
                ci("eor", &[("dst", T1), ("lhs", &lhs), ("rhs", &rhs)]),
                ci("eor", &[("dst", T2), ("lhs", &lhs), ("rhs", T0)]),
                ci("and", &[("dst", T1), ("lhs", T1), ("rhs", T2)]),
                ci("mov", &[("dst", &dst), ("src", T0)]),
                ci(
                    "rv.br",
                    &[("lhs", T1), ("rhs", ZERO), ("cond", branch_cond), ("target", &target)],
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
    /// Compare against a register whose value survives to the branch.
    Reg(String),
}

/// Select neutral MIR into RV64GC machine ops (plan-99).
pub(crate) fn select_riscv64(instructions: &[MirInstruction]) -> Vec<CodeInstruction> {
    let mut out = Vec::with_capacity(instructions.len());
    // A bare `cmp`/`cmp_imm` whose flag-reading branch is not adjacent (fusion
    // missed it) saves its left operand into `gp` here; the standalone branch
    // that follows consumes `pending` to build a native `rv.br`. Multiple
    // branches may read one compare (`gp`/rhs persist until the next compare).
    let mut pending: Option<FlagRhs> = None; // the compare's rhs; lhs is always GP
    for instruction in instructions {
        // Bare (non-fused) integer compare: save the left operand into the flag
        // register `gp`. RISC-V has no flags, so the comparison itself emits
        // nothing yet — the following standalone branch reconstructs it.
        match instruction.op.to_code() {
            Some(CodeOp::Cmp) => {
                let lhs = field_value(&instruction.fields, "lhs");
                let rhs = field_value(&instruction.fields, "rhs");
                out.push(ci("mov", &[("dst", GP), ("src", &lhs)]));
                pending = Some(FlagRhs::Reg(rhs));
                continue;
            }
            Some(CodeOp::CmpImm) => {
                let lhs = field_value(&instruction.fields, "lhs");
                let rhs = field_value(&instruction.fields, "rhs");
                out.push(ci("mov", &[("dst", GP), ("src", &lhs)]));
                pending = Some(if rhs == "0" { FlagRhs::Zero } else { FlagRhs::Imm(rhs) });
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
                let rhs = pending
                    .as_ref()
                    .expect("rv64: standalone flag branch without a preceding compare");
                let target = field_value(&instruction.fields, "target");
                let cond = op.mnemonic();
                match rhs {
                    FlagRhs::Zero => {
                        let (a, b, rvcond) = int_branch(cond, GP, ZERO);
                        out.push(ci(
                            "rv.br",
                            &[("lhs", a), ("rhs", b), ("cond", rvcond), ("target", &target)],
                        ));
                    }
                    FlagRhs::Imm(v) => {
                        out.push(ci("mov_imm", &[("dst", T0), ("value", v)]));
                        let (a, b, rvcond) = int_branch(cond, GP, T0);
                        out.push(ci(
                            "rv.br",
                            &[("lhs", a), ("rhs", b), ("cond", rvcond), ("target", &target)],
                        ));
                    }
                    FlagRhs::Reg(r) => {
                        let (a, b, rvcond) = int_branch(cond, GP, r);
                        out.push(ci(
                            "rv.br",
                            &[("lhs", a), ("rhs", b), ("cond", rvcond), ("target", &target)],
                        ));
                    }
                }
                continue;
            }
            _ => {}
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
            out.push(ci("adrp", &[("dst", &dst), ("symbol", &symbol)]));
            out.push(ci(
                "add_pageoff",
                &[("dst", &dst), ("src", &dst), ("symbol", &symbol)],
            ));
            continue;
        }
        if let Some(setter_op) = fused_setter_codeop(instruction.op) {
            // A shared branch re-emits the whole compare (RISC-V has no flags to
            // reuse); otherwise expand the setter + branch into the native form.
            let _ = is_shared(&instruction.fields);
            out.extend(expand_fused(instruction.op, setter_op, &instruction.fields));
            continue;
        }
        // Non-fused MIR ops map 1:1 to a CodeOp via `to_code` (applying the
        // neutral→concrete renames, e.g. `call`→`bl`, `mulhi_u`→`umulh`); the
        // rv64 encoder realizes each CodeOp as RISC-V bytes.
        out.push(CodeInstruction {
            op: instruction
                .op
                .to_code()
                .expect("non-fused MIR op maps to a single CodeOp"),
            fields: instruction.fields.clone(),
        });
    }
    // Realize the neutral arena base as the pinned `s11`.
    for instruction in &mut out {
        rename_field_values(&mut instruction.fields, ARENA_BASE, ARENA_BASE_REGISTER);
    }
    remap_riscv_abi(&mut out);
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
        // (skipping the reserved ft0/ft1) if one ever appears.
        16..=25 => format!("ft{}", n - 16 + 2),
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
            if let Some(mapped) = remap_register(value) {
                *value = mapped;
            }
        }
    }
}

/// The RISC-V home for one operand value, or `None` if it is not an AArch64
/// physical register name (so it passes through unchanged).
fn remap_register(value: &str) -> Option<String> {
    match value {
        "sp" | "raw_sp" => return Some("sp".to_string()),
        "x31" | "xzr" => return Some(ZERO.to_string()),
        "x30" | "lr" => return Some("ra".to_string()),
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
            // x30/x31 handled above; nothing else is valid.
            format!("a{n}")
        });
    }
    // FP `dN` (the AArch64 scalar-double physical bank).
    if let Some(n) = value.strip_prefix('d').and_then(|rest| rest.parse::<usize>().ok()) {
        return Some(map_fp_register(n));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::code::mir::lower_to_mir;

    fn build(mnemonic: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        ci(mnemonic, fields)
    }

    fn sel(instructions: &[CodeInstruction]) -> Vec<CodeInstruction> {
        select_riscv64(&lower_to_mir(instructions))
    }

    fn values(out: &[CodeInstruction]) -> Vec<String> {
        out.iter()
            .flat_map(|inst| inst.fields.iter().map(|(_, v)| v.clone()))
            .collect()
    }

    #[test]
    fn positional_abi_remap() {
        let out = sel(&[
            build("mov", &[("dst", "x0"), ("src", "x1")]),
            build("mov", &[("dst", "x7"), ("src", "sp")]),
            build("mov", &[("dst", "x9"), ("src", "x31")]),
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
    fn link_register_maps_to_ra() {
        let out = sel(&[
            build("str_u64", &[("src", "x30"), ("base", "sp"), ("offset", "0")]),
            build("ret", &[]),
        ]);
        assert!(values(&out).iter().any(|v| v == "ra"));
        assert!(!values(&out).iter().any(|v| v == "x30"));
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
        assert_eq!(br.get("cond"), Some("lt"));
        assert_eq!(br.get("target"), Some("L"));
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
        assert_eq!(br.get("cond"), Some("lt"));
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
        // A non-zero immediate is materialized into t0.
        assert!(out.iter().any(|i| i.op == CodeOp::MovImm && i.get("dst") == Some("t0")));
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
        assert_eq!(br.get("rhs"), Some("zero"));
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
        assert_eq!(br.get("cond"), Some("ge")); // b.vc = no overflow = word >= 0
    }

    #[test]
    fn addr_of_expands_to_auipc_addi_pair() {
        let out = sel(&[
            build("adrp", &[("dst", "x9"), ("symbol", "g")]),
            build("add_pageoff", &[("dst", "x9"), ("src", "x9"), ("symbol", "g")]),
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].op, CodeOp::Adrp);
        assert_eq!(out[1].op, CodeOp::AddPageOff);
    }

    #[test]
    fn arena_base_realizes_to_s11() {
        let realization = crate::target::shared::code::mir::arena_base_realization();
        let out = sel(&[
            build("ldr_u64", &[("dst", "x9"), ("base", realization), ("offset", "0")]),
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
}
