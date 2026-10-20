// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::typed_list_element_type;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

mod mid;
pub(crate) use mid::MidStringRegs;

impl CodeBuilder<'_> {
    /// plan-77 U4: fast-forward a scalar-walk loop over runs of ASCII bytes.
    /// While at least 8 more scalars are needed to reach `target`, at least 8
    /// bytes remain, and the next 8 bytes are all ASCII (`b & 0x80 == 0`, so each
    /// is its own 1-byte scalar and a boundary), advance `cursor`/`remaining` by 8
    /// and `scalar_index` by 8 in one step instead of the per-byte continuation
    /// walk. All-ASCII is the only case handled (its correctness is trivial: every
    /// byte is a scalar start); any non-ASCII byte falls through to the caller's
    /// existing byte-accurate loop, so multibyte input is unaffected. The `< 8`
    /// guard means the walk never overshoots `target`. Emitted just before the
    /// per-scalar loop it accelerates; the three scratch registers must be free.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_ascii_scalar_fastforward(
        &mut self,
        cursor: impl Into<abi::Reg>,
        remaining: impl Into<abi::Reg>,
        scalar_index: impl Into<abi::Reg>,
        target: impl Into<abi::Reg>,
        diff: impl Into<abi::Reg>,
        word: impl Into<abi::Reg>,
        ascii_mask: impl Into<abi::Reg>,
    ) {
        let cursor = cursor.into();
        let remaining = remaining.into();
        let scalar_index = scalar_index.into();
        let diff = diff.into();
        let word = word.into();
        let ascii_mask = ascii_mask.into();
        let block = self.label("ascii_ff_block");
        let done = self.label("ascii_ff_done");
        // 0x8080808080808080 — the high bit of every byte.
        self.emit(abi::move_immediate(
            ascii_mask,
            "Integer",
            "9259542123273814144",
        ));
        self.emit(abi::label(&block));
        self.emit(abi::subtract_registers(diff, target, scalar_index));
        self.emit(abi::compare_immediate(diff, "8"));
        self.emit(abi::branch_lo(&done));
        self.emit(abi::compare_immediate(remaining, "8"));
        self.emit(abi::branch_lo(&done));
        self.emit(abi::load_u64(word, cursor, 0));
        self.emit(abi::and_registers(word, word, ascii_mask));
        self.emit(abi::compare_immediate(word, "0"));
        self.emit(abi::branch_ne(&done));
        self.emit(abi::add_immediate(cursor, cursor, 8));
        self.emit(abi::subtract_immediate(remaining, remaining, 8));
        self.emit(abi::add_immediate(scalar_index, scalar_index, 8));
        self.emit(abi::branch(&block));
        self.emit(abi::label(&done));
    }

    pub(crate) fn lower_find(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let haystack = self.lower_value(&args[0])?;
        if let Some(element_type) = typed_list_element_type(&haystack.type_).cloned() {
            let haystack_slot = self.allocate_stack_object("find_list_haystack", 8);
            self.emit(abi::store_u64(
                &haystack.location,
                abi::stack_pointer(),
                haystack_slot,
            ));
            let needle = self.lower_value(&args[1])?;
            let needle_slot = self.allocate_stack_object("find_list_needle", 8);
            // `d`-native float needle stores via `str d` (plan-01 float-dnative).
            self.store_value_at(&needle, abi::stack_pointer(), needle_slot);
            let start_slot = self.allocate_stack_object("find_list_start", 8);
            if let Some(start) = args.get(2) {
                let start = self.lower_value(start)?;
                if start.type_ != ParameterType::Integer {
                    return Err(format!(
                        "native list find start must be Integer, got {}",
                        start.type_
                    ));
                }
                self.emit(abi::store_u64(
                    &start.location,
                    abi::stack_pointer(),
                    start_slot,
                ));
            } else {
                self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
                self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), start_slot));
            }

            if needle.type_ == element_type {
                return self.lower_list_find_item(
                    haystack_slot,
                    needle_slot,
                    start_slot,
                    &haystack.type_,
                    &element_type,
                );
            }
            if needle.type_ == haystack.type_ {
                return self.lower_list_find_sublist(
                    haystack_slot,
                    needle_slot,
                    start_slot,
                    &haystack.type_,
                    &element_type,
                );
            }
            return Err(format!(
                "native list find needle must be {} or {}, got {}",
                element_type, haystack.type_, needle.type_
            ));
        }
        if haystack.type_ != ParameterType::String {
            return Err(format!(
                "native string find haystack must be String, got {}",
                haystack.type_
            ));
        }
        let haystack_slot = self.allocate_stack_object("find_haystack", 8);
        self.emit(abi::store_u64(
            &haystack.location,
            abi::stack_pointer(),
            haystack_slot,
        ));

        let needle = self.lower_value(&args[1])?;
        if needle.type_ != ParameterType::String {
            return Err(format!(
                "native string find needle must be String, got {}",
                needle.type_
            ));
        }
        let needle_slot = self.allocate_stack_object("find_needle", 8);
        self.emit(abi::store_u64(
            &needle.location,
            abi::stack_pointer(),
            needle_slot,
        ));

        let start_slot = self.allocate_stack_object("find_start", 8);
        if let Some(start) = args.get(2) {
            let start = self.lower_value(start)?;
            if start.type_ != ParameterType::Integer {
                return Err(format!(
                    "native string find start must be Integer, got {}",
                    start.type_
                ));
            }
            self.emit(abi::store_u64(
                &start.location,
                abi::stack_pointer(),
                start_slot,
            ));
        } else {
            self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
            self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), start_slot));
        }

        let result_slot = self.allocate_stack_object("find_result", 8);
        let haystack_ptr = &scratch8;
        let needle_ptr = &scratch9;
        let haystack_len = &scratch10;
        let needle_len = &scratch11;
        let start = &scratch12;
        let scalar_index = &scratch13;
        let cursor = &scratch14;
        let remaining = &scratch15;
        let byte = &scratch16;
        let mask = &scratch17;
        let candidate = &scratch20;
        let compare_remaining = &scratch21;
        let needle_cursor = &scratch22;
        let haystack_byte = &scratch23;
        let needle_byte = &scratch24;

        let locate_start = self.label("find_locate_start");
        let locate_continue = self.label("find_locate_continue");
        let locate_advanced = self.label("find_locate_advanced");
        let start_ready = self.label("find_start_ready");
        let search_loop = self.label("find_search_loop");
        let compare_loop = self.label("find_compare_loop");
        let advance_candidate = self.label("find_advance_candidate");
        let skip_continuation = self.label("find_skip_continuation");
        let candidate_ready = self.label("find_candidate_ready");
        let found = self.label("find_found");
        let invalid_start = self.label("find_invalid_start");
        let not_found = self.label("find_not_found");

        self.emit(abi::load_u64(
            haystack_ptr,
            abi::stack_pointer(),
            haystack_slot,
        ));
        self.emit(abi::load_u64(needle_ptr, abi::stack_pointer(), needle_slot));
        self.emit(abi::load_u64(haystack_len, haystack_ptr, 0));
        self.emit(abi::load_u64(needle_len, needle_ptr, 0));
        self.emit(abi::load_u64(start, abi::stack_pointer(), start_slot));
        // Negative start: raise the documented 77050001 immediately instead of
        // relying on loop exhaustion after a full O(n) walk (audit-unicode #6);
        // matches the explicit checks in list-find and mid.
        self.emit(abi::compare_immediate(start, "0"));
        self.emit(abi::branch_lt(&invalid_start));
        self.emit(abi::move_immediate(scalar_index, "Integer", "0"));
        self.emit(abi::add_immediate(cursor, haystack_ptr, 8));
        self.emit(abi::move_register(remaining, haystack_len));
        self.emit(abi::move_immediate(
            mask,
            "Integer",
            crate::codegen::string::unicode_props::UTF8_CONTINUATION_MASK,
        ));

        // Walk `start` characters forward. Each character is one lead/ASCII byte
        // followed by its continuation bytes; consume the whole character before
        // re-checking `scalar_index == start` so the cursor always lands on a
        // character boundary. The earlier form re-checked equality after seeing a
        // lead byte but before skipping its continuations, so when character
        // `start-1` was multibyte the cursor stopped on a continuation byte and
        // every returned index was inflated by one (bug-133). Mirrors `lower_mid`.
        // plan-77 U4: skip whole 8-byte ASCII runs before the per-scalar walk.
        let ff_diff = self.temporary_vreg();
        let ff_word = self.temporary_vreg();
        let ff_mask = self.temporary_vreg();
        self.emit_ascii_scalar_fastforward(
            cursor,
            remaining,
            scalar_index,
            start,
            &ff_diff,
            &ff_word,
            &ff_mask,
        );
        self.emit(abi::label(&locate_start));
        self.emit(abi::compare_registers(scalar_index, start));
        self.emit(abi::branch_eq(&start_ready));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&invalid_start));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit_scalar_skip_continuations(
            cursor,
            remaining,
            byte,
            mask,
            &locate_continue,
            &locate_advanced,
        );
        self.emit(abi::label(&locate_advanced));
        self.emit(abi::add_immediate(scalar_index, scalar_index, 1));
        self.emit(abi::branch(&locate_start));

        self.emit(abi::label(&start_ready));
        self.emit(abi::compare_immediate(needle_len, "0"));
        self.emit(abi::branch_eq(&found));

        self.emit(abi::label(&search_loop));
        self.emit(abi::compare_registers(remaining, needle_len));
        self.emit(abi::branch_lo(&not_found));
        self.emit(abi::move_register(candidate, cursor));
        self.emit(abi::add_immediate(needle_cursor, needle_ptr, 8));
        self.emit(abi::move_register(compare_remaining, needle_len));

        self.emit(abi::label(&compare_loop));
        self.emit(abi::compare_immediate(compare_remaining, "0"));
        self.emit(abi::branch_eq(&found));
        self.emit(abi::load_u8(haystack_byte, candidate, 0));
        self.emit(abi::load_u8(needle_byte, needle_cursor, 0));
        self.emit(abi::compare_registers(haystack_byte, needle_byte));
        self.emit(abi::branch_ne(&advance_candidate));
        self.emit(abi::add_immediate(candidate, candidate, 1));
        self.emit(abi::add_immediate(needle_cursor, needle_cursor, 1));
        self.emit(abi::subtract_immediate(
            compare_remaining,
            compare_remaining,
            1,
        ));
        self.emit(abi::branch(&compare_loop));

        self.emit(abi::label(&advance_candidate));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit_scalar_skip_continuations(
            cursor,
            remaining,
            byte,
            mask,
            &skip_continuation,
            &candidate_ready,
        );
        self.emit(abi::label(&candidate_ready));
        self.emit(abi::add_immediate(scalar_index, scalar_index, 1));
        self.emit(abi::branch(&search_loop));

        self.emit(abi::label(&invalid_start));
        self.raise_error("strings.find", "ErrIndexOutOfRange")?;
        self.emit(abi::label(&not_found));
        self.raise_error("strings.find", "ErrNotFound")?;
        self.emit(abi::label(&found));
        self.emit(abi::store_u64(
            scalar_index,
            abi::stack_pointer(),
            result_slot,
        ));
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));

        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Integer,
            location: Operand::from(result),
            text: "find(String, String)".to_string(),
        })
    }

    pub(crate) fn lower_list_find_item(
        &mut self,
        haystack_slot: usize,
        needle_slot: usize,
        start_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let result_slot = self.allocate_stack_object("find_list_result", 8);
        let valid_start = self.label("list_find_item_valid_start");
        let loop_label = self.label("list_find_item_loop");
        let found = self.label("list_find_item_found");
        let next = self.label("list_find_item_next");
        let invalid_start = self.label("list_find_item_invalid_start");
        let not_found = self.label("list_find_item_not_found");
        let done = self.label("list_find_item_done");

        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            haystack_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), needle_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), start_slot));
        self.emit(abi::compare_immediate(&scratch11, "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid_start));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers(&scratch11, &scratch10));
        self.emit(abi::branch_gt(&invalid_start));
        // kind 2: scratch15 is a byte OFFSET into the data region, seeded at
        // `start * payload`, and the span is derivable from it (plan-57-D).
        let find_payload = kind2_payload_size(element_type);
        self.emit(abi::move_register(&scratch12, &scratch11));
        self.emit(abi::move_immediate(
            &scratch13,
            "Integer",
            &find_payload.unwrap_or(COLLECTION_ENTRY_SIZE).to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch14, &scratch12, &scratch13));
        if find_payload.is_some() {
            self.emit(abi::move_register(&scratch15, &scratch14));
        } else {
            self.emit(abi::add_immediate(
                &scratch15,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::add_registers(&scratch15, &scratch15, &scratch14));
        }

        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(&scratch12, &scratch10));
        self.emit(abi::branch_ge(&not_found));
        if let Some(payload) = find_payload {
            self.emit(abi::move_register(&scratch16, &scratch15));
            self.emit(abi::move_immediate(
                &scratch17,
                "Integer",
                &payload.to_string(),
            ));
        } else {
            self.emit(abi::load_u64(
                &scratch16,
                &scratch15,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &scratch17,
                &scratch15,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit_collection_payload_matches_value_branch(
            element_type,
            element_type,
            &scratch8,
            &scratch16,
            &scratch17,
            &scratch9,
            &found,
            &next,
        )?;

        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(
            &scratch15,
            &scratch15,
            find_payload.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&invalid_start));
        self.raise_error("collections.find", "ErrIndexOutOfRange")?;
        self.emit(abi::label(&not_found));
        self.raise_error("collections.find", "ErrNotFound")?;
        self.emit(abi::label(&found));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&done));

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Integer,
            location: Operand::from(result),
            text: format!("find({list_type}, {element_type})"),
        })
    }

    pub(crate) fn lower_list_find_sublist(
        &mut self,
        haystack_slot: usize,
        needle_slot: usize,
        start_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        let sub_payload = kind2_payload_size(element_type);
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();

        let result_slot = self.allocate_stack_object("find_sublist_result", 8);
        let valid_start = self.label("list_find_sublist_valid_start");
        let empty_found = self.label("list_find_sublist_empty_found");
        let outer_loop = self.label("list_find_sublist_outer_loop");
        let compare_loop = self.label("list_find_sublist_compare_loop");
        let compare_next = self.label("list_find_sublist_compare_next");
        let found = self.label("list_find_sublist_found");
        let advance_outer = self.label("list_find_sublist_advance_outer");
        let invalid_start = self.label("list_find_sublist_invalid_start");
        let not_found = self.label("list_find_sublist_not_found");
        let done = self.label("list_find_sublist_done");

        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            haystack_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), needle_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), start_slot));
        self.emit(abi::compare_immediate(&scratch12, "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid_start));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers(&scratch12, &scratch10));
        self.emit(abi::branch_gt(&invalid_start));
        self.emit(abi::compare_immediate(&scratch11, "0"));
        self.emit(abi::branch_eq(&empty_found));

        self.emit(abi::move_register(&scratch13, &scratch12));
        self.emit(abi::label(&outer_loop));
        self.emit(abi::add_registers(&scratch14, &scratch13, &scratch11));
        self.emit(abi::compare_registers(&scratch14, &scratch10));
        self.emit(abi::branch_gt(&not_found));
        self.emit(abi::move_immediate(&scratch14, "Integer", "0"));

        self.emit(abi::label(&compare_loop));
        self.emit(abi::compare_registers(&scratch14, &scratch11));
        self.emit(abi::branch_eq(&found));
        self.emit(abi::add_registers(&scratch15, &scratch13, &scratch14));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &sub_payload.unwrap_or(COLLECTION_ENTRY_SIZE).to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch15, &scratch15, &scratch16));
        if let Some(payload) = sub_payload {
            // kind 2: the haystack span is `(i + j) * payload` and the needle's
            // is `j * payload`, each with a constant length — no entries on
            // either side (plan-57-D).
            self.emit(abi::move_register(&scratch21, &scratch15));
            self.emit(abi::move_immediate(
                &scratch22,
                "Integer",
                &payload.to_string(),
            ));
            self.emit(abi::multiply_registers(&scratch23, &scratch14, &scratch16));
            self.emit(abi::move_immediate(
                &scratch24,
                "Integer",
                &payload.to_string(),
            ));
        } else {
            self.emit(abi::add_immediate(
                &scratch17,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::add_registers(&scratch17, &scratch17, &scratch15));
            self.emit(abi::multiply_registers(&scratch20, &scratch14, &scratch16));
            self.emit(abi::add_immediate(
                &scratch25,
                &scratch9,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::add_registers(&scratch20, &scratch25, &scratch20));
            self.emit(abi::load_u64(
                &scratch21,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &scratch22,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::load_u64(
                &scratch23,
                &scratch20,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &scratch24,
                &scratch20,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit_collection_payloads_match_branch(
            element_type,
            element_type,
            &scratch8,
            &scratch21,
            &scratch22,
            &scratch9,
            &scratch23,
            &scratch24,
            &compare_next,
            &advance_outer,
        )?;

        self.emit(abi::label(&compare_next));
        self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        self.emit(abi::branch(&compare_loop));

        self.emit(abi::label(&advance_outer));
        self.emit(abi::add_immediate(&scratch13, &scratch13, 1));
        self.emit(abi::branch(&outer_loop));

        self.emit(abi::label(&empty_found));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid_start));
        self.raise_error("collections.find", "ErrIndexOutOfRange")?;
        self.emit(abi::label(&not_found));
        self.raise_error("collections.find", "ErrNotFound")?;
        self.emit(abi::label(&found));
        self.emit(abi::store_u64(
            &scratch13,
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::label(&done));

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Integer,
            location: Operand::from(result),
            text: format!("find({list_type}, {list_type}) over {element_type}"),
        })
    }
}
