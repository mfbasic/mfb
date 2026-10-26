//! Split from `the retired flat codegen_utils.rs` (category `engine.util`).

// --- codegen tier imports (migration) ---
use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::regalloc;
use crate::codegen::engine::types::*;
use crate::target::shared::abi;
/// Emit the prologue's frame allocation: one `sub sp, total` on a target whose
/// stack is addressable without cooperation, or a page-by-page probing sequence
/// on one that grows its stack a guard page at a time (Windows — see
/// `Backend::stack_probe_page_bytes`).
///
/// The probing form subtracts one page and writes a zero at the new `sp` for each
/// whole page of the frame, then subtracts the remainder. Every probe address
/// lies inside the frame being allocated (`total - i*page >= 0`) and is dead at
/// this point — the callee-saved stores, the spill area, and the outgoing-argument
/// tail are all written afterwards — so zeroing it changes nothing. The trailing
/// remainder needs no probe of its own: it is smaller than a page, so the first
/// body access below it can only ever land on the immediately next page, which is
/// the single-page step the guard is designed to absorb.
///
/// A frame of one page or less can never skip the guard, so it keeps the single
/// `sub sp` — which is every frame in the runtime helpers and the overwhelming
/// majority of lowered functions, leaving them byte-identical.
fn push_frame_allocation(prologue: &mut Vec<CodeInstruction>, total_stack_size: usize) {
    let page = crate::codegen::engine::mir::active_backend().stack_probe_page_bytes();
    if page == 0 || total_stack_size <= page {
        prologue.push(abi::subtract_stack(total_stack_size));
        return;
    }
    let whole_pages = total_stack_size / page;
    for _ in 0..whole_pages {
        prologue.push(abi::subtract_stack(page));
        prologue.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0));
    }
    let remainder = total_stack_size - whole_pages * page;
    if remainder != 0 {
        prologue.push(abi::subtract_stack(remainder));
    }
}

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
            .any(|register| register == abi::LR_SPELLING)
    {
        callee_saved.push(abi::LR_SPELLING.to_string());
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
    assert_stack_accesses_fit_frame(instructions, total_stack_size);

    // Resolve the incoming/outgoing stack-argument sentinels now that the final
    // frame size is known (bug-08). Incoming arguments sit above the whole frame,
    // past the entry return-address padding (8 on x86-64, 0 on AArch64); outgoing
    // arguments sit at the reserved frame bottom (`[sp+0..]`, already unshifted).
    if outgoing_bytes != 0 || has_incoming_stack_args(instructions) {
        let entry_padding = crate::codegen::engine::mir::active_backend().frame_call_padding();
        resolve_stack_arg_sentinels(instructions, total_stack_size, entry_padding);
    }

    // The save area as the prologue writes it and every epilogue reads it back:
    // one access per register, or — on a backend with pair access — one `stp`/`ldp`
    // per two adjacent same-kind slots (plan-171-A). The slots themselves, and so
    // the frame layout, are the same either way.
    let pairs = crate::codegen::engine::mir::active_backend().supports_pair_access();
    let accesses = save_accesses(&callee_saved, &callee_offsets, outgoing_bytes, pairs);
    // Pre-/post-index fold: a frame small enough for the pair's imm7 whose first
    // pair sits at the very bottom moves `sp` with that pair, `stp a, b, [sp,
    // #-total]!` in for `sub sp, total; stp a, b, [sp]`, and `ldp a, b, [sp],
    // #total` in for `ldp a, b, [sp]; add sp, total` (§4.3). A frame with an
    // outgoing-argument tail has no save at offset 0, and a probing frame
    // (`push_frame_allocation`'s page loop) keeps its own allocation.
    let probe_page = crate::codegen::engine::mir::active_backend().stack_probe_page_bytes();
    let fold = pairs
        && total_stack_size <= MAX_FOLDED_FRAME
        && (probe_page == 0 || total_stack_size <= probe_page)
        && matches!(accesses.first(), Some(SaveAccess::Pair { offset: 0, .. }));
    let mut prologue = Vec::new();
    if fold {
        prologue.push(accesses[0].store(-(total_stack_size as i64), abi::PairAddressing::PreIndex));
    } else {
        push_frame_allocation(&mut prologue, total_stack_size);
    }
    for access in accesses.iter().skip(usize::from(fold)) {
        prologue.push(access.save());
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
            for access in accesses.iter().skip(usize::from(fold)).rev() {
                rewritten.push(access.restore());
            }
            if fold {
                rewritten.push(
                    accesses[0].load(total_stack_size as i64, abi::PairAddressing::PostIndex),
                );
            } else {
                rewritten.push(abi::add_stack(total_stack_size));
            }
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

/// The largest frame the pre-/post-index fold moves `sp` by: the post-index
/// `ldp`'s imm7 reaches +504 (`63 * 8`), and the frame size is a multiple of 16,
/// so the largest folded frame is 496 bytes.
const MAX_FOLDED_FRAME: usize = 504;
/// The highest offset a 64-bit (`X`) pair can address (`63 * 8`).
const MAX_X_PAIR_OFFSET: usize = 504;
/// The highest offset a 128-bit (`Q`) pair can address (`63 * 16`).
const MAX_Q_PAIR_OFFSET: usize = 1008;

/// One access to the callee-saved area: a register on its own, or two adjacent
/// slots of one kind (both 8-byte integer slots, or both 16-byte `q` slots) as
/// one `stp`/`ldp` (plan-171-A). `offset` is the final `sp` offset of the
/// first (lower) slot.
enum SaveAccess<'a> {
    Single {
        register: &'a str,
        offset: usize,
    },
    Pair {
        first: &'a str,
        second: &'a str,
        offset: usize,
        simd: bool,
    },
}

impl SaveAccess<'_> {
    fn save(&self) -> CodeInstruction {
        match *self {
            SaveAccess::Single { register, offset } => save_callee_saved(register, offset),
            SaveAccess::Pair { offset, .. } => {
                self.store(offset as i64, abi::PairAddressing::Offset)
            }
        }
    }

    fn restore(&self) -> CodeInstruction {
        match *self {
            SaveAccess::Single { register, offset } => restore_callee_saved(register, offset),
            SaveAccess::Pair { offset, .. } => {
                self.load(offset as i64, abi::PairAddressing::Offset)
            }
        }
    }

    /// The pair's `stp` at `offset` in `addressing` mode.
    fn store(&self, offset: i64, addressing: abi::PairAddressing) -> CodeInstruction {
        let SaveAccess::Pair {
            first,
            second,
            simd,
            ..
        } = *self
        else {
            unreachable!("only a pair access has a pair store")
        };
        let (first, second) = (saved_register(first), saved_register(second));
        if simd {
            abi::store_pair_q(first, second, abi::stack_pointer(), offset, addressing)
        } else {
            abi::store_pair_u64(first, second, abi::stack_pointer(), offset, addressing)
        }
    }

    /// The pair's `ldp` at `offset` in `addressing` mode.
    fn load(&self, offset: i64, addressing: abi::PairAddressing) -> CodeInstruction {
        let SaveAccess::Pair {
            first,
            second,
            simd,
            ..
        } = *self
        else {
            unreachable!("only a pair access has a pair load")
        };
        let (first, second) = (saved_register(first), saved_register(second));
        if simd {
            abi::load_pair_q(first, second, abi::stack_pointer(), offset, addressing)
        } else {
            abi::load_pair_u64(first, second, abi::stack_pointer(), offset, addressing)
        }
    }
}

/// The save area's accesses in `callee_saved` order. With `pairs`, two
/// consecutive entries become one pair when both are 8-byte integer slots or
/// both 16-byte `q` slots, the second directly above the first, and the first
/// within the pair form's reach; everything else — an odd tail, a kind change,
/// a 64-bit FP slot, a slot past the reach — stays a single access.
fn save_accesses<'a>(
    callee_saved: &'a [String],
    offsets: &[usize],
    outgoing_bytes: usize,
    pairs: bool,
) -> Vec<SaveAccess<'a>> {
    let mut accesses = Vec::with_capacity(callee_saved.len());
    let mut index = 0;
    while index < callee_saved.len() {
        let register = callee_saved[index].as_str();
        let offset = outgoing_bytes + offsets[index];
        if pairs && index + 1 < callee_saved.len() {
            let next = callee_saved[index + 1].as_str();
            let simd = is_simd128_callee_saved(register);
            let integer = |name: &str| !is_simd128_callee_saved(name) && !is_fp_register(name);
            let (width, reach) = if simd {
                (16, MAX_Q_PAIR_OFFSET)
            } else {
                (8, MAX_X_PAIR_OFFSET)
            };
            let same_kind = if simd {
                is_simd128_callee_saved(next)
            } else {
                integer(register) && integer(next)
            };
            if same_kind && offsets[index + 1] == offsets[index] + width && offset <= reach {
                accesses.push(SaveAccess::Pair {
                    first: register,
                    second: next,
                    offset,
                    simd,
                });
                index += 2;
                continue;
            }
        }
        accesses.push(SaveAccess::Single { register, offset });
        index += 1;
    }
    accesses
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
/// The helper has no declared params (it uses the ABI registers directly).
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
        instructions,
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

/// A register the frame saves: one the coloring chose, or the link register (the
/// neutral `lr` token, which each backend realizes) when the body makes calls.
fn saved_register(name: &str) -> abi::Reg {
    if name == abi::LR_SPELLING {
        abi::LR
    } else {
        abi::Reg::physical(name)
    }
}

fn save_callee_saved(name: &str, offset: usize) -> CodeInstruction {
    let register = saved_register(name);
    if is_simd128_callee_saved(name) {
        // 128-bit `str q` (AArch64 `str q` / x86-64 `movups`) — a 64-bit store
        // would truncate a `v128` value's high lane (bug-124.2).
        abi::vector_store(register, abi::stack_pointer(), offset)
    } else if is_fp_register(name) {
        abi::store_double(register, abi::stack_pointer(), offset)
    } else {
        abi::store_u64(register, abi::stack_pointer(), offset)
    }
}

fn restore_callee_saved(name: &str, offset: usize) -> CodeInstruction {
    let register = saved_register(name);
    if is_simd128_callee_saved(name) {
        abi::vector_load(register, abi::stack_pointer(), offset)
    } else if is_fp_register(name) {
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

/// The sp-relative local scratch an already-emitted body needs: the high-water
/// mark of its literal `[sp + N]` accesses, in bytes.
///
/// This is the *measured* answer to the question `finalize_vreg_body_with_locals`
/// takes on faith from its `local_size` argument, and it exists so a caller that
/// appends a hand-written platform body — which reserves scratch by naming raw
/// offsets rather than through `allocate_stack_object` — can report the real
/// requirement instead of guessing. Guessing is what bug-360 and the
/// `term::on`/Win64-app drift both were: the body addressed `sp + 0x70` while its
/// caller declared `0` locals, so in release (where the debug assertion below is
/// compiled out) the store landed above the frame in the caller's.
///
/// Uses exactly the predicate the assertion uses, so "what this returns" and
/// "what the assertion accepts" cannot drift: same depth tracking (a platform
/// hook's own `sub_sp` region is not frame-relative), same numeric-parse filter
/// (argument sentinels are still unresolved here and are correctly skipped), and
/// the same `AddImm`-names-the-limit rule.
pub(crate) fn required_sp_local_bytes(instructions: &[CodeInstruction]) -> usize {
    let mut needed = 0usize;
    for_each_frame_relative_sp_access(instructions, |bytes| needed = needed.max(bytes));
    needed
}

/// Shared walk behind [`required_sp_local_bytes`] and the debug drift assertion:
/// calls `visit` with the number of frame bytes each `sp`-relative access needs.
fn for_each_frame_relative_sp_access(
    instructions: &[CodeInstruction],
    mut visit: impl FnMut(usize),
) {
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
            if !matches!(*name, "base" | "src" | "dst") {
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
            // A load/store consumes its own width at `offset` (bug-774: a byte
            // read of the frame's last byte needs `offset + 1`, not `offset + 8`;
            // a 16-byte `ldr_q`/`str_q` needs `offset + 16`); an address
            // computation (`add_immediate`) may legally name the frame's end as a
            // limit.
            visit(match instruction.op {
                CodeOp::AddImm => offset,
                CodeOp::LdrU8 | CodeOp::StrU8 => offset + 1,
                CodeOp::LdrU16 | CodeOp::StrU16 => offset + 2,
                CodeOp::LdrU32 | CodeOp::StrU32 => offset + 4,
                CodeOp::LdrQ | CodeOp::StrQ => offset + 16,
                _ => offset + 8,
            });
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
/// assertion rather than a threaded build error.
///
/// bug-550: no longer debug-only. It was gated to match the `RULES` drift guard
/// (bug-40), but every CI job builds `--release`, so the gate meant this ran on
/// no platform in CI — and an sp-relative access past the frame writes over the
/// caller's stack, which is undebuggable from the symptom. One extra linear pass
/// over a function's instruction stream is a small price beside the passes the
/// backend already makes.
fn assert_stack_accesses_fit_frame(instructions: &[CodeInstruction], total_stack_size: usize) {
    for_each_frame_relative_sp_access(instructions, |needed| {
        assert!(
            needed <= total_stack_size,
            "sp-relative access needing {needed} bytes escapes the {total_stack_size}-byte \
             frame (bug-360): the helper body's scratch offsets and the frame's \
             reserved local_size have drifted apart"
        );
    });
}

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
        .filter(|instruction| base_of(instruction).as_deref() == Some(abi::OUTGOING_ARGS_SPELLING))
        .map(offset_of)
        .max()
}

/// Whether any instruction reads an incoming stack argument (bug-08).
fn has_incoming_stack_args(instructions: &[CodeInstruction]) -> bool {
    instructions
        .iter()
        .any(|instruction| base_of(instruction).as_deref() == Some(abi::INCOMING_ARGS_SPELLING))
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
        let incoming = if base.as_ref() == abi::INCOMING_ARGS_SPELLING {
            true
        } else if base.as_ref() == abi::OUTGOING_ARGS_SPELLING {
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
///
/// Every name comes from the one per-compilation id source
/// ([`regalloc::mint_vreg_id`](crate::codegen::engine::regalloc::mint_vreg_id)), shared
/// with `CodeBuilder::allocate_register` — so no two minters, however nested, can
/// hand out the same name in one instruction stream (bug-721).
pub(crate) struct Vregs;

impl Vregs {
    pub(crate) fn new() -> Self {
        Vregs
    }

    pub(crate) fn next(&mut self) -> crate::codegen::engine::operand::VirtualRegister {
        crate::codegen::engine::regalloc::mint_vreg(crate::target::shared::regmodel::RegClass::Int)
    }
}

#[cfg(test)]
mod tests;
