//! plan-176-P: Unicode property lookups — the property record, range tables, the per-field
//! readers, the u32 mappings and Hangul composition. The constants, UTF-8 walking and whitespace
//! tests live in `string/unicode_props/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn emit_unicode_property_lookup(
        &mut self,
        codepoint: impl Into<abi::Reg>,
        property: impl Into<abi::Reg>,
    ) {
        let index = self.temporary_vreg();
        let table = self.temporary_vreg();
        let index = &index;
        let table = &table;
        let codepoint: abi::Reg = codepoint.into();
        let property: abi::Reg = property.into();
        self.emit(abi::shift_right_immediate(index, codepoint, 8));
        self.emit(abi::shift_left_immediate(index, index, 1));
        self.emit_load_data_address(table, UNICODE_STAGE1_SYMBOL);
        self.emit(abi::add_registers(table, table, index));
        self.emit(abi::load_u16(index, table, 0));
        self.emit(abi::move_immediate(table, "Integer", "255"));
        self.emit(abi::and_registers(table, codepoint, table));
        self.emit(abi::add_registers(index, index, table));
        self.emit(abi::shift_left_immediate(index, index, 1));
        self.emit_load_data_address(table, UNICODE_STAGE2_SYMBOL);
        self.emit(abi::add_registers(table, table, index));
        self.emit(abi::load_u16(index, table, 0));
        self.emit(abi::move_immediate(
            table,
            "Integer",
            &UNICODE_PROPERTY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(index, index, table));
        self.emit_load_data_address(property, UNICODE_PROPERTIES_SYMBOL);
        self.emit(abi::add_registers(property, property, index));
    }

    /// Look `codepoint` up in one of the pinned Unicode **run tables** and leave
    /// a `String` pointer to its property value in `output`.
    ///
    /// `runs_symbol` names an ascending array of `(u32 lastCodepoint, u32
    /// nameIndex)` records covering `0 ..= 0x10FFFF`; `names_symbol` names
    /// `mfb.string.v1` records at a fixed `name_stride`. The search is the
    /// standard lower-bound bisection — invariant: the answer is in
    /// `lo ..= hi` — so it is `ceil(log2(runs))` iterations: **12** for the
    /// 4,099-run general-category table, against the up-to-4,099 compares the
    /// generated `__regex_genCat` IF-chain it replaces performed (plan-118-B).
    ///
    /// `output` is a rodata pointer, not an allocation: the callers compare the
    /// returned category/script and drop it, and a bare `String` temp is never
    /// freed (`builder_values.rs::register_pending_temp`), so nothing tries to
    /// return this pointer to the arena.
    pub(crate) fn emit_unicode_range_lookup(
        &mut self,
        codepoint: impl Into<abi::Reg>,
        output: impl Into<abi::Reg>,
        runs_symbol: &str,
        names_symbol: &str,
        runs: usize,
        name_stride: usize,
    ) {
        let output: abi::Reg = output.into();
        let (addr, scratch) = self.emit_unicode_range_search(codepoint, runs_symbol, runs);
        // `RUNS[lo].nameIndex`, then `NAMES + index * stride`.
        self.emit(abi::load_u32(&addr, &addr, 4));
        self.emit(abi::move_immediate(
            &scratch,
            "Integer",
            &name_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&addr, &addr, &scratch));
        self.emit_load_data_address(output, names_symbol);
        self.emit(abi::add_registers(output, output, &addr));
    }

    /// The bisection `emit_unicode_range_lookup` runs, answering the found run's second
    /// word itself: the property value, for a table whose runs carry the value rather
    /// than a name index (`strings::lineBreakClass`, plan-162-E).
    pub(crate) fn emit_unicode_range_value(
        &mut self,
        codepoint: impl Into<abi::Reg>,
        output: impl Into<abi::Reg>,
        runs_symbol: &str,
        runs: usize,
    ) {
        let output: abi::Reg = output.into();
        let (addr, _) = self.emit_unicode_range_search(codepoint, runs_symbol, runs);
        self.emit(abi::load_u32(output, &addr, 4));
    }

    /// The lower-bound bisection both lookups share. It leaves `&RUNS[lo]` in the
    /// returned `addr` and hands back its `scratch` as well, so the name lookup reuses
    /// both rather than allocating two more temporaries. That keeps the lookup's code
    /// byte-identical to what it was before `emit_unicode_range_value` existed.
    fn emit_unicode_range_search(
        &mut self,
        codepoint: impl Into<abi::Reg>,
        runs_symbol: &str,
        runs: usize,
    ) -> (VirtualRegister, VirtualRegister) {
        let codepoint: abi::Reg = codepoint.into();
        let low = self.temporary_vreg();
        let high = self.temporary_vreg();
        let mid = self.temporary_vreg();
        let addr = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        let (low, high, mid, addr, scratch) = (&low, &high, &mid, &addr, &scratch);

        let loop_label = self.label("unicode_range_loop");
        let take_upper = self.label("unicode_range_upper");
        let found = self.label("unicode_range_found");

        self.emit(abi::move_immediate(low, "Integer", "0"));
        self.emit(abi::move_immediate(
            high,
            "Integer",
            &(runs.saturating_sub(1)).to_string(),
        ));

        self.emit(abi::label(&loop_label));
        // `lo >= hi` means the range has collapsed onto the answer.
        self.emit(abi::compare_registers(low, high));
        self.emit(abi::branch_ge(&found));
        self.emit(abi::add_registers(mid, low, high));
        self.emit(abi::shift_right_immediate(mid, mid, 1));
        // `RUNS[mid].lastCodepoint`
        self.emit(abi::shift_left_immediate(addr, mid, 3));
        self.emit_load_data_address(scratch, runs_symbol);
        self.emit(abi::add_registers(addr, addr, scratch));
        self.emit(abi::load_u32(scratch, addr, 0));
        self.emit(abi::compare_registers(codepoint, scratch));
        self.emit(abi::branch_gt(&take_upper));
        // cp <= end: `mid` still covers it, so it becomes the new upper bound.
        self.emit(abi::move_register(high, mid));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&take_upper));
        self.emit(abi::add_immediate(low, mid, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&found));
        // `&RUNS[lo]`; the caller reads the run's second word.
        self.emit(abi::shift_left_immediate(addr, low, 3));
        self.emit_load_data_address(scratch, runs_symbol);
        self.emit(abi::add_registers(addr, addr, scratch));
        (*addr, *scratch)
    }

    /// The shared body of the `genCat` / `scriptOf` internal members: look the
    /// one `Integer` argument up in `table` and produce its property value as a
    /// `String`.
    ///
    /// `regex` and `strings` each register their own member over the same
    /// table (a package's injected companion may only call its OWN package's
    /// members -- the `astrings::scalarLen` pattern -- and making one import the
    /// other to share a symbol would drag a whole package into every program
    /// using the other). Both route here, so there is one lowering.
    pub(crate) fn lower_unicode_range_member(
        &mut self,
        call: &str,
        args: &[ValueResult],
        table: &crate::unicode::range_tables::RangeTable,
        runs_symbol: &str,
        names_symbol: &str,
    ) -> Result<ValueResult, String> {
        if args.len() != 1 {
            return Err(format!("{call}: no native lowering for these arguments"));
        }
        let result = self.allocate_register();
        let codepoint = args[0].location.clone();
        self.emit_unicode_range_lookup(
            codepoint,
            result,
            runs_symbol,
            names_symbol,
            table.runs.len(),
            table.name_stride(),
        );
        Ok(ValueResult {
            origin: None,
            type_: crate::types::ParameterType::String,
            location: Operand::from(result),
            text: call.to_string(),
        })
    }

    pub(crate) fn emit_unicode_property_boundclass(
        &mut self,
        property: impl Into<abi::Reg>,
        output: impl Into<abi::Reg>,
    ) {
        self.emit(abi::load_u16(
            output,
            property,
            UNICODE_PROPERTY_OFFSET_BOUNDCLASS,
        ));
    }

    pub(crate) fn emit_unicode_property_u16(
        &mut self,
        property: impl Into<abi::Reg>,
        output: impl Into<abi::Reg>,
        offset: usize,
    ) {
        self.emit(abi::load_u16(output, property, offset));
    }

    pub(crate) fn emit_unicode_property_combining_class(
        &mut self,
        property: impl Into<abi::Reg>,
        output: impl Into<abi::Reg>,
    ) {
        self.emit_unicode_property_u16(property, output, UNICODE_PROPERTY_OFFSET_COMBINING_CLASS);
    }

    pub(crate) fn emit_unicode_property_comb_index(
        &mut self,
        property: impl Into<abi::Reg>,
        output: impl Into<abi::Reg>,
    ) {
        self.emit_unicode_property_u16(property, output, UNICODE_PROPERTY_OFFSET_COMB_INDEX);
    }

    pub(crate) fn emit_unicode_property_comb_length(
        &mut self,
        property: impl Into<abi::Reg>,
        output: impl Into<abi::Reg>,
    ) {
        self.emit_unicode_property_u16(property, output, UNICODE_PROPERTY_OFFSET_COMB_LENGTH);
    }

    pub(crate) fn emit_unicode_property_flags(
        &mut self,
        property: impl Into<abi::Reg>,
        output: impl Into<abi::Reg>,
    ) {
        self.emit_unicode_property_u16(property, output, UNICODE_PROPERTY_OFFSET_FLAGS);
    }

    pub(crate) fn emit_unicode_property_indic_conjunct_break(
        &mut self,
        property: impl Into<abi::Reg>,
        output: impl Into<abi::Reg>,
    ) {
        self.emit(abi::load_u16(
            output,
            property,
            UNICODE_PROPERTY_OFFSET_INDIC_CONJUNCT_BREAK,
        ));
    }

    /// The terminal display width (0/1/2 columns) of a codepoint, read from the
    /// `charwidth` field packed into `flags` bits 4–5 (plan-70-A). The runtime
    /// computes `(flags >> 4) & 0b11`; `ambiguous_width` (bit 6) is carried in the
    /// table but not read here (policy: East-Asian Ambiguous = narrow). This is
    /// the per-scalar width primitive `term::scalarWidthRaw` and every renderer
    /// (plan-70-B..F) consume.
    pub(crate) fn emit_unicode_property_charwidth(
        &mut self,
        property: impl Into<abi::Reg>,
        output: impl Into<abi::Reg>,
    ) {
        let mask = self.temporary_vreg();
        let mask = &mask;
        let output: abi::Reg = output.into();
        self.emit(abi::load_u16(
            output,
            property,
            UNICODE_PROPERTY_OFFSET_FLAGS,
        ));
        self.emit(abi::shift_right_immediate(output, output, 4));
        self.emit(abi::move_immediate(mask, "Integer", "3"));
        self.emit(abi::and_registers(output, output, mask));
    }

    pub(crate) fn emit_unicode_u32_mapping_lookup(
        &mut self,
        codepoint: impl Into<abi::Reg>,
        entries_symbol: &str,
        entry_count: usize,
        sequences_symbol: &str,
        sequence_ptr: impl Into<abi::Reg>,
        sequence_length: impl Into<abi::Reg>,
    ) {
        let loop_label = self.label("unicode_mapping_lookup_loop");
        let move_left = self.label("unicode_mapping_lookup_left");
        let found = self.label("unicode_mapping_lookup_found");
        let not_found = self.label("unicode_mapping_lookup_not_found");
        let done = self.label("unicode_mapping_lookup_done");
        let lo = self.temporary_vreg();
        let hi = self.temporary_vreg();
        let lo = &lo;
        let hi = &hi;
        // Binary-search scratch as vregs.
        let mid_v = self.temporary_vreg();
        let offset_v = self.temporary_vreg();
        let entry_ptr_v = self.temporary_vreg();
        let field_v = self.temporary_vreg();
        let mid = &mid_v;
        let offset = &offset_v;
        let entry_ptr = &entry_ptr_v;
        let field = &field_v;
        let sequence_ptr: abi::Reg = sequence_ptr.into();
        let sequence_length: abi::Reg = sequence_length.into();

        self.emit(abi::move_immediate(lo, "Integer", "0"));
        self.emit(abi::move_immediate(hi, "Integer", &entry_count.to_string()));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(lo, hi));
        self.emit(abi::branch_ge(&not_found));
        self.emit(abi::add_registers(mid, lo, hi));
        self.emit(abi::shift_right_immediate(mid, mid, 1));
        // `mid * UNICODE_NFD_ENTRY_SIZE`, as a shift. Deriving the shift from the
        // constant rather than hard-coding `4` is what keeps the stride and the
        // record layout above from drifting apart — the spec cites the constant
        // by name (`unicode/01_tables-and-algorithms.md`), so a silent
        // disagreement here would make the spec wrong (bug-326-D2).
        const NFD_ENTRY_SHIFT: u32 = UNICODE_NFD_ENTRY_SIZE.trailing_zeros();
        const _: () = assert!(UNICODE_NFD_ENTRY_SIZE.is_power_of_two());
        self.emit(abi::shift_left_immediate(
            offset,
            mid,
            NFD_ENTRY_SHIFT as u8,
        ));
        self.emit_load_data_address(entry_ptr, entries_symbol);
        self.emit(abi::add_registers(entry_ptr, entry_ptr, offset));
        self.emit(abi::load_u32(
            field,
            entry_ptr,
            UNICODE_NFD_ENTRY_OFFSET_CODEPOINT,
        ));
        self.emit(abi::compare_registers(field, codepoint));
        self.emit(abi::branch_eq(&found));
        self.emit(abi::branch_lo(&move_left));
        self.emit(abi::move_register(hi, mid));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&move_left));
        self.emit(abi::add_immediate(lo, mid, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&found));
        self.emit(abi::load_u32(
            field,
            entry_ptr,
            UNICODE_NFD_ENTRY_OFFSET_SEQUENCE_OFFSET,
        ));
        self.emit(abi::load_u32(
            sequence_length,
            entry_ptr,
            UNICODE_NFD_ENTRY_OFFSET_SEQUENCE_LENGTH,
        ));
        self.emit(abi::shift_left_immediate(field, field, 2));
        self.emit_load_data_address(sequence_ptr, sequences_symbol);
        self.emit(abi::add_registers(sequence_ptr, sequence_ptr, field));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&not_found));
        self.emit(abi::move_immediate(sequence_length, "Integer", "0"));
        self.emit(abi::label(&done));
    }

    pub(crate) fn emit_hangul_composition_attempt(
        &mut self,
        starter: impl Into<abi::Reg>,
        current: impl Into<abi::Reg>,
        output: impl Into<abi::Reg>,
        found_label: &str,
        fallback_label: &str,
    ) {
        let check_lv_t = self.label("hangul_compose_check_lv_t");
        let base = self.temporary_vreg();
        let l_index = self.temporary_vreg();
        let base = &base;
        let l_index = &l_index;
        // Scratch as a vreg. A raw physical-register write here is invisible to
        // the register allocator, so a caller value it had placed in that
        // register is silently clobbered — a layout-sensitive miscompile.
        let v_index = self.temporary_vreg();
        let v_index = &v_index;
        let starter: abi::Reg = starter.into();
        let current: abi::Reg = current.into();
        let output: abi::Reg = output.into();

        self.emit(abi::move_immediate(base, "Integer", "4352"));
        self.emit(abi::compare_registers(starter, base));
        self.emit(abi::branch_lo(&check_lv_t));
        self.emit(abi::subtract_registers(l_index, starter, base));
        self.emit(abi::compare_immediate(l_index, "19"));
        self.emit(abi::branch_ge(&check_lv_t));

        self.emit(abi::move_immediate(base, "Integer", "4449"));
        self.emit(abi::compare_registers(current, base));
        self.emit(abi::branch_lo(&check_lv_t));
        self.emit(abi::subtract_registers(v_index, current, base));
        self.emit(abi::compare_immediate(v_index, "21"));
        self.emit(abi::branch_ge(&check_lv_t));
        self.emit(abi::move_immediate(base, "Integer", "21"));
        self.emit(abi::multiply_registers(output, l_index, base));
        self.emit(abi::add_registers(output, output, v_index));
        self.emit(abi::move_immediate(base, "Integer", "28"));
        self.emit(abi::multiply_registers(output, output, base));
        self.emit(abi::move_immediate(base, "Integer", "44032"));
        self.emit(abi::add_registers(output, output, base));
        self.emit(abi::branch(found_label));

        self.emit(abi::label(&check_lv_t));
        self.emit(abi::move_immediate(base, "Integer", "44032"));
        self.emit(abi::compare_registers(starter, base));
        self.emit(abi::branch_lo(fallback_label));
        self.emit(abi::subtract_registers(l_index, starter, base));
        self.emit(abi::move_immediate(base, "Integer", "11172"));
        self.emit(abi::compare_registers(l_index, base));
        self.emit(abi::branch_ge(fallback_label));
        self.emit(abi::move_immediate(base, "Integer", "28"));
        self.emit(abi::unsigned_divide_registers(v_index, l_index, base));
        self.emit(abi::multiply_subtract_registers(
            v_index, v_index, base, l_index,
        ));
        self.emit(abi::compare_immediate(v_index, "0"));
        self.emit(abi::branch_ne(fallback_label));
        self.emit(abi::move_immediate(base, "Integer", "4519"));
        self.emit(abi::compare_registers(current, base));
        self.emit(abi::branch_lo(fallback_label));
        self.emit(abi::subtract_registers(v_index, current, base));
        self.emit(abi::compare_immediate(v_index, "0"));
        self.emit(abi::branch_eq(fallback_label));
        self.emit(abi::compare_immediate(v_index, "28"));
        self.emit(abi::branch_ge(fallback_label));
        self.emit(abi::add_registers(output, starter, v_index));
        self.emit(abi::branch(found_label));
    }
}
