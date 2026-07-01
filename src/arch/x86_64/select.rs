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

// SysV: call args rdi,rsi,rdx,rcx,r8,r9; syscall args rdi,rsi,rdx,r10,r8,r9;
// returns rax,rdx; syscall nr + result rax.
const CALL_ARGS: &[&str] = &["rdi", "rsi", "rdx", "rcx", "r8", "r9"];
const SYS_ARGS: &[&str] = &["rdi", "rsi", "rdx", "r10", "r8", "r9"];
const RETS: &[&str] = &["rax", "rdx"];

/// Map an AArch64 ABI register `xN` (N ≤ 8) to its SysV/x86-64 home given its
/// role: an argument flowing into the next call/syscall, a return value, or a
/// result coming out of a preceding call/syscall.
fn map_abi_register(n: usize, role: Option<AbiBoundary>, is_result: bool) -> String {
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
    // The boundary each register's value flows into, resolved along CONTROL FLOW
    // (not just linear order). A value set right before `b <label>` flows to the
    // branch target, so an unconditional branch must be followed — otherwise a
    // return value set before `b <ret_label>` would be misread as an argument to
    // whatever call happens to sit linearly after the branch (e.g. the grow
    // block after `arena_alloc_done`), sending the status/pointer to `rdi`/`rsi`
    // instead of `rax`/`rdx`.
    let label_index: std::collections::HashMap<&str, usize> = instructions
        .iter()
        .enumerate()
        .filter(|(_, inst)| inst.op == CodeOp::Label)
        .filter_map(|(i, inst)| {
            inst.fields
                .iter()
                .find(|(key, _)| *key == "name")
                .map(|(_, name)| (name.as_str(), i))
        })
        .collect();
    let branch_target = |i: usize| -> Option<usize> {
        instructions[i]
            .fields
            .iter()
            .find(|(key, _)| *key == "target")
            .and_then(|(_, name)| label_index.get(name.as_str()).copied())
    };
    // First boundary reached when execution begins at index `start`, following
    // fall-through and unconditional branches (a cycle with no boundary → None).
    let first_boundary_from = |start: usize| -> Option<AbiBoundary> {
        let mut j = start;
        let mut seen = vec![false; count];
        loop {
            if j >= count || seen[j] {
                return None;
            }
            seen[j] = true;
            if let Some(b) = abi_boundary_of(&instructions[j]) {
                return Some(b);
            }
            if instructions[j].op == CodeOp::Branch {
                match branch_target(j) {
                    Some(target) => j = target,
                    None => return None,
                }
            } else {
                j += 1;
            }
        }
    };
    // Nearest boundary strictly AFTER each index (the one its value flows into),
    // where "after" follows the control transfer that index performs.
    let next_after: Vec<Option<AbiBoundary>> = (0..count)
        .map(|i| {
            let next = if instructions[i].op == CodeOp::Branch {
                branch_target(i).unwrap_or(count)
            } else {
                i + 1
            };
            first_boundary_from(next)
        })
        .collect();

    // The call/syscall boundary in effect when CONTROL FLOW reaches each index —
    // the mirror of `next_after`, for the result direction. An `x0`/`x1` read at a
    // point whose boundary is a call/syscall is that call's result. Computed as a
    // forward dataflow over the CFG (not the linear predecessor): a label reached
    // only through `b.eq call_ok` (its fall-through blocked by the error path's
    // `ret`) inherits the boundary from the branch source — the original call —
    // instead of whatever call the error path happened to make (e.g. `arena_free`
    // in a scope-drop), which would make the result read the wrong register.
    let mut branch_preds: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for j in 0..count {
        if let Some(target) = branch_target(j) {
            branch_preds.entry(target).or_default().push(j);
        }
    }
    let falls_into = |i: usize| -> bool {
        // The instruction before index i transfers control to i by fall-through
        // unless it ends the block (an unconditional branch or a return).
        i > 0
            && instructions[i - 1].op != CodeOp::Branch
            && instructions[i - 1].op != CodeOp::Ret
    };
    let out_boundary = |i: usize, before: Option<AbiBoundary>| -> Option<AbiBoundary> {
        match abi_boundary_of(&instructions[i]) {
            Some(b @ (AbiBoundary::Call | AbiBoundary::Syscall)) => Some(b),
            _ => before, // a `ret`/non-boundary passes the incoming context through
        }
    };
    let mut boundary_before: Vec<Option<AbiBoundary>> = vec![None; count];
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..count {
            // Merge the boundary out of every control-flow predecessor. `merged`
            // is `None` until a predecessor is seen; a conflict (a boundary path
            // meeting a no-boundary path) collapses to `None`.
            let mut merged: Option<Option<AbiBoundary>> = None;
            let mut absorb = |val: Option<AbiBoundary>| match merged {
                None => merged = Some(val),
                Some(cur) => {
                    merged = Some(match (cur, val) {
                        (Some(_), Some(_)) => cur, // two boundaries: result-equivalent
                        _ => None,
                    })
                }
            };
            if falls_into(i) {
                absorb(out_boundary(i - 1, boundary_before[i - 1]));
            }
            if let Some(preds) = branch_preds.get(&i) {
                for &j in preds {
                    absorb(out_boundary(j, boundary_before[j]));
                }
            }
            let new_val = merged.unwrap_or(None);
            if new_val != boundary_before[i] {
                boundary_before[i] = new_val;
                changed = true;
            }
        }
    }
    // A block-entry index is reached only through branches (no fall-through) — its
    // linear def state belongs to a different path, so the walk resets there.
    let block_entry: Vec<bool> = (0..count).map(|i| !falls_into(i)).collect();

    // Walk forward tracking, per ABI register, whether it has been (re)defined
    // since the last boundary — an `x0`/`x1` USE not redefined since its CFG
    // boundary is that call's result. `defined_since_boundary` is reset at a
    // branch-entered block, since its defs come from a different linear path.
    let mut defined_since_boundary: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    // Incoming-parameter tracking, scoped to the whole function (not reset at
    // boundaries). An incoming parameter is a *live-in* ABI register: `xK`
    // (K ≤ 7) read before it is defined and before any call/syscall. SysV
    // delivers it in `CALL_ARGS[K]` (rdi, rsi, …), but a vreg-pure helper copies
    // it into a vreg at entry via `mov %vK, xK`, where the body maps that `xK`
    // use by its role (e.g. `rax` in a call-free leaf). We bridge the two with a
    // `mov <home>, CALL_ARGS[K]` prologue so the copy reads the real argument.
    let mut defined_since_entry: std::collections::HashSet<usize> =
        std::collections::HashSet::new();
    let mut boundary_since_entry = false;
    let mut param_home: std::collections::BTreeMap<usize, String> =
        std::collections::BTreeMap::new();

    for i in 0..count {
        let role = next_after[i];
        // A block reached only by branches starts its def tracking fresh (its
        // linear predecessor is a different control-flow path).
        if block_entry[i] {
            defined_since_boundary.clear();
        }
        let mut new_defs: Vec<String> = Vec::new();
        let mut new_def_ns: Vec<usize> = Vec::new();
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
                && matches!(
                    boundary_before[i],
                    Some(AbiBoundary::Call) | Some(AbiBoundary::Syscall)
                );
            let mapped = map_abi_register(n, role, is_result);
            if !is_def && n <= 7 && !boundary_since_entry && !defined_since_entry.contains(&n) {
                param_home.entry(n).or_insert_with(|| mapped.clone());
            }
            if is_def {
                new_defs.push(value.clone());
                new_def_ns.push(n);
            }
            *value = mapped;
        }
        match abi_boundary_of(&instructions[i]) {
            // Only a call/syscall produces an x0/x1 result and opens a new result
            // context. A `ret` does NOT — and crucially the error-check path puts
            // a `ret` between a call and the `call_ok` label where its result is
            // consumed, so treating `ret` as the last boundary would misread the
            // result as an argument to the *next* call.
            Some(AbiBoundary::Call | AbiBoundary::Syscall) => {
                boundary_since_entry = true;
                defined_since_boundary.clear();
            }
            Some(AbiBoundary::Ret) => {}
            None => {
                for def in new_defs {
                    defined_since_boundary.insert(def);
                }
            }
        }
        // A definition retires `xK` as an incoming-parameter candidate for the
        // rest of the function, regardless of boundaries.
        for n in new_def_ns {
            defined_since_entry.insert(n);
        }
    }

    // Bridge each incoming parameter from its SysV argument register into the
    // register the body addresses it by. A parameter the body already reads from
    // its arg register (`home == CALL_ARGS[k]`, the common case for helpers that
    // pass it straight into a nested call) needs no copy.
    let mut prologue: Vec<CodeInstruction> = Vec::new();
    for (k, home) in &param_home {
        let Some(arg) = CALL_ARGS.get(*k) else {
            continue;
        };
        if home == arg {
            continue;
        }
        prologue.push(CodeInstruction {
            op: CodeOp::from_mnemonic("mov").expect("x86 has a register-move op"),
            fields: vec![("dst", home.clone()), ("src", (*arg).to_string())],
        });
    }
    if !prologue.is_empty() {
        // Insert after the leading `entry` label; the frame `sub_sp` only touches
        // rsp, so the copies may precede it. The arg registers are still live.
        let at = usize::from(
            instructions
                .first()
                .map(|inst| inst.op == CodeOp::Label)
                .unwrap_or(false),
        );
        for (offset, inst) in prologue.into_iter().enumerate() {
            instructions.insert(at + offset, inst);
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
