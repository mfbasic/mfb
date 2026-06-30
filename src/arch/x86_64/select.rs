//! x86-64 instruction selection (plan-00-H): neutral MIR → x86-64 machine ops.
//!
//! The x86 counterpart of `arch::aarch64::select`. It consumes the shared
//! neutral [`MirInstruction`] stream (via `mir::Backend::select`) and produces
//! [`CodeInstruction`]s with x86/SysV registers, using the shared MIR primitives
//! (`fused_setter_codeop`, `rename_field_values`, …) — so all the ISA-specific
//! selection lives here, not in shared `mir.rs`.

use crate::arch::aarch64::ops::CodeOp;
use crate::arch::x86_64::regmodel::ZERO_REGISTER;
use crate::target::shared::code::mir::{
    fused_setter_codeop, MirInstruction, MirOp, ARENA_BASE, FUSED_COND_FIELD, FUSED_SHARE_FIELD,
};
use crate::target::shared::code::CodeInstruction;

/// A call/return boundary that fixes the SysV ABI role of an `x0`–`x8` operand.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AbiBoundary {
    Call,
    Syscall,
    Ret,
}

fn abi_boundary_of(instruction: &CodeInstruction) -> Option<AbiBoundary> {
    match instruction.op {
        CodeOp::BranchLink | CodeOp::BranchLinkRegister => Some(AbiBoundary::Call),
        CodeOp::Svc => Some(AbiBoundary::Syscall),
        CodeOp::Ret => Some(AbiBoundary::Ret),
        _ => None,
    }
}

const X86_DEF_FIELDS: &[&str] = &["dst", "carry_out", "borrow_out"];

/// Map residual AArch64 scratch `xN` (N ≥ 9) to an x86 GPR (encoding-only; see
/// the call site). Avoids `r14` (zero), `r15` (arena_base), and `rsp`.
fn map_scratch_register(n: usize) -> &'static str {
    const POOL: &[&str] = &[
        "rax", "rcx", "rdx", "rsi", "rdi", "r8", "r9", "r10", "r11", "rbx", "r12", "r13", "rbp",
    ];
    POOL[(n - 9) % POOL.len()]
}

/// Map an AArch64 ABI register `xN` (N ≤ 8) to its SysV/x86-64 home given its
/// role: an argument flowing into the next call/syscall, a return value, or a
/// result coming out of a preceding call/syscall.
fn map_abi_register(n: usize, role: Option<AbiBoundary>, is_result: bool) -> String {
    // SysV: call args rdi,rsi,rdx,rcx,r8,r9; syscall args rdi,rsi,rdx,r10,r8,r9;
    // returns rax,rdx; syscall nr + result rax.
    const CALL_ARGS: &[&str] = &["rdi", "rsi", "rdx", "rcx", "r8", "r9"];
    const SYS_ARGS: &[&str] = &["rdi", "rsi", "rdx", "r10", "r8", "r9"];
    const RETS: &[&str] = &["rax", "rdx"];
    let reg = if is_result {
        RETS.get(n).copied().unwrap_or("rax")
    } else {
        match role {
            Some(AbiBoundary::Call) => CALL_ARGS.get(n).copied().unwrap_or("rax"),
            Some(AbiBoundary::Syscall) if n == 8 => "rax", // syscall number
            Some(AbiBoundary::Syscall) => SYS_ARGS.get(n).copied().unwrap_or("rax"),
            Some(AbiBoundary::Ret) => RETS.get(n).copied().unwrap_or("rax"),
            // No following boundary: a leftover ABI register used as a plain
            // value (rare). Fall back to the return register.
            None => "rax",
        }
    };
    reg.to_string()
}

/// Remap the residual AArch64 physical registers a selected stream still carries
/// (the ABI registers `x0`–`x8`, `sp`, `xzr`/`x31`, the link register `x30`, and
/// leftover scratch) to their x86-64 / SysV homes. Virtual registers (`%vN`) and
/// `arena_base` (already realized to `r15`) pass through. The hard case is
/// `x0`–`x8`, whose role depends on the nearest call/`svc`/`ret` boundary.
fn remap_x86_abi(instructions: &mut Vec<CodeInstruction>) {
    // The link register has no x86 equivalent — `call` pushes / `ret` pops the
    // return address — so drop the frame's x30 save/restore entirely.
    instructions.retain(|inst| !inst.fields.iter().any(|(_, value)| value == "x30"));

    let count = instructions.len();
    // Nearest boundary strictly AFTER each index (the one an argument flows into).
    let mut next_after: Vec<Option<AbiBoundary>> = vec![None; count];
    let mut carry = None;
    for i in (0..count).rev() {
        next_after[i] = carry;
        if let Some(b) = abi_boundary_of(&instructions[i]) {
            carry = Some(b);
        }
    }

    // Walk forward tracking, per ABI register, whether it has been (re)defined
    // since the last boundary — an `x0`/`x1` USE not redefined since a preceding
    // call/syscall is that call's result.
    let mut defined_since_boundary: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut last_boundary: Option<AbiBoundary> = None;

    for i in 0..count {
        let role = next_after[i];
        let mut new_defs: Vec<String> = Vec::new();
        for (key, value) in instructions[i].fields.iter_mut() {
            if value == "sp" {
                *value = "rsp".to_string();
                continue;
            }
            if value == "x31" {
                *value = ZERO_REGISTER.to_string();
                continue;
            }
            let Some(n) = value
                .strip_prefix('x')
                .and_then(|rest| rest.parse::<usize>().ok())
                .filter(|n| *n <= 30)
            else {
                continue;
            };
            if n > 8 {
                // Residual AArch64 caller/callee-saved scratch (`x9`–`x30`). The
                // vreg-migrated helpers are mostly pure, but a few (arena_alloc's
                // reserved-survivor save/restore around its nested fill call,
                // errno bridges) still name physical scratch. Map it to an x86
                // GPR so it ENCODES; such helpers may not be correct on x86 yet
                // (Phase 1 runs integer programs that don't call them), tracked
                // as the helper-purity follow-up.
                *value = map_scratch_register(n).to_string();
                continue;
            }
            let is_def = X86_DEF_FIELDS.contains(key);
            let is_result = !is_def
                && (n == 0 || n == 1)
                && !defined_since_boundary.contains(value)
                && matches!(last_boundary, Some(AbiBoundary::Call) | Some(AbiBoundary::Syscall));
            let mapped = map_abi_register(n, role, is_result);
            if is_def {
                new_defs.push(value.clone());
            }
            *value = mapped;
        }
        match abi_boundary_of(&instructions[i]) {
            // Only a call/syscall produces an x0/x1 result and opens a new result
            // context. A `ret` does NOT — and crucially the error-check path puts
            // a `ret` between a call and the `call_ok` label where its result is
            // consumed, so treating `ret` as the last boundary would misread the
            // result as an argument to the *next* call.
            Some(b @ (AbiBoundary::Call | AbiBoundary::Syscall)) => {
                last_boundary = Some(b);
                defined_since_boundary.clear();
            }
            Some(AbiBoundary::Ret) => {}
            None => {
                for def in new_defs {
                    defined_since_boundary.insert(def);
                }
            }
        }
    }
}

/// Select neutral MIR into x86-64 machine ops (plan-00-H). Mirrors the AArch64
/// selection's structural conversion — `addr_of` becomes a single RIP-relative
/// load (`adrp{dst,symbol}`, which the x86 encoder emits as `lea`; the page-pair
/// `add_pageoff` is unused), a fused flagless op splits into its `cmp`/`adds`/…
/// setter + the flag-reading branch (x86 `cmp; jcc` works the same way), and
/// `arena_base` realizes to the pinned `r15` — then remaps the residual AArch64
/// ABI registers to their SysV homes ([`remap_x86_abi`]).
pub(crate) fn select_x86(instructions: &[MirInstruction]) -> Vec<CodeInstruction> {
    let mut out = Vec::with_capacity(instructions.len());
    for instruction in instructions {
        if instruction.op == MirOp::AddrOf {
            // Single RIP-relative reference (no aarch64 page pair): the x86
            // encoder turns `adrp{dst,symbol}` into `lea dst,[rip+disp32]`.
            out.push(CodeInstruction {
                op: CodeOp::Adrp,
                fields: instruction.fields.clone(),
            });
            continue;
        }
        if let Some(setter_op) = fused_setter_codeop(instruction.op) {
            let split = instruction
                .fields
                .iter()
                .position(|(key, _)| *key == FUSED_COND_FIELD)
                .expect("fused MIR op carries a cond field");
            let setter_fields = instruction.fields[..split].to_vec();
            let branch_op = CodeOp::from_mnemonic(&instruction.fields[split].1)
                .expect("fused MIR op carries a valid branch mnemonic");
            let mut branch_fields = Vec::new();
            let mut shared = false;
            for (key, value) in &instruction.fields[split + 1..] {
                if *key == FUSED_SHARE_FIELD {
                    shared = true;
                } else {
                    branch_fields.push((*key, value.clone()));
                }
            }
            if !shared {
                out.push(CodeInstruction {
                    op: setter_op,
                    fields: setter_fields,
                });
            }
            out.push(CodeInstruction {
                op: branch_op,
                fields: branch_fields,
            });
        } else {
            // Non-fused MIR ops map 1:1 to a CodeOp via `to_code` (which applies
            // the neutral→concrete renames, e.g. `call`→`bl`); the x86 encoder
            // realizes each CodeOp as x86 bytes.
            out.push(CodeInstruction {
                op: instruction
                    .op
                    .to_code()
                    .expect("non-fused MIR op maps to a single CodeOp"),
                fields: instruction.fields.clone(),
            });
        }
    }
    for instruction in &mut out {
        crate::target::shared::code::mir::rename_field_values(
            &mut instruction.fields,
            ARENA_BASE,
            "r15",
        );
    }
    remap_x86_abi(&mut out);
    out
}
