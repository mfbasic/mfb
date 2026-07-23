use super::*;

/// Lower the standalone string-list sort helper used to give `fs::listDirectory`
/// a deterministic, stable order. It takes a `List OF String` collection pointer
/// in `x0` and sorts its entries in place by ascending byte-wise (UTF-8
/// lexicographic) order using selection sort, swapping only the fixed-size entry
/// records and leaving the data region untouched. It makes no calls.
pub(super) fn lower_sort_string_list_helper() -> CodeFunction {
    let symbol = SORT_STRING_LIST_SYMBOL;
    // x0  = collection pointer (preserved for the caller)
    // x9  = entries base (collection + header)
    // x10 = count
    // x11 = data region base (entries base + count * entry size)
    // x12 = i (outer index), x13 = min index, x14 = j (inner index)
    // x15 = entry[min] address, x16 = entry[j] address
    // x1..x7 = comparison/swap scratch
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let done = format!("{symbol}_done");
    let outer = format!("{symbol}_outer");
    let inner = format!("{symbol}_inner");
    let inner_done = format!("{symbol}_inner_done");
    let no_swap = format!("{symbol}_no_swap");
    let next_inner = format!("{symbol}_next_inner");
    let cmp_loop = format!("{symbol}_cmp_loop");
    let take_j = format!("{symbol}_take_j");
    let keep_min = format!("{symbol}_keep_min");

    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64("%v10", abi::ARG[0], COLLECTION_OFFSET_COUNT),
        abi::compare_immediate("%v10", "1"),
        abi::branch_le(&done),
        abi::add_immediate("%v9", abi::ARG[0], COLLECTION_HEADER_SIZE),
        abi::move_immediate("%v1", "Integer", &entry_size),
        // data region base = entries base + capacity * entry size (the data
        // region sits past the full lookup capacity for a grown list; §4.2).
        abi::load_u64("%v8", abi::ARG[0], COLLECTION_OFFSET_CAPACITY),
        abi::multiply_registers("%v11", "%v8", "%v1"),
        abi::add_registers("%v11", "%v9", "%v11"),
        abi::move_immediate("%v12", "Integer", "0"),
        // outer: for i in 0..count-1
        abi::label(&outer),
        abi::add_immediate("%v2", "%v12", 1),
        abi::compare_registers("%v2", "%v10"),
        abi::branch_ge(&done),
        abi::move_register("%v13", "%v12"),
        abi::move_register("%v14", "%v2"),
        // inner: for j in i+1..count
        abi::label(&inner),
        abi::compare_registers("%v14", "%v10"),
        abi::branch_ge(&inner_done),
        // entry[min] -> x15, entry[j] -> x16
        abi::move_immediate("%v1", "Integer", &entry_size),
        abi::multiply_registers("%v15", "%v13", "%v1"),
        abi::add_registers("%v15", "%v9", "%v15"),
        abi::multiply_registers("%v16", "%v14", "%v1"),
        abi::add_registers("%v16", "%v9", "%v16"),
        // name pointers: data_base + value_offset ; lengths: value_length
        abi::load_u64("%v2", "%v15", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers("%v2", "%v11", "%v2"),
        abi::load_u64("%v3", "%v15", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::load_u64("%v4", "%v16", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers("%v4", "%v11", "%v4"),
        abi::load_u64("%v5", "%v16", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        // compare bytes: x2/x3 = min name ptr/len, x4/x5 = j name ptr/len
        abi::move_immediate("%v6", "Integer", "0"),
        abi::label(&cmp_loop),
        // if reached end of min name -> min is prefix; j<min iff j also ended? no: min shorter => min<j => keep_min
        abi::compare_registers("%v6", "%v3"),
        abi::branch_ge(&keep_min),
        // if reached end of j name -> j shorter, j<min => take_j
        abi::compare_registers("%v6", "%v5"),
        abi::branch_ge(&take_j),
        abi::load_u8("%v7", "%v2", 0),
        abi::load_u8("%v1", "%v4", 0),
        abi::compare_registers("%v1", "%v7"),
        abi::branch_lo(&take_j),
        abi::branch_hi(&keep_min),
        abi::add_immediate("%v2", "%v2", 1),
        abi::add_immediate("%v4", "%v4", 1),
        abi::add_immediate("%v6", "%v6", 1),
        abi::branch(&cmp_loop),
        abi::label(&take_j),
        abi::move_register("%v13", "%v14"),
        abi::label(&keep_min),
        abi::label(&next_inner),
        abi::add_immediate("%v14", "%v14", 1),
        abi::branch(&inner),
        abi::label(&inner_done),
        // swap entry[i] and entry[min] if different
        abi::compare_registers("%v13", "%v12"),
        abi::branch_eq(&no_swap),
        abi::move_immediate("%v1", "Integer", &entry_size),
        abi::multiply_registers("%v2", "%v12", "%v1"),
        abi::add_registers("%v2", "%v9", "%v2"),
        abi::multiply_registers("%v3", "%v13", "%v1"),
        abi::add_registers("%v3", "%v9", "%v3"),
    ];
    // swap COLLECTION_ENTRY_SIZE bytes (8 at a time)
    let mut offset = 0;
    while offset < COLLECTION_ENTRY_SIZE {
        instructions.extend([
            abi::load_u64("%v4", "%v2", offset),
            abi::load_u64("%v5", "%v3", offset),
            abi::store_u64("%v5", "%v2", offset),
            abi::store_u64("%v4", "%v3", offset),
        ]);
        offset += 8;
    }
    instructions.extend([
        abi::label(&no_swap),
        abi::add_immediate("%v12", "%v12", 1),
        abi::branch(&outer),
        abi::label(&done),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    CodeFunction {
        name: "runtime.sortStringList".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations: Vec::new(),
    }
}

/// Emit a call to the shared [`VALIDATE_UTF8_SYMBOL`] helper. The byte pointer
/// must already be in `x0` and the byte length in `x1`. The helper returns `0`
/// in `x0` for valid UTF-8 and `1` for invalid; this branches to `error_label`
/// when invalid. Keeping validation in a separate `bl`-reachable function (with
/// its own frame and short-range internal branches) keeps the filesystem read
/// helpers small.
pub(super) fn emit_call_validate_utf8(
    symbol: &str,
    error_label: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::branch_link(VALIDATE_UTF8_SYMBOL));
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: VALIDATE_UTF8_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::RET[0], "0"),
        abi::branch_ne(error_label),
    ]);
}

/// Lower the standalone UTF-8 validation helper. It takes a byte pointer in `x0`
/// and a byte length in `x1`, and returns `0` in `x0` when the buffer is
/// well-formed UTF-8 or `1` otherwise. The working set is virtual registers the
/// allocator colors per-ISA (a hardcoded pool would land on x86 callee-saved
/// GPRs and clobber the caller); it makes no calls, so the resulting frame is
/// whatever callee-saved saves the coloring requires (typically none).
pub(super) fn lower_validate_utf8_helper() -> CodeFunction {
    let symbol = VALIDATE_UTF8_SYMBOL;
    let invalid = format!("{symbol}_invalid");
    let mut vregs = Vregs::new();
    let mut instructions = vec![abi::label("entry")];
    if std::env::var("MFB_ASCII").is_ok() {
        let lp = format!("{symbol}_lp");
        let ok = format!("{symbol}_ok");
        let pos = vregs.next();
        let rem = vregs.next();
        let byte = vregs.next();
        instructions.extend([
            abi::move_register(&pos, abi::ARG[0]),
            abi::move_register(&rem, abi::ARG[1]),
            abi::label(&lp),
            abi::compare_immediate(&rem, "0"),
            abi::branch_eq(&ok),
            abi::load_u8(&byte, &pos, 0),
            abi::compare_immediate(&byte, "127"),
            abi::branch_hi(&invalid),
            abi::add_immediate(&pos, &pos, 1),
            abi::subtract_immediate(&rem, &rem, 1),
            abi::branch(&lp),
            abi::label(&ok),
            abi::move_immediate(abi::RET[0], "Integer", "0"),
            abi::return_(),
            abi::label(&invalid),
            abi::move_immediate(abi::RET[0], "Integer", "1"),
            abi::return_(),
        ]);
    } else {
        emit_validate_utf8(
            symbol,
            abi::ARG[0],
            abi::ARG[1],
            &invalid,
            &mut instructions,
            &mut vregs,
        );
        instructions.extend([
            abi::move_immediate(abi::RET[0], "Integer", "0"),
            abi::return_(),
            abi::label(&invalid),
            abi::move_immediate(abi::RET[0], "Integer", "1"),
            abi::return_(),
        ]);
    }
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    CodeFunction {
        name: "runtime.validateUtf8".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations: Vec::new(),
    }
}

/// Validate that the `len`-byte buffer at `ptr` is well-formed UTF-8, branching
/// to `error_label` on the first invalid sequence. Used by
/// [`lower_validate_utf8_helper`]. The working set is minted from `vregs`; `ptr`
/// and `len` are read into it before any other def, so they may name `x0`/`x1`.
fn emit_validate_utf8(
    symbol: &str,
    ptr: &str,
    len: &str,
    error_label: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let pos = &vregs.next();
    let rem = &vregs.next();
    let byte = &vregs.next();
    let cont = &vregs.next();
    let lo = &vregs.next();
    let hi = &vregs.next();

    let loop_start = format!("{symbol}_utf8_loop");
    let done = format!("{symbol}_utf8_done");
    let one = format!("{symbol}_utf8_one");
    let two = format!("{symbol}_utf8_two");
    let three = format!("{symbol}_utf8_three");
    let four = format!("{symbol}_utf8_four");
    let three_ed = format!("{symbol}_utf8_three_ed");
    let three_bounds = format!("{symbol}_utf8_three_bounds");
    let four_f4 = format!("{symbol}_utf8_four_f4");
    let four_bounds = format!("{symbol}_utf8_four_bounds");

    instructions.extend([
        abi::move_register(pos, ptr),
        abi::move_register(rem, len),
        abi::label(&loop_start),
        abi::compare_immediate(rem, "0"),
        abi::branch_eq(&done),
        abi::load_u8(byte, pos, 0),
        abi::compare_immediate(byte, "128"),
        abi::branch_lo(&one),
        abi::compare_immediate(byte, "194"),
        abi::branch_lo(error_label),
        abi::compare_immediate(byte, "224"),
        abi::branch_lo(&two),
        abi::compare_immediate(byte, "240"),
        abi::branch_lo(&three),
        abi::compare_immediate(byte, "245"),
        abi::branch_lo(&four),
        abi::branch(error_label),
        // 1-byte ASCII
        abi::label(&one),
        abi::add_immediate(pos, pos, 1),
        abi::subtract_immediate(rem, rem, 1),
        abi::branch(&loop_start),
        // 2-byte sequence
        abi::label(&two),
        abi::compare_immediate(rem, "2"),
        abi::branch_lo(error_label),
        abi::load_u8(cont, pos, 1),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::add_immediate(pos, pos, 2),
        abi::subtract_immediate(rem, rem, 2),
        abi::branch(&loop_start),
        // 3-byte sequence
        abi::label(&three),
        abi::compare_immediate(rem, "3"),
        abi::branch_lo(error_label),
        abi::move_immediate(lo, "Integer", "128"),
        abi::move_immediate(hi, "Integer", "191"),
        abi::compare_immediate(byte, "224"),
        abi::branch_ne(&three_ed),
        abi::move_immediate(lo, "Integer", "160"),
        abi::branch(&three_bounds),
        abi::label(&three_ed),
        abi::compare_immediate(byte, "237"),
        abi::branch_ne(&three_bounds),
        abi::move_immediate(hi, "Integer", "159"),
        abi::label(&three_bounds),
        abi::load_u8(cont, pos, 1),
        abi::compare_registers(cont, lo),
        abi::branch_lo(error_label),
        abi::compare_registers(cont, hi),
        abi::branch_hi(error_label),
        abi::load_u8(cont, pos, 2),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::add_immediate(pos, pos, 3),
        abi::subtract_immediate(rem, rem, 3),
        abi::branch(&loop_start),
        // 4-byte sequence
        abi::label(&four),
        abi::compare_immediate(rem, "4"),
        abi::branch_lo(error_label),
        abi::move_immediate(lo, "Integer", "128"),
        abi::move_immediate(hi, "Integer", "191"),
        abi::compare_immediate(byte, "240"),
        abi::branch_ne(&four_f4),
        abi::move_immediate(lo, "Integer", "144"),
        abi::branch(&four_bounds),
        abi::label(&four_f4),
        abi::compare_immediate(byte, "244"),
        abi::branch_ne(&four_bounds),
        abi::move_immediate(hi, "Integer", "143"),
        abi::label(&four_bounds),
        abi::load_u8(cont, pos, 1),
        abi::compare_registers(cont, lo),
        abi::branch_lo(error_label),
        abi::compare_registers(cont, hi),
        abi::branch_hi(error_label),
        abi::load_u8(cont, pos, 2),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::load_u8(cont, pos, 3),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::add_immediate(pos, pos, 4),
        abi::subtract_immediate(rem, rem, 4),
        abi::branch(&loop_start),
        abi::label(&done),
    ]);
}

pub(super) fn finalize_frame(
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
        if is_aarch64_fp_callee_saved(register) {
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
        super::mir::active_backend().frame_call_padding()
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
    let outgoing_bytes = match max_outgoing_arg_offset(instructions) {
        Some(max_offset) => align(max_offset + 8, 16),
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
        let entry_padding = super::mir::active_backend().frame_call_padding();
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
pub(super) fn finalize_vreg_helper(
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
pub(super) fn finalize_vreg_body(
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
pub(super) fn finalize_vreg_body_with_locals(
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
        super::mir::active_backend().register_model(),
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
            * super::mir::active_backend()
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

fn save_callee_saved(register: &str, offset: usize) -> CodeInstruction {
    if is_aarch64_fp_callee_saved(register) {
        // 128-bit `str q` — a 64-bit `str d` would truncate a `v128` value's high
        // lane (bug-124.2). Only AArch64 `d`-registers can carry a 128-bit vector.
        abi::vector_store(register, abi::stack_pointer(), offset)
    } else if is_fp_register(register) {
        abi::store_double(register, abi::stack_pointer(), offset)
    } else {
        abi::store_u64(register, abi::stack_pointer(), offset)
    }
}

fn restore_callee_saved(register: &str, offset: usize) -> CodeInstruction {
    if is_aarch64_fp_callee_saved(register) {
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
            matches!(*name, "base" | "src")
                && (abi::is_stack_pointer(value)
                    || value == crate::arch::x86_64::regmodel::STACK_POINTER)
        });
        if !stack_relative {
            continue;
        }
        for (name, value) in &mut instruction.fields {
            if matches!(*name, "offset" | "imm") {
                if let Ok(offset) = value.parse::<usize>() {
                    *value = (offset + offset_delta).to_string();
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
            matches!(*name, "base" | "src")
                && (abi::is_stack_pointer(value)
                    || value == crate::arch::x86_64::regmodel::STACK_POINTER)
        });
        if !stack_relative {
            continue;
        }
        for (name, value) in &instruction.fields {
            if !matches!(*name, "offset" | "imm") {
                continue;
            }
            let Ok(offset) = value.parse::<usize>() else {
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
fn base_of(instruction: &CodeInstruction) -> Option<&str> {
    instruction
        .fields
        .iter()
        .find(|(name, _)| *name == "base")
        .map(|(_, value)| value.as_str())
}

fn offset_of(instruction: &CodeInstruction) -> usize {
    instruction
        .fields
        .iter()
        .find(|(name, _)| *name == "offset")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0)
}

/// The widest outgoing stack-argument byte offset any call in this function
/// writes (bug-08), or `None` when no call passes stack arguments. Drives the
/// size of the reserved outgoing tail at the frame bottom.
fn max_outgoing_arg_offset(instructions: &[CodeInstruction]) -> Option<usize> {
    instructions
        .iter()
        .filter(|instruction| base_of(instruction) == Some(abi::OUTGOING_ARGS_BASE))
        .map(offset_of)
        .max()
}

/// Whether any instruction reads an incoming stack argument (bug-08).
fn has_incoming_stack_args(instructions: &[CodeInstruction]) -> bool {
    instructions
        .iter()
        .any(|instruction| base_of(instruction) == Some(abi::INCOMING_ARGS_BASE))
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
        let incoming = if base == abi::INCOMING_ARGS_BASE {
            true
        } else if base == abi::OUTGOING_ARGS_BASE {
            false
        } else {
            continue;
        };
        let resolved_offset = if incoming {
            frame_size + entry_padding + offset_of(instruction)
        } else {
            offset_of(instruction)
        };
        for (name, value) in &mut instruction.fields {
            match *name {
                "base" => *value = abi::stack_pointer().to_string(),
                "offset" => *value = resolved_offset.to_string(),
                _ => {}
            }
        }
    }
}

/// A monotonic virtual-register name generator for a hand-written vreg helper
/// (plan-00-G Phase 2): each call yields a fresh `%vN` the shared allocator
/// colors. Lets the PCG64 / arena helpers be written in target-neutral MIR (no
/// fixed `x9`/`x13`…) so register placement is a per-ISA backend job.
pub(super) struct Vregs(usize);

impl Vregs {
    pub(super) fn new() -> Self {
        Vregs(0)
    }

    pub(super) fn next(&mut self) -> String {
        let name = format!("%v{}", self.0);
        self.0 += 1;
        name
    }
}
