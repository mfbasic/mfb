//! x86-64 instruction selection (plan-00-H): neutral MIR → x86-64 machine ops.
//!
//! The x86 counterpart of `arch::aarch64::select`. It consumes the shared
//! neutral [`MirInstruction`] stream (via `mir::Backend::select`) and produces
//! [`CodeInstruction`]s with x86/SysV registers, using the shared MIR primitives
//! (`fused_setter_codeop`, `rename_field_values`, …) — so all the ISA-specific
//! selection lives here, not in shared `mir.rs`.

use crate::arch::aarch64::ops::CodeOp;
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
    // rax and rdx are excluded: `mul`/`imul`/`div`/`idiv`/`cqo` use them
    // *implicitly* (dividend/quotient in rax, high-half/remainder in rdx), so a
    // long-lived scratch value mapped there would be silently destroyed across a
    // division or wide multiply — e.g. the digit-loop divisor `10` in
    // `emit_write_integer_to_stderr` lived across the `div` that clobbers rdx.
    //
    // Ordering matters: the hand-written helpers inherit the AArch64 convention
    // that x19–x28 are *callee-saved* — values parked there survive an
    // intervening `call`/`syscall` (e.g. the entry's error message in x20 across
    // the code-printing `write` syscall, which clobbers rcx; argc/argv in x27/x28
    // across `clock_gettime`). So the pool is arranged so those high registers
    // land on x86's callee-saved bank (rbx/rbp/r12/r13): with the `(n-9) % 11`
    // index, x20→rbx, x27→r12, x28→r13, x19→rbp. The low scratch (x8–x18, not
    // parked across calls) takes the caller-saved remainder (rcx/rsi/rdi/r8–r11).
    const POOL: &[&str] = &[
        "rbx", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13", "rcx", "rbp",
    ];
    POOL[(n - 9) % POOL.len()]
}

// SysV: call args rdi,rsi,rdx,rcx,r8,r9; syscall args rdi,rsi,rdx,r10,r8,r9;
// returns rax,rdx; syscall nr + result rax.
// SysV integer argument registers, extended with two INTERNAL argument
// registers for `x6`/`x7`: MFBASIC functions take up to 8 parameters and
// AArch64 has 8 argument registers, but SysV only has 6 — so internal calls
// pass the 7th in `rax` (dead at a call site: the variadic al marker is only
// emitted for external libc calls, see the `bl` encoder) and the 8th in `rbp`
// (reserved, never allocated, and no vregified builder function names it).
// Libc calls never exceed 6 integer args, so the extension is internal-only
// in practice.
const CALL_ARGS: &[&str] = &["rdi", "rsi", "rdx", "rcx", "r8", "r9", "rax", "rbp"];
const SYS_ARGS: &[&str] = &["rdi", "rsi", "rdx", "r10", "r8", "r9"];
// x0/x1 are the SysV return registers (rax/rdx). x2/x3 extend the set only for
// the runtime's 4-register error-Result convention (tag=x0, value=x1,
// message=x2, source=x3), which `make_error_result` produces and the error/TRAP
// path consumes immediately (no intervening call), so caller-saved rcx/rsi are
// safe distinct homes. Without these, x2/x3 fell back to rax and aliased,
// corrupting propagated errors.
const RETS: &[&str] = &["rax", "rdx", "rcx", "rsi"];

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
            // No following boundary: a leftover ABI register used as a plain value
            // — most often a call RESULT whose boundary the dataflow lost (e.g. an
            // arena pointer `x1` copied in a loop, where the loop back-edge poisons
            // `boundary_before` so `is_result` is false at the copy). Fall back to
            // that index's RESULT register (`x1`→rdx), NOT always rax — mapping a
            // leftover `x1` to rax (the OK tag = 0) gave a null-dst copy → SIGSEGV
            // in the datetime/json/regex/lambda/resource record builders.
            None => RETS.get(n).copied().unwrap_or("rax"),
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
    // return address — so drop the frame's LR save/restore entirely. Shared code
    // now spells it with the neutral `abi::LR` token (`"lr"`); the `"x30"`
    // spelling is still accepted from any non-shared producer (plan-34-A).
    instructions
        .retain(|inst| !inst.fields.iter().any(|(_, value)| value == "x30" || value == "lr"));

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
        i > 0 && instructions[i - 1].op != CodeOp::Branch && instructions[i - 1].op != CodeOp::Ret
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
            // is `None` until a predecessor is seen. A call/syscall boundary wins
            // over a no-boundary path: the error-Result staging block
            // (`raw_conversion_done_0`) is entered both from `make_error_result`
            // calls (which deliver x0–x3 in RETS) AND by fall-through from the
            // success path (which sets x0/x1 manually with no call). Both paths
            // MUST read x0–x3 from RETS to match the callee, so the block's
            // in-effect boundary is the call.
            let mut merged: Option<Option<AbiBoundary>> = None;
            let mut absorb = |val: Option<AbiBoundary>| match merged {
                None => merged = Some(val),
                Some(cur) => {
                    merged = Some(match (cur, val) {
                        (Some(a), _) => Some(a), // a boundary wins over anything
                        (None, other) => other,
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
    // A block-entry index resets the linear def state: either it is reached ONLY
    // through branches (no fall-through), or it is a MERGE — reachable by a branch
    // from another block as well as by fall-through. In the merge case the
    // fall-through path's defs did not happen on the branch-in paths, so a use
    // here must not be treated as "still defined since the boundary" based on the
    // fall-through predecessor alone (that is what let the error-Result staging
    // stores read CALL_ARGS instead of the call's RETS).
    let block_entry: Vec<bool> = (0..count)
        .map(|i| !falls_into(i) || branch_preds.contains_key(&i))
        .collect();

    // A def of `xK` (K < RETS.len()) is a *staged result* — part of the
    // 4-register error-Result convention — when the first thing that consumes it
    // along control flow is a result-read USE: a use in a block whose in-effect
    // boundary is a call/syscall (e.g. `error_label`'s `store x1,[arena+32]` and
    // `mov x20,x2`), reached BEFORE the value flows into any call/syscall
    // boundary. Such a def must be colored `RETS[K]` to agree with that consumer.
    // `next_after` alone would see the later code-printing `write` syscall and
    // miscolor the def `SYS_ARGS[K]` (rdi/rsi/rdx) — so the exit-range error
    // report would store the message pointer into the code slot and read the
    // message back from the wrong register. A value consumed directly BY a
    // boundary instead (the program-exit code handed to the `exit` syscall with
    // no intervening use) is NOT staged and keeps its arg mapping.
    let def_is_staged_result = |def_idx: usize, n: usize| -> bool {
        let target = format!("x{n}");
        // BFS over control flow following BOTH edges of conditional branches:
        // the SIMD binary tail's select (`ldr x0,[a]; …; b.le done; mov x0,x1;
        // done: str x0`) delivers the first def to the reset-block store along
        // the TAKEN edge while the fall-through path redefines it — the def is
        // staged if ANY path reaches a qualifying reset-block result read.
        let mut work: Vec<(usize, bool)> = vec![(def_idx + 1, false)];
        let mut seen = std::collections::HashSet::new();
        let mut staged = false;
        while let Some((mut j, mut entered_reset)) = work.pop() {
            loop {
                if j >= count || !seen.insert((j, entered_reset)) {
                    break;
                }
                // Passing into a reset block (a branch-only target or a merge)
                // means a use there is colored `is_result` (its
                // `defined_since_boundary` is cleared) — so the def must be RETS
                // to match. A read in the SAME straight-line block does not
                // finalize the coloring — the value stays live past it (the
                // entry's exit path is `mov x0,x1; cmp x0,255; ja …; jmp
                // exit_label`, where the decisive consumer is the exit label's
                // arena-staging store); such a use is forced to the matching
                // RETS color by `staged_live` below.
                if block_entry[j] {
                    entered_reset = true;
                }
                if abi_boundary_of(&instructions[j]).is_some() {
                    break; // this path consumes it at a call/syscall boundary
                }
                let mut reads = false;
                let mut redefines = false;
                for (k, v) in &instructions[j].fields {
                    if v == &target {
                        if X86_DEF_FIELDS.contains(k) {
                            redefines = true;
                        } else {
                            reads = true;
                        }
                    }
                }
                if reads
                    && entered_reset
                    && matches!(
                        boundary_before[j],
                        Some(AbiBoundary::Call) | Some(AbiBoundary::Syscall)
                    )
                {
                    staged = true;
                    break;
                }
                if redefines {
                    break; // overwritten on this path before a deciding use
                }
                if instructions[j].op == CodeOp::Branch {
                    match branch_target(j) {
                        Some(t) => j = t,
                        None => break,
                    }
                } else {
                    // A conditional branch (any non-Branch op with a target —
                    // b.cc / cbz-style / x86.jcc) forks: queue the taken edge
                    // and continue on the fall-through.
                    if let Some(t) = branch_target(j) {
                        work.push((t, entered_reset));
                    }
                    j += 1;
                }
            }
            if staged {
                break;
            }
        }
        staged
    };
    let staged_result_def: Vec<bool> = (0..count)
        .map(|i| {
            let def_n = instructions[i].fields.iter().find_map(|(k, v)| {
                if X86_DEF_FIELDS.contains(k) {
                    v.strip_prefix('x')
                        .and_then(|rest| rest.parse::<usize>().ok())
                        .filter(|n| *n < RETS.len())
                } else {
                    None
                }
            });
            match def_n {
                Some(n) => def_is_staged_result(i, n),
                None => false,
            }
        })
        .collect();

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

    // Registers whose live def was colored RETS as a staged result: same-block
    // uses (the exit path's `cmp x0,255` between the staged `mov x0,x1` and the
    // exit label's arena-staging store) must read the SAME register the def
    // wrote, not the role-based coloring (whose next boundary is the shutdown
    // call, giving CALL_ARGS). Cleared on redefinition, at boundaries, and at
    // block entries (a reset block's uses are colored `is_result` = RETS
    // directly, so the def and the cross-block consumer already agree).
    let mut staged_live: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for i in 0..count {
        let role = next_after[i];
        // A block reached only by branches starts its def tracking fresh (its
        // linear predecessor is a different control-flow path).
        if block_entry[i] {
            defined_since_boundary.clear();
            staged_live.clear();
        }
        let mut new_defs: Vec<String> = Vec::new();
        let mut new_def_ns: Vec<usize> = Vec::new();
        for (key, value) in instructions[i].fields.iter_mut() {
            if value == "sp" {
                *value = "rsp".to_string();
                continue;
            }
            if value == "x31" {
                // The legacy zero spelling → the neutral zero token, which the
                // encoder emits as an immediate zero (`store xzr` → `mov r/m, 0`).
                // r14 is no longer pinned at 0 (plan-34-C freed it for allocation),
                // so x31 must NOT map to r14 — that now holds an allocated value.
                *value = crate::target::shared::abi::ZERO.to_string();
                continue;
            }
            // Physical FP registers `dN` (the AArch64 double bank, used by the
            // float builders/kernels) map 1:1 to `xmmN`. The `vN`/`qN` SIMD banks
            // alias the same register file (NEON `v`/`q` = the `d` register's full
            // 128 bits), so they map to the same `xmmN`. FP virtual registers
            // (`%fN`) are colored to xmm by the allocator and pass through here.
            // `abi::FP_SCRATCH` tokens (plan-34-D) never reach this arm: the
            // Phase-3b seam in `select_x86` realizes every token to its AArch64
            // spelling (`%fscratch0` → `d0`) before `remap_x86_abi` runs.
            if let Some(fp) = value
                .strip_prefix(['d', 'v', 'q'])
                .and_then(|rest| rest.parse::<usize>().ok())
                .filter(|n| *n < 16)
            {
                *value = format!("xmm{fp}");
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
            // x0/x1 are the standard results; x2/x3 are results only for the
            // 4-register error-Result convention (a callee that returns them
            // without the caller redefining them since the call — regular calls
            // return x0/x1, so this only fires for a propagated error).
            let is_result = !is_def
                && n < RETS.len()
                && !defined_since_boundary.contains(value)
                && matches!(
                    boundary_before[i],
                    Some(AbiBoundary::Call) | Some(AbiBoundary::Syscall)
                );
            // An incoming parameter USE reached before any def of `xK` and before
            // any call/syscall boundary consumes the SysV-delivered value, which
            // lives in `CALL_ARGS[k]` (rdi, rsi, …). The role-based `map_abi_register`
            // resolves `xK`'s home by the NEXT boundary its value flows into, but a
            // parameter that is spilled straight to a stack slot (e.g. the Fixed
            // toString formatter's `str x0,[sp]; str x1,[sp+8]` prologue) has no such
            // downstream boundary along its control-flow path, so the role collapses
            // to `None` → the rax fallback. Two such params (x0 and x1) then both map
            // to rax, and the incoming-param bridge emits `mov rax,rdi; mov rax,rsi`,
            // clobbering the first before its store — corrupting both spilled values.
            // Pin such a use to its argument register so the store reads the real
            // parameter and `param_home == arg` suppresses a bogus bridge.
            let is_param_use =
                !is_def && n <= 7 && !boundary_since_entry && !defined_since_entry.contains(&n);
            let mapped = if is_def && n < RETS.len() && staged_result_def[i] {
                staged_live.insert(n);
                RETS[n].to_string()
            } else if !is_def && n < RETS.len() && staged_live.contains(&n) {
                // A same-block use of a staged-result def reads the register the
                // def actually wrote (RETS), not the role-based coloring.
                RETS[n].to_string()
            } else if is_param_use {
                CALL_ARGS
                    .get(n)
                    .map(|reg| reg.to_string())
                    .unwrap_or_else(|| map_abi_register(n, role, is_result))
            } else {
                if is_def {
                    staged_live.remove(&n);
                }
                map_abi_register(n, role, is_result)
            };
            if is_param_use {
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
                staged_live.clear();
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

/// Rewrite the flag-reading branch of a fused *float* compare into the x86
/// branch(es) that read `ucomisd`'s CF/ZF/PF with IEEE-754 unordered semantics.
///
/// After `ucomisd lhs, rhs` (`lhs` vs `rhs`): `CF=1` iff `lhs<rhs` or unordered;
/// `ZF=1` iff `lhs=rhs` or unordered; `PF=1` iff unordered (either is NaN). The
/// AArch64 `b.cc` mnemonics were chosen for `fcmp`'s NZCV, which differs, so the
/// integer `b.cc → jcc` mapping mishandles every NaN case. The mapping below
/// reproduces each AArch64 float relation's *exact* truth set on x86:
///
/// - `>`/`>=` (`b.gt`/`b.ge`) → `ja`/`jae`: `CF=0` already excludes unordered.
/// - `<`/`<=`/`=` (`b.mi`/`b.ls`/`b.eq`) → `jp skip; jb|jbe|je target; skip:`:
///   `jb`/`jbe`/`je` alone would also fire on unordered (CF/ZF set), so a leading
///   `jp` skips the branch when unordered (PF=1), yielding the ordered-only set.
/// - `<>` (`b.ne`) → `jp target; jne target`: true on unordered *or* ordered-≠.
/// - `b.lt`/`b.le` (integer-style `<`/`<=`, unordered ⇒ true) → `jb`/`jbe`.
/// - `b.vs`/`b.vc` (NaN / not-NaN finiteness checks) → `jp`/`jnp`.
///
/// `site` is a per-function index that makes each synthesized skip label unique.
/// Naming it from `target` alone let two ordered-only branches to the same label
/// (e.g. `IF a < b OR c < d THEN GOTO L`) emit two labels of the same name; the
/// encoder's name-keyed label map is last-writer-wins, so the first `jp` resolved
/// to the *second* label and a NaN first operand jumped clean over the second
/// comparison (bug-15).
fn x86_float_branch(cond: &str, target: &str, site: usize) -> Vec<CodeInstruction> {
    // Emit ONLY `x86.*`-namespaced branches: this function's output is re-lowered
    // (`route_function_through_mir`) after selection, and a real AArch64 `b.cc`
    // sitting right after the `fcmp` would re-fuse and be remapped a second time.
    // The `x86.*` ops are not flag-reading branches for `lower_to_mir`, so the
    // stream is a fixed point on the second pass.
    let br = |mnemonic: &str, tgt: &str| CodeInstruction::new(mnemonic).field("target", tgt);
    // `jp skip; <cc> target; skip:` — take <cc> only when ordered (PF clear).
    let ordered_only = |cc: &str| {
        let skip = format!("{target}__x86ford{site}");
        vec![
            br("x86.jp", &skip),
            br(cc, target),
            CodeInstruction::new("label").field("name", &skip),
        ]
    };
    match cond {
        "b.gt" => vec![br("x86.ja", target)], // ja  (CF=0 && ZF=0)          {GT}
        "b.ge" => vec![br("x86.jae", target)], // jae (CF=0)                 {EQ,GT}
        "b.mi" => ordered_only("x86.jb"),     // jb  (CF=1), NaN-excluded     {LT}
        "b.lo" => ordered_only("x86.jb"),     // b.lo(C=0)==LT after fcmp     {LT}
        "b.ls" => ordered_only("x86.jbe"),    // jbe (CF=1 || ZF=1), NaN-excl {LT,EQ}
        "b.eq" => ordered_only("x86.je"),     // je  (ZF=1), NaN-excluded     {EQ}
        "b.ne" => vec![br("x86.jp", target), br("x86.jne", target)], // jp||jne {LT,GT,uno}
        "b.hi" => vec![br("x86.jp", target), br("x86.ja", target)], // jp||ja  {GT,uno}
        "b.lt" => vec![br("x86.jb", target)], // jb  (CF=1) — LT or unordered {LT,uno}
        "b.le" => vec![br("x86.jbe", target)], // jbe (CF=1 || ZF=1)          {LT,EQ,uno}
        "b.vs" => vec![br("x86.jp", target)], // jp  (PF=1 → unordered/NaN)   {uno}
        "b.vc" => vec![br("x86.jnp", target)], // jnp (PF=0 → ordered)        {LT,EQ,GT}
        other => panic!("unmapped x86 float-compare branch condition '{other}'"),
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
    // Distinguishes the skip label of every ordered-only float branch in this
    // function (see `x86_float_branch`).
    let mut float_branch_site = 0_usize;
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
            // A branch reading a float compare's flags needs the x86 IEEE remap:
            // `ucomisd` sets CF/ZF/PF (not the AArch64 NZCV the `b.cc` mnemonics
            // read), and an unordered (NaN) result sets CF=ZF=PF=1, so the naive
            // integer `b.cc → jcc` mapping mishandles every NaN case. Rewrite the
            // branch here where the setter kind is known.
            if matches!(setter_op, CodeOp::FCmpD | CodeOp::FCmpZeroD) {
                let target = branch_fields
                    .iter()
                    .find(|(k, _)| *k == "target")
                    .map(|(_, v)| v.clone())
                    .expect("float compare branch carries a target");
                for inst in x86_float_branch(&instruction.fields[split].1, &target, float_branch_site)
                {
                    out.push(inst);
                }
                float_branch_site += 1;
            } else {
                out.push(CodeInstruction {
                    op: branch_op,
                    fields: branch_fields,
                });
            }
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
        // plan-34-B Phase-3b seam: realize a role token to its AArch64 spelling
        // (`%arg3` → `x3`) so `remap_x86_abi`'s existing role inference reproduces
        // today's result exactly (byte-identical). Phase 4 replaces the inference
        // with a direct token→SysV lookup and drops this.
        for (_, value) in instruction.fields.iter_mut() {
            if let Some(reg) = crate::target::shared::abi::realize_abi_token(value) {
                *value = reg.to_string();
            }
        }
    }
    remap_x86_abi(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::code::mir::lower_to_mir;

    /// Build one aarch64-form `CodeInstruction`.
    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut ins = CodeInstruction::new(op);
        for (k, v) in fields {
            ins = ins.field(k, v);
        }
        ins
    }

    /// Select a stream from aarch64-form instructions.
    fn sel(instructions: &[CodeInstruction]) -> Vec<CodeInstruction> {
        select_x86(&lower_to_mir(instructions))
    }

    /// Every field value in the selected stream, flattened.
    fn values(out: &[CodeInstruction]) -> Vec<String> {
        out.iter()
            .flat_map(|inst| inst.fields.iter().map(|(_, v)| v.clone()))
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
    fn call_argument_and_return_mapping() {
        // x0..x5 set before a `bl` are call arguments (rdi, rsi, rdx, rcx, r8, r9);
        // x0/x1 read after the call are results (rax, rdx).
        let out = sel(&[
            ci("mov_imm", &[("dst", "x0"), ("value", "1")]),
            ci("mov_imm", &[("dst", "x1"), ("value", "2")]),
            ci("mov_imm", &[("dst", "x2"), ("value", "3")]),
            ci("mov_imm", &[("dst", "x3"), ("value", "4")]),
            ci("mov_imm", &[("dst", "x4"), ("value", "5")]),
            ci("mov_imm", &[("dst", "x5"), ("value", "6")]),
            ci("bl", &[("target", "_mfb_f")]),
            ci("mov", &[("dst", "x9"), ("src", "x0")]),
            ci("ret", &[]),
        ]);
        let vals = values(&out);
        for arg in ["rdi", "rsi", "rdx", "rcx", "r8", "r9"] {
            assert!(vals.contains(&arg.to_string()), "missing arg {arg}");
        }
        // x0 read after the call → rax (result).
        assert!(vals.contains(&"rax".to_string()));
    }

    #[test]
    fn internal_seventh_eighth_call_args() {
        // x6/x7 as internal call args map to rax and rbp.
        let out = sel(&[
            ci("mov_imm", &[("dst", "x6"), ("value", "7")]),
            ci("mov_imm", &[("dst", "x7"), ("value", "8")]),
            ci("bl", &[("target", "_mfb_f")]),
            ci("ret", &[]),
        ]);
        let vals = values(&out);
        assert!(vals.contains(&"rax".to_string()));
        assert!(vals.contains(&"rbp".to_string()));
    }

    #[test]
    fn syscall_argument_and_number_mapping() {
        // Before an `svc`: x0..x5 are syscall args (rdi rsi rdx r10 r8 r9) and x8
        // is the syscall number (rax).
        let out = sel(&[
            ci("mov_imm", &[("dst", "x0"), ("value", "1")]),
            ci("mov_imm", &[("dst", "x3"), ("value", "4")]),
            ci("mov_imm", &[("dst", "x8"), ("value", "60")]),
            ci("svc", &[]),
            ci("ret", &[]),
        ]);
        let vals = values(&out);
        assert!(vals.contains(&"rdi".to_string()));
        assert!(vals.contains(&"r10".to_string())); // x3 syscall arg
        assert!(vals.contains(&"rax".to_string())); // x8 syscall number
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
    fn map_abi_register_fallbacks() {
        // Out-of-range indices fall back to rax in every role.
        assert_eq!(map_abi_register(9, Some(AbiBoundary::Call), false), "rax");
        assert_eq!(
            map_abi_register(9, Some(AbiBoundary::Syscall), false),
            "rax"
        );
        assert_eq!(map_abi_register(9, Some(AbiBoundary::Ret), false), "rax");
        assert_eq!(map_abi_register(9, None, false), "rax");
        assert_eq!(map_abi_register(9, Some(AbiBoundary::Call), true), "rax");
        // A leftover ABI register with no boundary uses that index's RETS home.
        assert_eq!(map_abi_register(1, None, false), "rdx");
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
            .map(|i| i.fields[0].1.clone())
            .collect();
        assert_eq!(labels.len(), 3, "two skip labels + the shared target: {labels:?}");
        let skips: Vec<&String> = labels.iter().filter(|n| n.contains("__x86ford")).collect();
        assert_eq!(skips.len(), 2);
        assert_ne!(skips[0], skips[1], "skip labels collide: {skips:?}");
        // Each `jp` targets its own skip label, which sits right after its `jb`.
        let jps: Vec<&String> = out
            .iter()
            .filter(|i| i.op.mnemonic() == "x86.jp")
            .map(|i| &i.fields[0].1)
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
    fn incoming_parameter_bridge_prologue() {
        // A function that reads x1 as an incoming parameter (spilled to a slot
        // with no downstream call) gets a `mov <home>, rsi` bridge inserted after
        // the entry label.
        let out = sel(&[
            ci("label", &[("name", "entry")]),
            ci("str_u64", &[("src", "x1"), ("base", "sp"), ("offset", "8")]),
            ci("ret", &[]),
        ]);
        // The prologue bridge copies from the SysV arg register rsi.
        assert!(values(&out).iter().any(|v| v == "rsi"));
        // The first instruction is still the entry label.
        assert_eq!(out[0].op, CodeOp::Label);
    }

    #[test]
    fn staged_error_result_def_and_use_colored_rets() {
        // A def of x1 after a call whose only consumer is a result-read in a
        // branch-only reset block (whose in-effect boundary is that call) is a
        // staged error-Result: both the def and the cross-block read take the
        // RETS coloring (x1 → rdx), matching the callee.
        let out = sel(&[
            ci("bl", &[("target", "_mfb_make_err")]),
            ci("mov", &[("dst", "x1"), ("src", "x9")]), // staged def of x1
            ci("b", &[("target", "stage")]),
            ci("label", &[("name", "dead")]),
            ci("ret", &[]),
            ci("label", &[("name", "stage")]),
            ci("str_u64", &[("src", "x1"), ("base", "sp"), ("offset", "0")]), // result read
            ci("ret", &[]),
        ]);
        // x1 was colored rdx (RETS[1]) at both the def and the reset-block store.
        let vals = values(&out);
        assert!(vals.contains(&"rdx".to_string()));
        assert!(!vals.iter().any(|v| v == "x1"));
    }

    #[test]
    fn same_block_staged_use_reads_rets() {
        // A staged-result def followed by a same-block use (before the deciding
        // reset-block read) reads the register the def wrote (RETS), via the
        // `staged_live` branch.
        let out = sel(&[
            ci("bl", &[("target", "_mfb_make_err")]),
            ci("mov", &[("dst", "x0"), ("src", "x9")]), // staged def of x0
            ci("cmp", &[("lhs", "x0"), ("rhs", "x10")]), // same-block use of x0
            ci("b", &[("target", "stage")]),
            ci("label", &[("name", "stage")]),
            ci("str_u64", &[("src", "x0"), ("base", "sp"), ("offset", "0")]),
            ci("ret", &[]),
        ]);
        // x0 stays as rax (RETS[0]) throughout — no x0 residue.
        assert!(!values(&out).iter().any(|v| v == "x0"));
    }

    #[test]
    fn branch_to_missing_label_has_no_boundary() {
        // A branch whose target label is absent yields no resolvable boundary
        // (exercises the `branch_target -> None` fall-throughs).
        let out = sel(&[
            ci("mov_imm", &[("dst", "x0"), ("value", "1")]),
            ci("b", &[("target", "nowhere")]),
            ci("ret", &[]),
        ]);
        // Still selects; x0 with no downstream boundary uses its RETS home (rax).
        assert!(values(&out).iter().any(|v| v == "rax"));
    }

    #[test]
    fn arena_base_realizes_to_r15() {
        // The AArch64 arena-base realization register, once lowered to the neutral
        // `arena_base` and selected, becomes the x86 pin r15.
        let realization = crate::target::shared::code::mir::arena_base_realization();
        let out = sel(&[
            ci(
                "ldr_u64",
                &[("dst", "x9"), ("base", realization), ("offset", "0")],
            ),
            ci("ret", &[]),
        ]);
        assert!(values(&out).iter().any(|v| v == "r15"));
    }
}
