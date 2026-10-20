//! plan-176-P: free-function mirrors of the Unicode primitives, for lowerings that emit into a
//! plain instruction list rather than a `CodeBuilder`. The constants live in
//! `string/unicode_props/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// Free-function Unicode primitives for the neutral console codegen (plan-70-B/C).
//
// `term_grid.rs`/`term.rs` build their writer/present/stamp loops as free
// functions that push raw `abi::` instructions into a `Vec<CodeInstruction>` and
// their relocations into a `Vec<CodeRelocation>` — they are NOT `CodeBuilder`
// methods, so they cannot call the `emit_unicode_*` helpers above (those go
// through `self.emit`/`self.relocations`). These free mirrors take the current
// function `symbol` (the relocation `from`), an explicit `label_prefix` for
// unique labels, and caller-chosen scratch vregs, so the console width path can
// reuse the exact two-stage property walk without a `CodeBuilder`.

/// Load the address of an internal data `symbol` into `register` via the
/// `adrp`/`add` page pair, recording the two data-address relocations against
/// `from`. Free mirror of `CodeBuilder::emit_load_data_address`.
pub(crate) fn emit_load_data_address_free(
    from: &str,
    symbol: &str,
    register: impl Into<abi::Reg>,
    instrs: &mut Vec<CodeInstruction>,
    relocs: &mut Vec<CodeRelocation>,
) {
    let register: abi::Reg = register.into();
    instrs.push(abi::load_page_address(register, symbol));
    relocs.push(CodeRelocation {
        from: from.to_string(),
        to: symbol.to_string(),
        kind: RelocIntent::DataAddrHi,
        binding: "data".to_string(),
        library: None,
    });
    instrs.push(abi::add_page_offset(register, register, symbol));
    relocs.push(CodeRelocation {
        from: from.to_string(),
        to: symbol.to_string(),
        kind: RelocIntent::DataAddrLo,
        binding: "data".to_string(),
        library: None,
    });
}

/// Decode the UTF-8 scalar at `ptr` (whose byte length `len` is already known,
/// 1..=4, with lead byte in `lead`) into `codepoint`. Standard mask/shift decode;
/// the caller has already clamped `len` to the remaining bytes, so this never
/// reads past the string. `mask`/`byte` are caller-owned throwaway vregs. Used by
/// the console writer/stamp path (plan-70-B/C) to recover a codepoint for the
/// width lookup after the raw bytes were packed into the cell glyph.
pub(crate) fn emit_utf8_codepoint_by_len(
    label_prefix: &str,
    ptr: impl Into<abi::Reg>,
    len: impl Into<abi::Reg>,
    lead: impl Into<abi::Reg>,
    codepoint: impl Into<abi::Reg>,
    byte: impl Into<abi::Reg>,
    mask: impl Into<abi::Reg>,
    instrs: &mut Vec<CodeInstruction>,
) {
    let ptr: abi::Reg = ptr.into();
    let len: abi::Reg = len.into();
    let lead: abi::Reg = lead.into();
    let codepoint: abi::Reg = codepoint.into();
    let byte: abi::Reg = byte.into();
    let mask: abi::Reg = mask.into();
    let do2 = format!("{label_prefix}_cp2");
    let do3 = format!("{label_prefix}_cp3");
    let done = format!("{label_prefix}_cpdone");
    // Continuation byte payload mask.
    // len == 1 (default): codepoint = lead.
    instrs.push(abi::move_register(codepoint, lead));
    instrs.push(abi::compare_immediate(len, "2"));
    instrs.push(abi::branch_lo(&done));
    instrs.push(abi::branch_eq(&do2));
    instrs.push(abi::compare_immediate(len, "3"));
    instrs.push(abi::branch_eq(&do3));
    // len == 4: (lead & 0x07)<<18 | (b1&0x3F)<<12 | (b2&0x3F)<<6 | (b3&0x3F)
    instrs.push(abi::move_immediate(mask, "Integer", "7"));
    instrs.push(abi::and_registers(codepoint, lead, mask));
    instrs.push(abi::shift_left_immediate(codepoint, codepoint, 18));
    instrs.push(abi::move_immediate(mask, "Integer", "63"));
    instrs.push(abi::load_u8(byte, ptr, 1));
    instrs.push(abi::and_registers(byte, byte, mask));
    instrs.push(abi::shift_left_immediate(byte, byte, 12));
    instrs.push(abi::or_registers(codepoint, codepoint, byte));
    instrs.push(abi::load_u8(byte, ptr, 2));
    instrs.push(abi::and_registers(byte, byte, mask));
    instrs.push(abi::shift_left_immediate(byte, byte, 6));
    instrs.push(abi::or_registers(codepoint, codepoint, byte));
    instrs.push(abi::load_u8(byte, ptr, 3));
    instrs.push(abi::and_registers(byte, byte, mask));
    instrs.push(abi::or_registers(codepoint, codepoint, byte));
    instrs.push(abi::branch(&done));
    // len == 3: (lead & 0x0F)<<12 | (b1&0x3F)<<6 | (b2&0x3F)
    instrs.push(abi::label(&do3));
    instrs.push(abi::move_immediate(mask, "Integer", "15"));
    instrs.push(abi::and_registers(codepoint, lead, mask));
    instrs.push(abi::shift_left_immediate(codepoint, codepoint, 12));
    instrs.push(abi::move_immediate(mask, "Integer", "63"));
    instrs.push(abi::load_u8(byte, ptr, 1));
    instrs.push(abi::and_registers(byte, byte, mask));
    instrs.push(abi::shift_left_immediate(byte, byte, 6));
    instrs.push(abi::or_registers(codepoint, codepoint, byte));
    instrs.push(abi::load_u8(byte, ptr, 2));
    instrs.push(abi::and_registers(byte, byte, mask));
    instrs.push(abi::or_registers(codepoint, codepoint, byte));
    instrs.push(abi::branch(&done));
    // len == 2: (lead & 0x1F)<<6 | (b1&0x3F)
    instrs.push(abi::label(&do2));
    instrs.push(abi::move_immediate(mask, "Integer", "31"));
    instrs.push(abi::and_registers(codepoint, lead, mask));
    instrs.push(abi::shift_left_immediate(codepoint, codepoint, 6));
    instrs.push(abi::move_immediate(mask, "Integer", "63"));
    instrs.push(abi::load_u8(byte, ptr, 1));
    instrs.push(abi::and_registers(byte, byte, mask));
    instrs.push(abi::or_registers(codepoint, codepoint, byte));
    instrs.push(abi::label(&done));
}

/// Compute the address of `codepoint`'s property record into `prop_out` via the
/// two-stage trie. A codepoint `>= 0x110000` yields the default (index-0) record,
/// matching `property_for_codepoint`'s guard and avoiding an OOB stage-1 read.
/// `scratch` is a caller-owned throwaway vreg. The console cluster walk
/// (plan-70-B Phase 2) reads boundclass (offset 18), indic-conjunct-break (offset
/// 20), and charwidth (`(flags@16 >> 4) & 3`) straight from `prop_out`.
pub(crate) fn emit_unicode_property_ptr_free(
    from: &str,
    label_prefix: &str,
    codepoint: impl Into<abi::Reg>,
    prop_out: impl Into<abi::Reg>,
    scratch: impl Into<abi::Reg>,
    instrs: &mut Vec<CodeInstruction>,
    relocs: &mut Vec<CodeRelocation>,
) {
    let codepoint: abi::Reg = codepoint.into();
    let prop_out: abi::Reg = prop_out.into();
    let scratch: abi::Reg = scratch.into();
    let in_range = format!("{label_prefix}_pp_in");
    let done = format!("{label_prefix}_pp_done");
    instrs.push(abi::move_immediate(scratch, "Integer", "1114112"));
    instrs.push(abi::compare_registers(codepoint, scratch));
    instrs.push(abi::branch_lo(&in_range));
    // Out of range → the index-0 (default) record.
    emit_load_data_address_free(from, UNICODE_PROPERTIES_SYMBOL, prop_out, instrs, relocs);
    instrs.push(abi::branch(&done));
    instrs.push(abi::label(&in_range));
    // stage1[cp >> 8]
    instrs.push(abi::shift_right_immediate(prop_out, codepoint, 8));
    instrs.push(abi::shift_left_immediate(prop_out, prop_out, 1));
    emit_load_data_address_free(from, UNICODE_STAGE1_SYMBOL, scratch, instrs, relocs);
    instrs.push(abi::add_registers(scratch, scratch, prop_out));
    instrs.push(abi::load_u16(prop_out, scratch, 0));
    // stage2[stage1 + (cp & 0xff)]
    instrs.push(abi::move_immediate(scratch, "Integer", "255"));
    instrs.push(abi::and_registers(scratch, codepoint, scratch));
    instrs.push(abi::add_registers(prop_out, prop_out, scratch));
    instrs.push(abi::shift_left_immediate(prop_out, prop_out, 1));
    emit_load_data_address_free(from, UNICODE_STAGE2_SYMBOL, scratch, instrs, relocs);
    instrs.push(abi::add_registers(scratch, scratch, prop_out));
    instrs.push(abi::load_u16(prop_out, scratch, 0));
    // properties + stage2 * PROPERTY_SIZE
    instrs.push(abi::move_immediate(
        scratch,
        "Integer",
        &UNICODE_PROPERTY_SIZE.to_string(),
    ));
    instrs.push(abi::multiply_registers(prop_out, prop_out, scratch));
    emit_load_data_address_free(from, UNICODE_PROPERTIES_SYMBOL, scratch, instrs, relocs);
    instrs.push(abi::add_registers(prop_out, scratch, prop_out));
    instrs.push(abi::label(&done));
}

/// The free-function form of [`CodeBuilder::emit_seed_indic_conjunct_state`],
/// for the hand-labelled cluster walks (bug-796).
pub(crate) fn emit_seed_indic_conjunct_state_free(
    label_prefix: &str,
    icb: impl Into<abi::Reg>,
    instrs: &mut Vec<CodeInstruction>,
) {
    emit_seed_indic_conjunct_state_instrs(icb, &format!("{label_prefix}_icbseed"), instrs);
}

pub(super) fn emit_seed_indic_conjunct_state_instrs(
    icb: impl Into<abi::Reg>,
    seeded: &str,
    instrs: &mut Vec<CodeInstruction>,
) {
    let icb: abi::Reg = icb.into();
    instrs.extend([
        abi::compare_immediate(icb, INDIC_CONJUNCT_BREAK_CONSONANT),
        abi::branch_eq(seeded),
        abi::move_immediate(icb, "Integer", "0"),
        abi::label(seeded),
    ]);
}

/// Read a property record's boundclass (offset 18), indic-conjunct-break (offset
/// 20), and charwidth (`(flags@16 >> 4) & 3`) from `prop` into the three outputs.
/// `scratch` is a caller-owned throwaway vreg. plan-70-B Phase 2 cluster walk.
pub(crate) fn emit_read_boundclass_icb_charwidth_free(
    prop: impl Into<abi::Reg>,
    bc_out: impl Into<abi::Reg>,
    icb_out: impl Into<abi::Reg>,
    width_out: impl Into<abi::Reg>,
    scratch: impl Into<abi::Reg>,
    instrs: &mut Vec<CodeInstruction>,
) {
    let prop: abi::Reg = prop.into();
    let bc_out: abi::Reg = bc_out.into();
    let icb_out: abi::Reg = icb_out.into();
    let width_out: abi::Reg = width_out.into();
    let scratch: abi::Reg = scratch.into();
    instrs.push(abi::load_u16(
        bc_out,
        prop,
        UNICODE_PROPERTY_OFFSET_BOUNDCLASS,
    ));
    instrs.push(abi::load_u16(
        icb_out,
        prop,
        UNICODE_PROPERTY_OFFSET_INDIC_CONJUNCT_BREAK,
    ));
    instrs.push(abi::load_u16(
        width_out,
        prop,
        UNICODE_PROPERTY_OFFSET_FLAGS,
    ));
    instrs.push(abi::shift_right_immediate(width_out, width_out, 4));
    instrs.push(abi::move_immediate(scratch, "Integer", "3"));
    instrs.push(abi::and_registers(width_out, width_out, scratch));
}

/// Free mirror of `CodeBuilder::emit_grapheme_break_branch` (UAX #29 GB rules):
/// branch to `break_label` if there is a cluster boundary between the previous
/// scalar (`state_bc`/`state_icb`) and the current one (`current_bc`/`current_icb`),
/// else to `no_break_label`. Uses only caller-passed registers + labels derived
/// from `label_prefix`; no scratch. plan-70-B Phase 2.
pub(crate) fn emit_grapheme_break_branch_free(
    label_prefix: &str,
    state_bc: impl Into<abi::Reg>,
    state_icb: impl Into<abi::Reg>,
    current_bc: impl Into<abi::Reg>,
    current_icb: impl Into<abi::Reg>,
    break_label: &str,
    no_break_label: &str,
    instrs: &mut Vec<CodeInstruction>,
) {
    let state_bc: abi::Reg = state_bc.into();
    let state_icb: abi::Reg = state_icb.into();
    let current_bc: abi::Reg = current_bc.into();
    let current_icb: abi::Reg = current_icb.into();
    let no_break = format!("{label_prefix}_gnb");
    let maybe_break = format!("{label_prefix}_gmb");
    let gb3_not_cr = format!("{label_prefix}_g3");
    let gb4_not_control = format!("{label_prefix}_g4");
    let gb5_not_control = format!("{label_prefix}_g5");
    let gb6_check = format!("{label_prefix}_g6");
    let gb7_check = format!("{label_prefix}_g7");
    let gb7_no = format!("{label_prefix}_g7n");
    let gb8_check = format!("{label_prefix}_g8");
    let gb8_no = format!("{label_prefix}_g8n");
    let gb9_check = format!("{label_prefix}_g9");
    let gb11_check = format!("{label_prefix}_g11");
    let gb1213_check = format!("{label_prefix}_g1213");
    let gb9c_break = format!("{label_prefix}_g9c");

    // GB3: CR × LF.
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_CR));
    instrs.push(abi::branch_ne(&gb3_not_cr));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_LF));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::label(&gb3_not_cr));
    // GB4: (CR|LF|Control) ÷.
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_CR));
    instrs.push(abi::branch_lo(&gb4_not_control));
    instrs.push(abi::compare_immediate(
        state_bc,
        GRAPHEME_BOUNDCLASS_CONTROL,
    ));
    instrs.push(abi::branch_le(&maybe_break));
    instrs.push(abi::label(&gb4_not_control));
    // GB5: ÷ (CR|LF|Control).
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_CR));
    instrs.push(abi::branch_lo(&gb5_not_control));
    instrs.push(abi::compare_immediate(
        current_bc,
        GRAPHEME_BOUNDCLASS_CONTROL,
    ));
    instrs.push(abi::branch_le(&maybe_break));
    instrs.push(abi::label(&gb5_not_control));
    // GB6: L × (L|V|LV|LVT).
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_L));
    instrs.push(abi::branch_ne(&gb6_check));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_L));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_V));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_LV));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_LVT));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::label(&gb6_check));
    // GB7: (LV|V) × (V|T).
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_LV));
    instrs.push(abi::branch_eq(&gb7_check));
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_V));
    instrs.push(abi::branch_ne(&gb7_no));
    instrs.push(abi::label(&gb7_check));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_V));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_T));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::label(&gb7_no));
    // GB8: (LVT|T) × T.
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_LVT));
    instrs.push(abi::branch_eq(&gb8_check));
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_T));
    instrs.push(abi::branch_ne(&gb8_no));
    instrs.push(abi::label(&gb8_check));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_T));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::label(&gb8_no));
    // GB9/GB9a/GB9b: × (Extend|ZWJ|SpacingMark), Prepend ×.
    instrs.push(abi::label(&gb9_check));
    instrs.push(abi::compare_immediate(
        current_bc,
        GRAPHEME_BOUNDCLASS_EXTEND,
    ));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_ZWJ));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::compare_immediate(
        current_bc,
        GRAPHEME_BOUNDCLASS_SPACINGMARK,
    ));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::compare_immediate(
        state_bc,
        GRAPHEME_BOUNDCLASS_PREPEND,
    ));
    instrs.push(abi::branch_eq(&no_break));
    // GB11: ExtPict ZWJ × ExtPict (state E_ZWG).
    instrs.push(abi::label(&gb11_check));
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_E_ZWG));
    instrs.push(abi::branch_ne(&gb1213_check));
    instrs.push(abi::compare_immediate(
        current_bc,
        GRAPHEME_BOUNDCLASS_EXTENDED_PICTOGRAPHIC,
    ));
    instrs.push(abi::branch_eq(&no_break));
    // GB12/GB13: RI × RI (paired).
    instrs.push(abi::label(&gb1213_check));
    instrs.push(abi::compare_immediate(
        state_bc,
        GRAPHEME_BOUNDCLASS_REGIONAL_INDICATOR,
    ));
    instrs.push(abi::branch_ne(&maybe_break));
    instrs.push(abi::compare_immediate(
        current_bc,
        GRAPHEME_BOUNDCLASS_REGIONAL_INDICATOR,
    ));
    instrs.push(abi::branch_eq(&no_break));
    // GB9c: Consonant [Extend] Linker × Consonant.
    instrs.push(abi::label(&maybe_break));
    instrs.push(abi::compare_immediate(
        state_icb,
        INDIC_CONJUNCT_BREAK_LINKER,
    ));
    instrs.push(abi::branch_ne(&gb9c_break));
    instrs.push(abi::compare_immediate(
        current_icb,
        INDIC_CONJUNCT_BREAK_CONSONANT,
    ));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::label(&gb9c_break));
    instrs.push(abi::branch(break_label));
    instrs.push(abi::label(&no_break));
    instrs.push(abi::branch(no_break_label));
}

/// Free mirror of `CodeBuilder::emit_grapheme_state_update`: advance the running
/// boundclass/indic-conjunct-break state (`state_bc`/`state_icb`) after consuming
/// the current scalar (`current_bc`/`current_icb`). plan-70-B Phase 2.
pub(crate) fn emit_grapheme_state_update_free(
    label_prefix: &str,
    state_bc: impl Into<abi::Reg>,
    state_icb: impl Into<abi::Reg>,
    current_bc: impl Into<abi::Reg>,
    current_icb: impl Into<abi::Reg>,
    instrs: &mut Vec<CodeInstruction>,
) {
    let state_bc: abi::Reg = state_bc.into();
    let state_icb: abi::Reg = state_icb.into();
    let current_bc: abi::Reg = current_bc.into();
    let current_icb: abi::Reg = current_icb.into();
    let icb_consonant = format!("{label_prefix}_uc");
    let icb_existing_consonant = format!("{label_prefix}_uec");
    let icb_existing_extend = format!("{label_prefix}_uee");
    let icb_linker = format!("{label_prefix}_ul");
    let icb_linker_extend = format!("{label_prefix}_ule");
    let icb_done = format!("{label_prefix}_ud");
    let bc_ri_check = format!("{label_prefix}_uri");
    let bc_extpic_check = format!("{label_prefix}_uep");
    let bc_extpic_extend = format!("{label_prefix}_uepe");
    let bc_extpic_zwj = format!("{label_prefix}_uepz");
    let bc_set_current = format!("{label_prefix}_usc");
    let bc_done = format!("{label_prefix}_ubd");

    instrs.push(abi::compare_immediate(
        current_icb,
        INDIC_CONJUNCT_BREAK_CONSONANT,
    ));
    instrs.push(abi::branch_eq(&icb_consonant));
    instrs.push(abi::compare_immediate(
        state_icb,
        INDIC_CONJUNCT_BREAK_CONSONANT,
    ));
    instrs.push(abi::branch_eq(&icb_existing_consonant));
    instrs.push(abi::compare_immediate(
        state_icb,
        INDIC_CONJUNCT_BREAK_EXTEND,
    ));
    instrs.push(abi::branch_eq(&icb_existing_extend));
    instrs.push(abi::compare_immediate(
        state_icb,
        INDIC_CONJUNCT_BREAK_LINKER,
    ));
    instrs.push(abi::branch_eq(&icb_linker));
    instrs.push(abi::branch(&icb_done));
    instrs.push(abi::label(&icb_consonant));
    instrs.push(abi::move_register(state_icb, current_icb));
    instrs.push(abi::branch(&icb_done));
    instrs.push(abi::label(&icb_existing_consonant));
    instrs.push(abi::move_register(state_icb, current_icb));
    instrs.push(abi::branch(&icb_done));
    instrs.push(abi::label(&icb_existing_extend));
    instrs.push(abi::move_register(state_icb, current_icb));
    instrs.push(abi::branch(&icb_done));
    instrs.push(abi::label(&icb_linker));
    instrs.push(abi::compare_immediate(
        current_icb,
        INDIC_CONJUNCT_BREAK_EXTEND,
    ));
    instrs.push(abi::branch_eq(&icb_linker_extend));
    instrs.push(abi::move_register(state_icb, current_icb));
    instrs.push(abi::branch(&icb_done));
    instrs.push(abi::label(&icb_linker_extend));
    instrs.push(abi::move_immediate(
        state_icb,
        "Integer",
        INDIC_CONJUNCT_BREAK_LINKER,
    ));
    instrs.push(abi::label(&icb_done));

    instrs.push(abi::compare_registers(state_bc, current_bc));
    instrs.push(abi::branch_ne(&bc_extpic_check));
    instrs.push(abi::compare_immediate(
        current_bc,
        GRAPHEME_BOUNDCLASS_REGIONAL_INDICATOR,
    ));
    instrs.push(abi::branch_eq(&bc_ri_check));
    instrs.push(abi::label(&bc_extpic_check));
    instrs.push(abi::compare_immediate(
        state_bc,
        GRAPHEME_BOUNDCLASS_EXTENDED_PICTOGRAPHIC,
    ));
    instrs.push(abi::branch_ne(&bc_set_current));
    instrs.push(abi::compare_immediate(
        current_bc,
        GRAPHEME_BOUNDCLASS_EXTEND,
    ));
    instrs.push(abi::branch_eq(&bc_extpic_extend));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_ZWJ));
    instrs.push(abi::branch_eq(&bc_extpic_zwj));
    instrs.push(abi::branch(&bc_set_current));
    instrs.push(abi::label(&bc_ri_check));
    instrs.push(abi::move_immediate(state_bc, "Integer", "1"));
    instrs.push(abi::branch(&bc_done));
    instrs.push(abi::label(&bc_extpic_extend));
    instrs.push(abi::move_immediate(
        state_bc,
        "Integer",
        GRAPHEME_BOUNDCLASS_EXTENDED_PICTOGRAPHIC,
    ));
    instrs.push(abi::branch(&bc_done));
    instrs.push(abi::label(&bc_extpic_zwj));
    instrs.push(abi::move_immediate(
        state_bc,
        "Integer",
        GRAPHEME_BOUNDCLASS_E_ZWG,
    ));
    instrs.push(abi::branch(&bc_done));
    instrs.push(abi::label(&bc_set_current));
    instrs.push(abi::move_register(state_bc, current_bc));
    instrs.push(abi::label(&bc_done));
}
