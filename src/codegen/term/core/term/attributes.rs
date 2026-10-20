//! plan-176-P: the terminal backend's colour and text attributes and cursor visibility (set and
//! get). The constants, data objects, `lower_term_helper` and `StampCtx` live in
//! `term/core/term/mod.rs`.

use super::*;

/// `term::setForeground`/`setBackground` (plan-35-B): pack `r|g<<8|b<<16` into the
/// term-state colour slot — the "current attribute" the grid writer stamps into
/// cells. Emits no ANSI; the colour is applied when `term::sync` presents.
pub(super) fn emit_set_color(
    symbol: &str,
    term_state_offset: usize,
    state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let v9: Reg = vregs.next().into();
    let v10: Reg = vregs.next().into();
    let v11: Reg = vregs.next().into();
    let v12: Reg = vregs.next().into();
    let inactive = format!("{symbol}_inactive");
    emit_gate_inactive(term_state_offset, &inactive, v9, instructions);
    // plan-122-F: the single MFB argument is a `color::Color` RECORD POINTER, in
    // `c_arg(0)` (established by reading this file's own pre-change unpacking, which
    // took its three `Byte` channels from `c_arg(0..2)` — NOT `return_register()`,
    // which `emit_get_color` twenty lines below uses for the arena allocator's first
    // argument).
    //
    // The pointer is moved into a callee-usable temporary FIRST and every field is
    // then loaded off that temporary. Loading straight out of `c_arg(0)` while also
    // writing the channel vregs would be safe here today, but staging is the rule for
    // this file: an emitter that writes an argument slot before every incoming
    // argument is read destroys one, and the symptom is a SIGSEGV at a tiny address
    // rather than a wrong colour.
    //
    // Field offsets are the record's declaration order — red 0, green 8, blue 16,
    // alpha 24 — matching what `emit_get_color` stores. ALPHA IS DELIBERATELY NOT
    // READ: a terminal cell has no alpha channel, and the state slot it packs into
    // is only 0xBBGGRR.
    instructions.extend([
        abi::move_register(&v12, abi::c_arg(0)),
        abi::load_u64(&v9, &v12, 0),
        abi::load_u64(&v10, &v12, 8),
        abi::load_u64(&v11, &v12, 16),
        abi::shift_left_immediate(&v10, &v10, 8),
        abi::shift_left_immediate(&v11, &v11, 16),
        abi::or_registers(&v9, &v9, &v10),
        abi::or_registers(&v9, &v9, &v11),
        abi::store_u64(&v9, ARENA_STATE_REGISTER, state_offset),
    ]);
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::setBold`/`setUnderline` (plan-35-B): store the flag into its term-state
/// slot (the current attribute). Emits no ANSI.
pub(super) fn emit_set_attr(
    symbol: &str,
    term_state_offset: usize,
    state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let v9: Reg = vregs.next().into();
    let inactive = format!("{symbol}_inactive");
    emit_gate_inactive(term_state_offset, &inactive, v9, instructions);
    instructions.push(abi::move_register(&v9, abi::c_arg(0)));
    instructions.push(abi::store_u64(&v9, ARENA_STATE_REGISTER, state_offset));
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::showCursor`/`hideCursor` (plan-35-B): store the cursor-visible flag; the
/// present applies it. Emits no ANSI.
pub(super) fn emit_set_cursor_visible(
    symbol: &str,
    term_state_offset: usize,
    value: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let v9: Reg = vregs.next().into();
    let inactive = format!("{symbol}_inactive");
    emit_gate_inactive(term_state_offset, &inactive, v9, instructions);
    instructions.push(abi::move_immediate(&v9, "Integer", value));
    instructions.push(abi::store_u64(
        &v9,
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_CURSOR_VISIBLE_OFFSET,
    ));
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_get_color(
    symbol: &str,
    term_state_offset: usize,
    state_offset: usize,
    inert_packed: &str,
    done: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let rec: Reg = vregs.next().into();
    let packed: Reg = vregs.next().into();
    let mask: Reg = vregs.next().into();
    let channel: Reg = vregs.next().into();
    let shifted: Reg = vregs.next().into();
    let inert = format!("{symbol}_inert");
    let have_src = format!("{symbol}_have_src");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    // Determine the source packed value: state when active, the inert default
    // otherwise (§4.2.1). Park it in the scratch arg slot.
    emit_gate_inactive(term_state_offset, &inert, rec, instructions);
    instructions.push(abi::load_u64(&packed, ARENA_STATE_REGISTER, state_offset));
    instructions.push(abi::branch(&have_src));
    instructions.push(abi::label(&inert));
    instructions.push(abi::move_immediate(&packed, "Integer", inert_packed));
    instructions.push(abi::label(&have_src));
    instructions.push(abi::store_u64(&packed, abi::stack_pointer(), ARG0_OFFSET));
    // Allocate the 4-field `color::Color` record (plan-122-F widened it from the
    // retired 3-field `TermColor`).
    instructions.extend([
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &COLOR_RECORD_SIZE.to_string(),
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&rec, RESULT_VALUE_REGISTER),
        abi::load_u64(&packed, abi::stack_pointer(), ARG0_OFFSET),
        abi::move_immediate(&mask, "Integer", "255"),
        abi::and_registers(&channel, &packed, &mask),
        abi::store_u64(&channel, &rec, 0),
        abi::shift_right_immediate(&shifted, &packed, 8),
        abi::and_registers(&channel, &shifted, &mask),
        abi::store_u64(&channel, &rec, 8),
        abi::shift_right_immediate(&shifted, &packed, 16),
        abi::and_registers(&channel, &shifted, &mask),
        abi::store_u64(&channel, &rec, 16),
        // alpha: a terminal cell has no alpha channel, so the fourth field is
        // always fully opaque. Stored as an immediate rather than unpacked from the
        // state slot, which only carries 0xBBGGRR (plan-122-F).
        abi::move_immediate(&channel, "Integer", "255"),
        abi::store_u64(&channel, &rec, 24),
        abi::move_register(RESULT_VALUE_REGISTER, &rec),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
        abi::label(&alloc_error),
    ]);
    raise_error_into(symbol, "ErrOutOfMemory", instructions, relocations);
}

pub(super) fn emit_get_attr(
    term_state_offset: usize,
    state_offset: usize,
    done: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let active: Reg = vregs.next().into();
    let inert = format!("term_get_attr_inert_{state_offset}");
    emit_gate_inactive(term_state_offset, &inert, active, instructions);
    instructions.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        ARENA_STATE_REGISTER,
        state_offset,
    ));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    instructions.push(abi::branch(done));
    instructions.push(abi::label(&inert));
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}
