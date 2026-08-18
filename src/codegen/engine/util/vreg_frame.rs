//! Split from `src/target/shared/code/codegen_utils.rs` (category `engine.util`).

// --- codegen tier imports (migration) ---
use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::regalloc;
use crate::codegen::engine::types::*;
use crate::target::shared::abi;
pub(crate) fn finalize_frame(
    instructions: &mut Vec<CodeInstruction>,
    stack_slots: &mut [CodeStackSlot],
    local_stack_size: usize,
    mut callee_saved: Vec<String>,
) -> CodeFrame {
    let has_calls = instructions.iter().any(|instruction| {
        instruction.op == CodeOp::BranchLink || instruction.op == CodeOp::BranchLinkRegister
    });
    if has_calls
        && !callee_saved
            .iter()
            .any(|register| register == abi::link_register())
    {
        callee_saved.push(abi::link_register().to_string());
    }
    // Per-register save-area offsets (bug-124.2). An AArch64 FP/SIMD callee-saved
    // register (`d8`–`d15`) can hold a 128-bit `v128` value live across a call, so
    // it is saved with the 128-bit `str q`/`ldr q` into a 16-byte, 16-aligned slot
    // — a 64-bit `str d` would drop lane[1] and corrupt the vector::/math-array
    // kernels. Every other callee-saved register (integer, and RISC-V's 64-bit
    // `fs*` FP scalars — no 128-bit SIMD on that path) keeps an 8-byte slot, so a
    // target with no 128-bit FP callee-saved register lays out `index * 8` exactly
    // as before and stays byte-identical. `outgoing_bytes` is 16-aligned, so an
    // FP slot placed at a 16-aligned running offset is 16-aligned overall, which
    // the `str q` scaled immediate requires.
    let mut callee_offsets: Vec<usize> = Vec::with_capacity(callee_saved.len());
    let mut save_cursor = 0usize;
    for register in &callee_saved {
        if is_simd128_callee_saved(register) {
            save_cursor = align(save_cursor, 16);
            callee_offsets.push(save_cursor);
            save_cursor += 16;
        } else {
            callee_offsets.push(save_cursor);
            save_cursor += 8;
        }
    }
    // Rounded to 16 so the shift below keeps every 16-aligned spill offset
    // 16-aligned (the spill area sits above this callee-saved area).
    let save_size = align(save_cursor, 16);
    // A called function on x86-64 must offset its 16-aligned frame by the pushed
    // return address so rsp is 16-aligned at its own call sites (0 on AArch64).
    let call_padding = if has_calls {
        crate::codegen::engine::mir::active_backend().frame_call_padding()
    } else {
        0
    };
    // Outgoing stack-argument tail (bug-08): the widest call in this function that
    // passes more than 8 arguments needs its extra arguments laid out at `[sp+0..]`
    // at the moment of the call, so reserve that many bytes at the very bottom of
    // the frame (below the callee-saved area). 16-aligned to keep the save area's
    // alignment and the stack pointer 16-aligned at call sites. Zero — and the
    // whole frame byte-identical to the register-only convention — when no call
    // passes stack arguments.
    // Win64 (plan-47-B §4.3): a callee may spill its four register arguments into
    // a 32-byte "shadow" region the caller reserves below the first stack
    // argument, so a calling frame owes those bytes even with no stack tail.
    // `shadow_space_bytes()` defaults to 0, so SysV/AAPCS64 frames are unchanged.
    let shadow = if has_calls {
        crate::codegen::engine::mir::active_backend().shadow_space_bytes()
    } else {
        0
    };
    let outgoing_bytes = match max_outgoing_arg_offset(instructions) {
        Some(max_offset) => align(shadow + max_offset + 8, 16),
        // A leaf-calling Win64 frame with no stack tail still owes the shadow space.
        None if shadow > 0 => shadow,
        None => 0,
    };
    let total_stack_size = outgoing_bytes + align(save_size + local_stack_size, 16) + call_padding;
    if total_stack_size == 0 {
        return CodeFrame {
            stack_size: 0,
            callee_saved,
        };
    }

    // Body `sp`-relative accesses and stack-slot metadata clear both the outgoing
    // tail (frame bottom) and the callee-saved area above it.
    let body_shift = outgoing_bytes + save_size;
    for slot in stack_slots {
        slot.offset += body_shift as i32;
    }
    adjust_stack_instruction_offsets(instructions, body_shift);
    #[cfg(debug_assertions)]
    assert_stack_accesses_fit_frame(instructions, total_stack_size);

    // Resolve the incoming/outgoing stack-argument sentinels now that the final
    // frame size is known (bug-08). Incoming arguments sit above the whole frame,
    // past the entry return-address padding (8 on x86-64, 0 on AArch64); outgoing
    // arguments sit at the reserved frame bottom (`[sp+0..]`, already unshifted).
    if outgoing_bytes != 0 || has_incoming_stack_args(instructions) {
        let entry_padding = crate::codegen::engine::mir::active_backend().frame_call_padding();
        resolve_stack_arg_sentinels(instructions, total_stack_size, entry_padding);
    }

    let mut prologue = Vec::new();
    prologue.push(abi::subtract_stack(total_stack_size));
    for (index, register) in callee_saved.iter().enumerate() {
        prologue.push(save_callee_saved(
            register,
            outgoing_bytes + callee_offsets[index],
        ));
    }

    let insert_at = if instructions
        .first()
        .is_some_and(|instruction| instruction.op == CodeOp::Label)
    {
        1
    } else {
        0
    };
    instructions.splice(insert_at..insert_at, prologue);

    let mut rewritten = Vec::new();
    for instruction in instructions.drain(..) {
        if instruction.op == CodeOp::Ret {
            for (index, register) in callee_saved.iter().enumerate().rev() {
                rewritten.push(restore_callee_saved(
                    register,
                    outgoing_bytes + callee_offsets[index],
                ));
            }
            rewritten.push(abi::add_stack(total_stack_size));
            rewritten.push(instruction);
        } else {
            rewritten.push(instruction);
        }
    }
    *instructions = rewritten;

    CodeFrame {
        stack_size: total_stack_size,
        callee_saved,
    }
}

/// Allocate registers for a hand-written runtime helper whose body is built with
/// **virtual registers** (`%vN`/`%fN`) and finalize its frame — the same pipeline
/// the builder functions use (`regalloc::allocate` + [`finalize_frame`]). This
/// lets a helper be written in target-neutral vreg MIR (no fixed `x9`/`v22`…) so
/// the shared allocator places its registers per-ISA, which is what makes the
/// helpers portable (plan-00-G Phase 2). Physical operands the body still names —
/// `arena_base` (the reserved arena register), the ABI `x0`–`x7` it loads call
/// args into and reads results from — stay physical (the allocator never colors
/// them, and the call clobber model spills any vreg live across a `bl`/`svc`).
/// The helper makes no use of eager hints; it has no declared params (it uses the
/// ABI registers directly).
pub(crate) fn finalize_vreg_helper(
    name: &str,
    symbol: &str,
    returns: &str,
    mut instructions: Vec<CodeInstruction>,
    relocations: Vec<CodeRelocation>,
) -> CodeFunction {
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    CodeFunction {
        name: name.to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: returns.to_string(),
        frame,
        instructions,
        relocations,
        stack_slots,
    }
}

/// Run the shared allocator (`regalloc::allocate`) + frame builder
/// ([`finalize_frame`]) over a vreg-built helper body in place, returning the
/// resulting frame and spill stack slots. The building block of
/// [`finalize_vreg_helper`]; used directly by helpers that produce their
/// `CodeFunction` fields (params, name) at the call site rather than here.
pub(crate) fn finalize_vreg_body(
    instructions: &mut Vec<CodeInstruction>,
    reserved: &[&str],
) -> (CodeFrame, Vec<CodeStackSlot>) {
    finalize_vreg_body_with_locals(instructions, reserved, 0)
}

/// Like [`finalize_vreg_body`], but reserves `local_size` bytes of explicit
/// `sp`-relative scratch *below* the spill area for a helper that needs a fixed
/// on-stack buffer (e.g. a `stat`/`getcwd`/`readdir` struct a syscall fills). The
/// helper addresses that buffer at offsets `0..local_size` from `sp`; the spills
/// the allocator adds land at `local_size` and up, and [`finalize_frame`] shifts
/// every `sp`-relative access (buffer and spill alike) past the callee-saved area
/// uniformly, so the two never overlap. `local_size` is rounded up to 16 to keep
/// the spill area 8-aligned and the buffer suitably aligned.
pub(crate) fn finalize_vreg_body_with_locals(
    instructions: &mut Vec<CodeInstruction>,
    reserved: &[&str],
    local_size: usize,
) -> (CodeFrame, Vec<CodeStackSlot>) {
    let local_size = align(local_size, 16);
    // plan-34-D: hand-built helper bodies (runtime helpers, link thunks) are
    // shared lowering too — their pre-allocation stream must name no physical
    // register. A hit is a compiler-source regression, never input-dependent,
    // so it is an ICE rather than a threaded build error.
    if let Some(offense) = regalloc::find_physical_operand(instructions) {
        panic!(
            "shared helper lowering violated the zero-physical-register \
             invariant (plan-34-D): {offense}"
        );
    }
    let outcome = regalloc::allocate(
        regalloc::active_kind(),
        instructions,
        &[],
        &[],
        crate::codegen::engine::mir::active_backend().register_model(),
        local_size,
        reserved,
    );
    let mut stack_slots: Vec<CodeStackSlot> = outcome
        .spill_slots
        .iter()
        .enumerate()
        .map(|(index, offset)| CodeStackSlot {
            name: format!("spill_{index}"),
            type_: "spill".to_string(),
            offset: *offset as i32,
        })
        .collect();
    let stack_size = local_size
        + outcome.spill_slots.len()
            * crate::codegen::engine::mir::active_backend()
                .register_model()
                .spill_slot_bytes();
    let frame = finalize_frame(
        instructions,
        &mut stack_slots,
        stack_size,
        outcome.extra_callee_saved,
    );
    (frame, stack_slots)
}

/// Whether `register` is a 64-bit FP scalar (`d0`–`d31`), which must be spilled
/// with `str d`/`ldr d` in the callee-save area rather than the GPR `str`/`ldr`
/// (plan-03 Stage D callee-saved FP).
fn is_fp_register(register: &str) -> bool {
    // AArch64 scalar `d0`–`d31`.
    if register
        .strip_prefix('d')
        .is_some_and(|rest| rest.parse::<u8>().is_ok())
    {
        return true;
    }
    // RISC-V FP ABI names `ft*`/`fs*`/`fa*` (plan-99). The integer saved/temp/arg
    // registers `s*`/`t*`/`a*` have no `f` prefix, so this does not confuse them
    // (e.g. `fs0` is FP, `s0` is integer).
    ["ft", "fs", "fa"].iter().any(|prefix| {
        register
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.parse::<u8>().is_ok())
    })
}

/// Whether `register` is an AArch64 FP/SIMD callee-saved register (`d8`–`d15`).
/// These are the only callee-saved registers that can carry a 128-bit `v128`
/// value, so they must be saved/restored with the 128-bit `str q`/`ldr q` into a
/// 16-byte slot (bug-124.2). RISC-V's FP callee-saved registers are 64-bit
/// doubles (`fs*`; no 128-bit SIMD on that path) and take the `str d` branch —
/// so this predicate matches only the `d`-prefixed spelling, never `fs*`.
/// Written with a prefix + numeric-range check (not literal register names) so it
/// does not trip the plan-34-D "shared lowering names no physical register"
/// source scan — it is a *classifier*, not a hardcoded operand.
fn is_aarch64_fp_callee_saved(register: &str) -> bool {
    register
        .strip_prefix('d')
        .and_then(|rest| rest.parse::<u8>().ok())
        .is_some_and(|n| (8..=15).contains(&n))
}

/// Whether `register` is a callee-saved SIMD register that can carry a 128-bit
/// `v128` value and so must be spilled with the 128-bit `str q`/`ldr q` into a
/// 16-byte slot: AArch64 `d8`–`d15` (bug-124.2) OR x86-64 `xmm6`–`xmm15`. Only
/// Win64 makes any `xmm` callee-saved (SysV makes them all caller-saved, so its
/// `callee_saved` list never contains one), which is why a callee-saved `xmm`
/// spelled with a GPR `str_u64` — the encoder rejecting `unknown register
/// 'xmm10'` — went unseen until a float-using Win64 function needed one to hold a
/// value live across a call (plan-66: the `audio::` library's `render`/`play`).
/// The 64-bit `str d`/`ldr d` would truncate a spilled vector's high lane exactly
/// as on AArch64, so both take the 128-bit `movups` path.
fn is_simd128_callee_saved(register: &str) -> bool {
    is_aarch64_fp_callee_saved(register)
        || register
            .strip_prefix("xmm")
            .and_then(|rest| rest.parse::<u8>().ok())
            .is_some()
}

fn save_callee_saved(register: &str, offset: usize) -> CodeInstruction {
    if is_simd128_callee_saved(register) {
        // 128-bit `str q` (AArch64 `str q` / x86-64 `movups`) — a 64-bit store
        // would truncate a `v128` value's high lane (bug-124.2).
        abi::vector_store(register, abi::stack_pointer(), offset)
    } else if is_fp_register(register) {
        abi::store_double(register, abi::stack_pointer(), offset)
    } else {
        abi::store_u64(register, abi::stack_pointer(), offset)
    }
}

fn restore_callee_saved(register: &str, offset: usize) -> CodeInstruction {
    if is_simd128_callee_saved(register) {
        abi::vector_load(register, abi::stack_pointer(), offset)
    } else if is_fp_register(register) {
        abi::load_double(register, abi::stack_pointer(), offset)
    } else {
        abi::load_u64(register, abi::stack_pointer(), offset)
    }
}

fn adjust_stack_instruction_offsets(instructions: &mut [CodeInstruction], offset_delta: usize) {
    if offset_delta == 0 {
        return;
    }
    // `sp`-relative accesses are shifted to clear the callee-saved area the frame
    // prologue adds. But a platform hook may bracket a call with its own
    // `sub_sp N … str x, [sp, k] … add_sp N` to pass a variadic stack argument
    // (e.g. the `open` mode on Darwin); those `[sp, k]` are relative to that local
    // region, not the function frame, and must NOT be shifted. Track the local
    // stack-adjustment depth and only shift accesses at depth 0.
    let mut depth = 0usize;
    for instruction in instructions {
        match instruction.op {
            CodeOp::SubSp => {
                depth += 1;
                continue;
            }
            CodeOp::AddSp => {
                depth = depth.saturating_sub(1);
                continue;
            }
            _ => {}
        }
        if depth > 0 {
            continue;
        }
        // "sp" is the neutral/AArch64 spelling; "rsp" is the x86-64 spelling the
        // per-ISA selection rewrites it to. Both must shift: selection runs
        // BEFORE frame finalization, so an x86 body arrives here rsp-flavored,
        // while post-selection insertions (the prologue zero-init splices) are
        // still sp-flavored. Shifting only "sp" left the x86 body (and the
        // regalloc's rsp-based spill slots) UNSHIFTED while the splices and the
        // stack-slot metadata shifted — so the callee-saved save area at
        // [rsp+0..save_size) collided with body slots 0/8/16 (e.g.
        // make_error_result's param spill to slot 0 destroyed the saved r12),
        // and the owned-value zero-inits landed save_size bytes away from the
        // slots the scope-drops actually read.
        let stack_relative = instruction.fields.iter().any(|(name, value)| {
            // Check the field name before rendering so a non-`base`/`src` operand
            // (the common case) never allocates. `rendered()` borrows the `Raw`/
            // `Phys` register spelling — no `String` clone.
            if !matches!(*name, "base" | "src") {
                return false;
            }
            let value = value.rendered();
            abi::is_stack_pointer(&value)
                || value.as_ref() == crate::arch::x86_64::regmodel::STACK_POINTER
        });
        if !stack_relative {
            continue;
        }
        for (name, value) in &mut instruction.fields {
            if matches!(*name, "offset" | "imm") {
                if let Ok(offset) = value.rendered().parse::<usize>() {
                    *value = Operand::imm((offset + offset_delta) as i64);
                }
            }
        }
    }
}

/// Drift guard (bug-360): every `sp`-relative body access must land inside the
/// frame this function just sized.
///
/// A hand-written helper body addresses `sp + k` for scratch it believes the
/// frame reserves, but the reservation (`finalize_vreg_body_with_locals`'s
/// `local_size`) and the offsets live in different files — and, for a platform
/// hook like `emit_temp_directory`, in different *modules*. When they drift the
/// access silently lands above the frame, in the caller's, and the first thing up
/// there is the caller's saved link register. bug-360 was exactly that: a
/// `sp + 32` scratch store against a 48-byte aarch64 frame overwrote the caller's
/// `x30` with the capacity constant 4096, so every program that touched
/// `fs::tempDirectory` ran to completion, printed correct output, and then
/// branched to `0x1000` and took a SIGSEGV. Nothing failed near the cause.
///
/// Run after the body shift and *before* `resolve_stack_arg_sentinels`, so the
/// incoming-argument sentinels — which do legitimately address above the frame —
/// are still unresolved and fail the numeric parse, exactly as they do in the
/// shift itself. Depth tracking mirrors the shift for the same reason: a platform
/// hook's own `sub_sp`-bracketed region is not frame-relative.
///
/// A hit is a compiler-source regression, never input-dependent, so it is an
/// assertion rather than a threaded build error. Debug-only, matching the
/// `RULES` drift guard (bug-40).
#[cfg(debug_assertions)]
fn assert_stack_accesses_fit_frame(instructions: &[CodeInstruction], total_stack_size: usize) {
    let mut depth = 0usize;
    for instruction in instructions {
        match instruction.op {
            CodeOp::SubSp => {
                depth += 1;
                continue;
            }
            CodeOp::AddSp => {
                depth = depth.saturating_sub(1);
                continue;
            }
            _ => {}
        }
        if depth > 0 {
            continue;
        }
        let stack_relative = instruction.fields.iter().any(|(name, value)| {
            if !matches!(*name, "base" | "src") {
                return false;
            }
            let value = value.rendered();
            abi::is_stack_pointer(&value)
                || value.as_ref() == crate::arch::x86_64::regmodel::STACK_POINTER
        });
        if !stack_relative {
            continue;
        }
        for (name, value) in &instruction.fields {
            if !matches!(*name, "offset" | "imm") {
                continue;
            }
            let Ok(offset) = value.rendered().parse::<usize>() else {
                continue;
            };
            // A load/store consumes 8 bytes at `offset`; an address computation
            // (`add_immediate`) may legally name the frame's end as a limit.
            let needed = match instruction.op {
                CodeOp::AddImm => offset,
                _ => offset + 8,
            };
            assert!(
                needed <= total_stack_size,
                "sp-relative access at sp+{offset} escapes the {total_stack_size}-byte \
                 frame (bug-360): the helper body's scratch offsets and the frame's \
                 reserved local_size have drifted apart"
            );
        }
    }
}

/// Read the `base`/`offset` of a stack-argument sentinel load/store (bug-08).
/// Borrows the base operand's spelling (`rendered()` lends the `Raw` sentinel
/// string with no allocation); the callers only compare it against the two
/// sentinel constants.
fn base_of(instruction: &CodeInstruction) -> Option<std::borrow::Cow<'_, str>> {
    instruction
        .fields
        .iter()
        .find(|(name, _)| *name == "base")
        .map(|(_, value)| value.rendered())
}

fn offset_of(instruction: &CodeInstruction) -> usize {
    instruction
        .fields
        .iter()
        .find(|(name, _)| *name == "offset")
        .and_then(|(_, value)| value.rendered().parse::<usize>().ok())
        .unwrap_or(0)
}

/// The widest outgoing stack-argument byte offset any call in this function
/// writes (bug-08), or `None` when no call passes stack arguments. Drives the
/// size of the reserved outgoing tail at the frame bottom.
fn max_outgoing_arg_offset(instructions: &[CodeInstruction]) -> Option<usize> {
    instructions
        .iter()
        .filter(|instruction| base_of(instruction).as_deref() == Some(abi::OUTGOING_ARGS_BASE))
        .map(offset_of)
        .max()
}

/// Whether any instruction reads an incoming stack argument (bug-08).
fn has_incoming_stack_args(instructions: &[CodeInstruction]) -> bool {
    instructions
        .iter()
        .any(|instruction| base_of(instruction).as_deref() == Some(abi::INCOMING_ARGS_BASE))
}

/// Rewrite the stack-argument sentinel bases (`incoming_args`/`outgoing_args`)
/// to concrete `sp`-relative accesses now that the frame size is known (bug-08).
/// An incoming argument `k` lives above the whole frame, past the entry
/// return-address padding: `[sp + frame_size + entry_padding + k*8]`. An outgoing
/// argument keeps its frame-bottom offset (`[sp + k*8]`), which the body shift
/// deliberately skipped, and only has its base rewritten. Runs after
/// [`adjust_stack_instruction_offsets`], so the rewritten `sp` offsets are final.
fn resolve_stack_arg_sentinels(
    instructions: &mut [CodeInstruction],
    frame_size: usize,
    entry_padding: usize,
) {
    for instruction in instructions.iter_mut() {
        let base = match base_of(instruction) {
            Some(base) => base,
            None => continue,
        };
        let incoming = if base.as_ref() == abi::INCOMING_ARGS_BASE {
            true
        } else if base.as_ref() == abi::OUTGOING_ARGS_BASE {
            false
        } else {
            continue;
        };
        let resolved_offset = if incoming {
            // The caller places outgoing arg 0 above the Win64 shadow space
            // (`outgoing_args_base_offset()`), so the callee must read its incoming
            // args from the same shadow-space-adjusted position — otherwise a >8-arg
            // Win64 call reads garbage out of the shadow region (the offset defaults
            // to 0, so SysV/AAPCS64 frames are byte-identical).
            frame_size
                + entry_padding
                + crate::codegen::engine::mir::active_backend().outgoing_args_base_offset()
                + offset_of(instruction)
        } else {
            // Outgoing arg 0 sits above the Win64 shadow space (plan-47-B §4.3);
            // `outgoing_args_base_offset()` defaults to 0, so SysV/AAPCS64 place it
            // at the frame bottom unchanged.
            crate::codegen::engine::mir::active_backend().outgoing_args_base_offset()
                + offset_of(instruction)
        };
        for (name, value) in &mut instruction.fields {
            match *name {
                "base" => *value = Operand::from(abi::stack_pointer()),
                "offset" => *value = Operand::imm(resolved_offset as i64),
                _ => {}
            }
        }
    }
}

/// A monotonic virtual-register name generator for a hand-written vreg helper
/// (plan-00-G Phase 2): each call yields a fresh `%vN` the shared allocator
/// colors. Lets the PCG64 / arena helpers be written in target-neutral MIR (no
/// fixed `x9`/`x13`…) so register placement is a per-ISA backend job.
pub(crate) struct Vregs(usize);

impl Vregs {
    pub(crate) fn new() -> Self {
        Vregs(0)
    }

    pub(crate) fn next(&mut self) -> String {
        let name = format!("%v{}", self.0);
        self.0 += 1;
        name
    }
}
