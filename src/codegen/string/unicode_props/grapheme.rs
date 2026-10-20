//! plan-176-P: grapheme-cluster segmentation — the break test, the Indic-conjunct seed and the
//! state update. The constants and UTF-8 walking live in `string/unicode_props/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn emit_grapheme_break_branch(
        &mut self,
        state_bc: impl Into<abi::Reg>,
        state_icb: impl Into<abi::Reg>,
        current_bc: impl Into<abi::Reg>,
        current_icb: impl Into<abi::Reg>,
        break_label: &str,
        no_break_label: &str,
    ) {
        let no_break = self.label("grapheme_simple_no_break");
        let maybe_break = self.label("grapheme_maybe_break");
        let gb3_not_cr = self.label("grapheme_gb3_not_cr");
        let gb4_not_control = self.label("grapheme_gb4_not_control");
        let gb5_not_control = self.label("grapheme_gb5_not_control");
        let gb6_check = self.label("grapheme_gb6_check");
        let gb7_check = self.label("grapheme_gb7_check");
        let gb7_no = self.label("grapheme_gb7_no");
        let gb8_check = self.label("grapheme_gb8_check");
        let gb8_no = self.label("grapheme_gb8_no");
        let gb9_check = self.label("grapheme_gb9_check");
        let gb11_check = self.label("grapheme_gb11_check");
        let gb1213_check = self.label("grapheme_gb1213_check");
        let gb9c_break = self.label("grapheme_gb9c_break");
        let state_bc: abi::Reg = state_bc.into();
        let current_bc: abi::Reg = current_bc.into();

        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_CR));
        self.emit(abi::branch_ne(&gb3_not_cr));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_LF));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::label(&gb3_not_cr));

        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_CR));
        self.emit(abi::branch_lo(&gb4_not_control));
        self.emit(abi::compare_immediate(
            state_bc,
            GRAPHEME_BOUNDCLASS_CONTROL,
        ));
        self.emit(abi::branch_le(&maybe_break));
        self.emit(abi::label(&gb4_not_control));

        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_CR));
        self.emit(abi::branch_lo(&gb5_not_control));
        self.emit(abi::compare_immediate(
            current_bc,
            GRAPHEME_BOUNDCLASS_CONTROL,
        ));
        self.emit(abi::branch_le(&maybe_break));
        self.emit(abi::label(&gb5_not_control));

        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_L));
        self.emit(abi::branch_ne(&gb6_check));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_L));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_V));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_LV));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_LVT));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::label(&gb6_check));

        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_LV));
        self.emit(abi::branch_eq(&gb7_check));
        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_V));
        self.emit(abi::branch_ne(&gb7_no));
        self.emit(abi::label(&gb7_check));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_V));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_T));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::label(&gb7_no));

        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_LVT));
        self.emit(abi::branch_eq(&gb8_check));
        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_T));
        self.emit(abi::branch_ne(&gb8_no));
        self.emit(abi::label(&gb8_check));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_T));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::label(&gb8_no));

        self.emit(abi::label(&gb9_check));
        self.emit(abi::compare_immediate(
            current_bc,
            GRAPHEME_BOUNDCLASS_EXTEND,
        ));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_ZWJ));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::compare_immediate(
            current_bc,
            GRAPHEME_BOUNDCLASS_SPACINGMARK,
        ));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::compare_immediate(
            state_bc,
            GRAPHEME_BOUNDCLASS_PREPEND,
        ));
        self.emit(abi::branch_eq(&no_break));

        self.emit(abi::label(&gb11_check));
        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_E_ZWG));
        self.emit(abi::branch_ne(&gb1213_check));
        self.emit(abi::compare_immediate(
            current_bc,
            GRAPHEME_BOUNDCLASS_EXTENDED_PICTOGRAPHIC,
        ));
        self.emit(abi::branch_eq(&no_break));

        self.emit(abi::label(&gb1213_check));
        self.emit(abi::compare_immediate(
            state_bc,
            GRAPHEME_BOUNDCLASS_REGIONAL_INDICATOR,
        ));
        self.emit(abi::branch_ne(&maybe_break));
        self.emit(abi::compare_immediate(
            current_bc,
            GRAPHEME_BOUNDCLASS_REGIONAL_INDICATOR,
        ));
        self.emit(abi::branch_eq(&no_break));

        self.emit(abi::label(&maybe_break));
        self.emit(abi::compare_immediate(
            state_icb,
            INDIC_CONJUNCT_BREAK_LINKER,
        ));
        self.emit(abi::branch_ne(&gb9c_break));
        self.emit(abi::compare_immediate(
            current_icb,
            INDIC_CONJUNCT_BREAK_CONSONANT,
        ));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::label(&gb9c_break));
        self.emit(abi::branch(break_label));

        self.emit(abi::label(&no_break));
        self.emit(abi::branch(no_break_label));
    }

    /// Turn the first scalar's raw Indic_Conjunct_Break property, in `icb`, into
    /// the GB9c context state a walk starts from (bug-796). Only a consonant opens
    /// a conjunct prefix. A leading linker or extend has no consonant before it, so
    /// the state is "none" (0), and a consonant after it breaks. Seeding the raw
    /// property made a leading `U+094D` look like "consonant + linker seen", so
    /// `U+094D U+0915` came out as one cluster.
    pub(crate) fn emit_seed_indic_conjunct_state(&mut self, icb: impl Into<abi::Reg>) {
        let seeded = self.label("grapheme_icb_seeded");
        let mut instrs = Vec::new();
        emit_seed_indic_conjunct_state_instrs(icb, &seeded, &mut instrs);
        for instruction in instrs {
            self.emit(instruction);
        }
    }

    pub(crate) fn emit_grapheme_state_update(
        &mut self,
        state_bc: impl Into<abi::Reg>,
        state_icb: impl Into<abi::Reg>,
        current_bc: impl Into<abi::Reg>,
        current_icb: impl Into<abi::Reg>,
    ) {
        let icb_consonant = self.label("grapheme_icb_consonant");
        let icb_existing_consonant = self.label("grapheme_icb_existing_consonant");
        let icb_existing_extend = self.label("grapheme_icb_existing_extend");
        let icb_linker = self.label("grapheme_icb_linker");
        let icb_linker_extend = self.label("grapheme_icb_linker_extend");
        let icb_done = self.label("grapheme_icb_done");
        let bc_ri_check = self.label("grapheme_bc_ri_check");
        let bc_extpic_check = self.label("grapheme_bc_extpic_check");
        let bc_extpic_extend = self.label("grapheme_bc_extpic_extend");
        let bc_extpic_zwj = self.label("grapheme_bc_extpic_zwj");
        let bc_set_current = self.label("grapheme_bc_set_current");
        let bc_done = self.label("grapheme_bc_done");
        let state_bc: abi::Reg = state_bc.into();
        let state_icb: abi::Reg = state_icb.into();
        let current_bc: abi::Reg = current_bc.into();
        let current_icb: abi::Reg = current_icb.into();

        self.emit(abi::compare_immediate(
            current_icb,
            INDIC_CONJUNCT_BREAK_CONSONANT,
        ));
        self.emit(abi::branch_eq(&icb_consonant));
        self.emit(abi::compare_immediate(
            state_icb,
            INDIC_CONJUNCT_BREAK_CONSONANT,
        ));
        self.emit(abi::branch_eq(&icb_existing_consonant));
        self.emit(abi::compare_immediate(
            state_icb,
            INDIC_CONJUNCT_BREAK_EXTEND,
        ));
        self.emit(abi::branch_eq(&icb_existing_extend));
        self.emit(abi::compare_immediate(
            state_icb,
            INDIC_CONJUNCT_BREAK_LINKER,
        ));
        self.emit(abi::branch_eq(&icb_linker));
        self.emit(abi::branch(&icb_done));
        self.emit(abi::label(&icb_consonant));
        self.emit(abi::move_register(state_icb, current_icb));
        self.emit(abi::branch(&icb_done));
        self.emit(abi::label(&icb_existing_consonant));
        self.emit(abi::move_register(state_icb, current_icb));
        self.emit(abi::branch(&icb_done));
        self.emit(abi::label(&icb_existing_extend));
        self.emit(abi::move_register(state_icb, current_icb));
        self.emit(abi::branch(&icb_done));
        self.emit(abi::label(&icb_linker));
        self.emit(abi::compare_immediate(
            current_icb,
            INDIC_CONJUNCT_BREAK_EXTEND,
        ));
        self.emit(abi::branch_eq(&icb_linker_extend));
        self.emit(abi::move_register(state_icb, current_icb));
        self.emit(abi::branch(&icb_done));
        self.emit(abi::label(&icb_linker_extend));
        self.emit(abi::move_immediate(
            state_icb,
            "Integer",
            INDIC_CONJUNCT_BREAK_LINKER,
        ));
        self.emit(abi::label(&icb_done));

        self.emit(abi::compare_registers(state_bc, current_bc));
        self.emit(abi::branch_ne(&bc_extpic_check));
        self.emit(abi::compare_immediate(
            current_bc,
            GRAPHEME_BOUNDCLASS_REGIONAL_INDICATOR,
        ));
        self.emit(abi::branch_eq(&bc_ri_check));
        self.emit(abi::label(&bc_extpic_check));
        self.emit(abi::compare_immediate(
            state_bc,
            GRAPHEME_BOUNDCLASS_EXTENDED_PICTOGRAPHIC,
        ));
        self.emit(abi::branch_ne(&bc_set_current));
        self.emit(abi::compare_immediate(
            current_bc,
            GRAPHEME_BOUNDCLASS_EXTEND,
        ));
        self.emit(abi::branch_eq(&bc_extpic_extend));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_ZWJ));
        self.emit(abi::branch_eq(&bc_extpic_zwj));
        self.emit(abi::branch(&bc_set_current));
        self.emit(abi::label(&bc_ri_check));
        self.emit(abi::move_immediate(state_bc, "Integer", "1"));
        self.emit(abi::branch(&bc_done));
        self.emit(abi::label(&bc_extpic_extend));
        self.emit(abi::move_immediate(
            state_bc,
            "Integer",
            GRAPHEME_BOUNDCLASS_EXTENDED_PICTOGRAPHIC,
        ));
        self.emit(abi::branch(&bc_done));
        self.emit(abi::label(&bc_extpic_zwj));
        self.emit(abi::move_immediate(
            state_bc,
            "Integer",
            GRAPHEME_BOUNDCLASS_E_ZWG,
        ));
        self.emit(abi::branch(&bc_done));
        self.emit(abi::label(&bc_set_current));
        self.emit(abi::move_register(state_bc, current_bc));
        self.emit(abi::label(&bc_done));
    }
}
