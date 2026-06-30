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
        abi::load_u64("x10", "x0", COLLECTION_OFFSET_COUNT),
        abi::compare_immediate("x10", "1"),
        abi::branch_le(&done),
        abi::add_immediate("x9", "x0", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x1", "Integer", &entry_size),
        // data region base = entries base + capacity * entry size (the data
        // region sits past the full lookup capacity for a grown list; §4.2).
        abi::load_u64("x8", "x0", COLLECTION_OFFSET_CAPACITY),
        abi::multiply_registers("x11", "x8", "x1"),
        abi::add_registers("x11", "x9", "x11"),
        abi::move_immediate("x12", "Integer", "0"),
        // outer: for i in 0..count-1
        abi::label(&outer),
        abi::add_immediate("x2", "x12", 1),
        abi::compare_registers("x2", "x10"),
        abi::branch_ge(&done),
        abi::move_register("x13", "x12"),
        abi::move_register("x14", "x2"),
        // inner: for j in i+1..count
        abi::label(&inner),
        abi::compare_registers("x14", "x10"),
        abi::branch_ge(&inner_done),
        // entry[min] -> x15, entry[j] -> x16
        abi::move_immediate("x1", "Integer", &entry_size),
        abi::multiply_registers("x15", "x13", "x1"),
        abi::add_registers("x15", "x9", "x15"),
        abi::multiply_registers("x16", "x14", "x1"),
        abi::add_registers("x16", "x9", "x16"),
        // name pointers: data_base + value_offset ; lengths: value_length
        abi::load_u64("x2", "x15", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers("x2", "x11", "x2"),
        abi::load_u64("x3", "x15", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::load_u64("x4", "x16", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers("x4", "x11", "x4"),
        abi::load_u64("x5", "x16", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        // compare bytes: x2/x3 = min name ptr/len, x4/x5 = j name ptr/len
        abi::move_immediate("x6", "Integer", "0"),
        abi::label(&cmp_loop),
        // if reached end of min name -> min is prefix; j<min iff j also ended? no: min shorter => min<j => keep_min
        abi::compare_registers("x6", "x3"),
        abi::branch_ge(&keep_min),
        // if reached end of j name -> j shorter, j<min => take_j
        abi::compare_registers("x6", "x5"),
        abi::branch_ge(&take_j),
        abi::load_u8("x7", "x2", 0),
        abi::load_u8("x1", "x4", 0),
        abi::compare_registers("x1", "x7"),
        abi::branch_lo(&take_j),
        abi::branch_hi(&keep_min),
        abi::add_immediate("x2", "x2", 1),
        abi::add_immediate("x4", "x4", 1),
        abi::add_immediate("x6", "x6", 1),
        abi::branch(&cmp_loop),
        abi::label(&take_j),
        abi::move_register("x13", "x14"),
        abi::label(&keep_min),
        abi::label(&next_inner),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&inner),
        abi::label(&inner_done),
        // swap entry[i] and entry[min] if different
        abi::compare_registers("x13", "x12"),
        abi::branch_eq(&no_swap),
        abi::move_immediate("x1", "Integer", &entry_size),
        abi::multiply_registers("x2", "x12", "x1"),
        abi::add_registers("x2", "x9", "x2"),
        abi::multiply_registers("x3", "x13", "x1"),
        abi::add_registers("x3", "x9", "x3"),
    ];
    // swap COLLECTION_ENTRY_SIZE bytes (8 at a time)
    let mut offset = 0;
    while offset < COLLECTION_ENTRY_SIZE {
        instructions.extend([
            abi::load_u64("x4", "x2", offset),
            abi::load_u64("x5", "x3", offset),
            abi::store_u64("x5", "x2", offset),
            abi::store_u64("x4", "x3", offset),
        ]);
        offset += 8;
    }
    instructions.extend([
        abi::label(&no_swap),
        abi::add_immediate("x12", "x12", 1),
        abi::branch(&outer),
        abi::label(&done),
        abi::return_(),
    ]);
    CodeFunction {
        name: "runtime.sortStringList".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
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
        abi::compare_immediate("x0", "0"),
        abi::branch_ne(error_label),
    ]);
}

/// Lower the standalone UTF-8 validation helper. It takes a byte pointer in `x0`
/// and a byte length in `x1`, and returns `0` in `x0` when the buffer is
/// well-formed UTF-8 or `1` otherwise. It makes no calls, so it needs no stack
/// frame.
pub(super) fn lower_validate_utf8_helper() -> CodeFunction {
    let symbol = VALIDATE_UTF8_SYMBOL;
    let invalid = format!("{symbol}_invalid");
    let mut instructions = vec![abi::label("entry")];
    if std::env::var("MFB_ASCII").is_ok() {
        let lp = format!("{symbol}_lp");
        let ok = format!("{symbol}_ok");
        instructions.extend([
            abi::move_register("x9", "x0"),
            abi::move_register("x10", "x1"),
            abi::label(&lp),
            abi::compare_immediate("x10", "0"),
            abi::branch_eq(&ok),
            abi::load_u8("x11", "x9", 0),
            abi::compare_immediate("x11", "127"),
            abi::branch_hi(&invalid),
            abi::add_immediate("x9", "x9", 1),
            abi::subtract_immediate("x10", "x10", 1),
            abi::branch(&lp),
            abi::label(&ok),
            abi::move_immediate("x0", "Integer", "0"),
            abi::return_(),
            abi::label(&invalid),
            abi::move_immediate("x0", "Integer", "1"),
            abi::return_(),
        ]);
    } else {
        emit_validate_utf8(symbol, "x0", "x1", &invalid, &mut instructions);
        instructions.extend([
            abi::move_immediate("x0", "Integer", "0"),
            abi::return_(),
            abi::label(&invalid),
            abi::move_immediate("x0", "Integer", "1"),
            abi::return_(),
        ]);
    }
    CodeFunction {
        name: "runtime.validateUtf8".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// Validate that the `len`-byte buffer at `ptr` is well-formed UTF-8, branching
/// to `error_label` on the first invalid sequence. Used by
/// [`lower_validate_utf8_helper`]. Clobbers `x9`-`x14`. `ptr` and `len` are read
/// into scratch registers before any clobber, so they may name `x0`/`x1`.
fn emit_validate_utf8(
    symbol: &str,
    ptr: &str,
    len: &str,
    error_label: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let pos = "x9";
    let rem = "x10";
    let byte = "x11";
    let cont = "x12";
    let lo = "x13";
    let hi = "x14";

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
    if instructions.iter().any(|instruction| {
        instruction.op == CodeOp::BranchLink || instruction.op == CodeOp::BranchLinkRegister
    }) && !callee_saved
        .iter()
        .any(|register| register == abi::link_register())
    {
        callee_saved.push(abi::link_register().to_string());
    }
    let save_size = callee_saved.len() * 8;
    let total_stack_size = align(save_size + local_stack_size, 16);
    if total_stack_size == 0 {
        return CodeFrame {
            stack_size: 0,
            callee_saved,
        };
    }

    for slot in stack_slots {
        slot.offset += save_size as i32;
    }
    adjust_stack_instruction_offsets(instructions, save_size);

    let mut prologue = Vec::new();
    prologue.push(abi::subtract_stack(total_stack_size));
    for (index, register) in callee_saved.iter().enumerate() {
        prologue.push(save_callee_saved(register, index * 8));
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
                rewritten.push(restore_callee_saved(register, index * 8));
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
    instructions: Vec<CodeInstruction>,
    relocations: Vec<CodeRelocation>,
) -> CodeFunction {
    finalize_vreg_helper_reserved(name, symbol, returns, instructions, relocations, &[])
}

/// Like [`finalize_vreg_helper`], but holds `reserved` physical registers out of
/// allocation entirely. A helper whose hand-written callers rely on it preserving
/// a specific register across the call (the `_mfb_arena_alloc` `x8/x11/x12/x13/x17`
/// survivor contract, `.ai/compiler.md`) reserves that register so the allocator
/// never colors a value or spill scratch onto it — keeping the migrated helper's
/// clobber set identical to the hand-written original it replaces.
pub(super) fn finalize_vreg_helper_reserved(
    name: &str,
    symbol: &str,
    returns: &str,
    mut instructions: Vec<CodeInstruction>,
    relocations: Vec<CodeRelocation>,
    reserved: &[&str],
) -> CodeFunction {
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, reserved);
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
    let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    let outcome = regalloc::allocate(
        regalloc::active_kind(),
        instructions,
        &[],
        &[],
        &model,
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
    let stack_size = local_size + outcome.spill_slots.len() * 8;
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
    register
        .strip_prefix('d')
        .is_some_and(|rest| rest.parse::<u8>().is_ok())
}

fn save_callee_saved(register: &str, offset: usize) -> CodeInstruction {
    if is_fp_register(register) {
        abi::store_double(register, abi::stack_pointer(), offset)
    } else {
        abi::store_u64(register, abi::stack_pointer(), offset)
    }
}

fn restore_callee_saved(register: &str, offset: usize) -> CodeInstruction {
    if is_fp_register(register) {
        abi::load_double(register, abi::stack_pointer(), offset)
    } else {
        abi::load_u64(register, abi::stack_pointer(), offset)
    }
}

fn adjust_stack_instruction_offsets(instructions: &mut [CodeInstruction], offset_delta: usize) {
    if offset_delta == 0 {
        return;
    }
    for instruction in instructions {
        let stack_relative = instruction
            .fields
            .iter()
            .any(|(name, value)| matches!(*name, "base" | "src") && abi::is_stack_pointer(value));
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
