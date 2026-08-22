//! Collection-only lowerings that sit on top of the shared memory/error layer
//! (plan-96 follow-up: A1 code motion out of `src/target`).
//!
//! `lower_map_projection` (the `keys`/`values`/`toList` projection over a map)
//! and `lower_collection_reduce_impl` (the `reduce`/`reduceRight` fold) are **A1**
//! per the caller census — their only callers are collection lowerings
//! (`func_keys`/`func_values`/`func_to_list`, `func_reduce`/`func_reduce_right`,
//! and sibling collection code). Unlike the get/query/membership primitives,
//! though, they are not self-contained: they build and free arena blocks, so they
//! call *down* into the shared memory/error helpers (`emit_arena_alloc_call`,
//! `emit_checked_size_*`, `raise_error_bare`, `emit_error_code_return`,
//! `emit_collection_data_pointer_for`, and the callback loop scaffolding).
//!
//! Those helpers are genuinely shared (~25 non-collection callers: strings, math,
//! money, conversions, cleanup, assignment, …) — the "A2 / memory-common" tier
//! that will eventually move to `src/codegen/memory` (see its readme). Until then
//! these two functions reach them through the accepted temporary
//! `codegen -> target` edge, which is why this module carries the wider import
//! surface. When `memory/` lands, this file's calls become `memory::…` across a
//! real seam. They stay `impl CodeBuilder` methods, so call sites are unchanged.

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::{callable_return_type, list_element_type};
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
impl CodeBuilder<'_> {
    pub(crate) fn lower_map_projection(
        &mut self,
        collection: &ValueResult,
        element_type: &str,
        project_key: bool,
    ) -> Result<ValueResult, String> {
        // The projected list's own entry stride: zero when the element type is
        // fixed-width, so the result is built entry-free (plan-57-D).
        let proj_stride = list_entry_stride(element_type);
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let collection_slot = self.allocate_stack_object("map_projection_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let layout = CollectionTypeLayout::from_type(&format!("List OF {element_type}"))
            .ok_or_else(|| {
                format!("native code collection type 'List OF {element_type}' is not supported")
            })?;
        let data_len_slot = self.allocate_stack_object("map_projection_data_len", 8);
        let result_slot = self.allocate_stack_object("map_projection_result", 8);
        let length_loop = self.label("map_projection_length_loop");
        let length_done = self.label("map_projection_length_done");
        let alloc_ok = self.label("map_projection_alloc_ok");
        let copy_loop = self.label("map_projection_copy_loop");
        let copy_bytes = self.label("map_projection_copy_bytes");
        let copy_bytes_done = self.label("map_projection_copy_bytes_done");
        let copy_done = self.label("map_projection_copy_done");
        let offset_field = if project_key {
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET
        } else {
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET
        };
        let length_field = if project_key {
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH
        } else {
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH
        };

        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch11, "Integer", "0"));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::label(&length_loop));
        self.emit(abi::compare_registers(&scratch10, &scratch9));
        self.emit(abi::branch_ge(&length_done));
        self.emit(abi::load_u64(&scratch13, &scratch12, length_field));
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch13));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch10, &scratch10, 1));
        self.emit(abi::branch(&length_loop));
        self.emit(abi::label(&length_done));
        self.emit(abi::store_u64(
            &scratch11,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &proj_stride.to_string(),
        ));
        // Checked collection-size arithmetic (bug-147.7 / bug-232): count and
        // dataLength come from live collection headers, so route
        // count*ENTRY + HEADER + dataLen through the overflow-guarded helpers the
        // mutate path uses — a wrapped 64-bit size would under-allocate.
        let size_overflow = self.label("map_projection_size_overflow");
        self.emit_checked_size_multiply(&scratch15, &scratch9, &scratch14, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &scratch15,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch11,
            &size_overflow,
        );
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(
            crate::codegen::registry::runtime_error("ErrOutOfMemory")
                .expect("errorCode name")
                .0,
            crate::codegen::registry::runtime_error("ErrOutOfMemory")
                .expect("errorCode name")
                .1,
        )?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::move_immediate(
            &scratch13,
            "Byte",
            &layout.kind.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_KIND,
        ));
        self.emit(abi::move_immediate(
            &scratch13,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_KEY_TYPE,
        ));
        self.emit(abi::move_immediate(
            &scratch13,
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate(&scratch13, "Byte", "1"));
        self.emit(abi::store_u8(
            &scratch13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        // `arena_alloc` does not zero the block, so the bucket-index-ready byte is
        // stale poison. This result is a `List OF ...` (never consults the bucket
        // index), but leaving it unwritten is an OOB read waiting to happen if the
        // shape ever changes — zero it like the header writers do (bug-232).
        self.emit(abi::store_u8(
            abi::ZERO,
            abi::mfb_return(1),
            COLLECTION_OFFSET_BUCKETS_READY,
        ));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(
            &scratch9,
            abi::mfb_return(1),
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            abi::mfb_return(1),
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch11,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch11,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));

        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_immediate(
            &scratch17,
            abi::mfb_return(1),
            COLLECTION_HEADER_SIZE,
        ));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, "");
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &proj_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch9, &scratch14));
        self.emit(abi::add_registers(&scratch21, &scratch17, &scratch21));
        self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch11, "Integer", "0"));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(&scratch10, &scratch9));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::move_immediate(
            &scratch22,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        if proj_stride != 0 {
            self.emit(abi::store_u8(
                &scratch22,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_FLAGS,
            ));
        }
        self.emit(abi::move_immediate(&scratch22, "Integer", "0"));
        if proj_stride != 0 {
            self.emit(abi::store_u64(
                &scratch22,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
        }
        if proj_stride != 0 {
            self.emit(abi::store_u64(
                &scratch22,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
        }
        self.emit(abi::load_u64(&scratch22, &scratch12, offset_field));
        self.emit(abi::load_u64(&scratch23, &scratch12, length_field));
        if proj_stride != 0 {
            self.emit(abi::store_u64(
                &scratch11,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
        }
        if proj_stride != 0 {
            self.emit(abi::store_u64(
                &scratch23,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit(abi::add_registers(&scratch24, &scratch20, &scratch22));
        self.emit(abi::add_registers(&scratch25, &scratch21, &scratch11));
        self.emit(abi::label(&copy_bytes));
        self.emit(abi::compare_immediate(&scratch23, "0"));
        self.emit(abi::branch_eq(&copy_bytes_done));
        self.emit(abi::load_u8(&scratch22, &scratch24, 0));
        self.emit(abi::store_u8(&scratch22, &scratch25, 0));
        self.emit(abi::add_immediate(&scratch24, &scratch24, 1));
        self.emit(abi::add_immediate(&scratch25, &scratch25, 1));
        self.emit(abi::subtract_immediate(&scratch23, &scratch23, 1));
        self.emit(abi::branch(&copy_bytes));
        self.emit(abi::label(&copy_bytes_done));
        if proj_stride != 0 {
            // Reload the length from the entry just written.
            self.emit(abi::load_u64(
                &scratch23,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        } else {
            // kind 2 wrote no entry to read back, so re-derive the length from
            // the SOURCE entry — the same value the copy loop consumed.
            self.emit(abi::load_u64(&scratch23, &scratch12, length_field));
        }
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch23));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch17, &scratch17, proj_stride));
        self.emit(abi::add_immediate(&scratch10, &scratch10, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: format!("List OF {element_type}"),
            location: Operand::from(result.render()),
            text: if project_key {
                format!("keys({})", collection.type_)
            } else {
                format!("values({})", collection.type_)
            },
        })
    }

    pub(crate) fn lower_collection_reduce_impl(
        &mut self,
        args: &[NirValue],
        reverse: bool,
    ) -> Result<ValueResult, String> {
        let scratch9 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection reduce does not accept {}",
                collection.type_
            ));
        };
        let collection_slot = self.allocate_stack_object("reduce_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let initial = self.lower_value(&args[1])?;
        let accumulator_slot = self.allocate_stack_object("reduce_accumulator", 8);
        self.emit(abi::store_u64(
            &initial.location,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        let action = self.lower_value(&args[2])?;
        let output_type = callable_return_type(&action.type_).ok_or_else(|| {
            format!(
                "native collection reduce reducer must be a function, got {}",
                action.type_
            )
        })?;
        if output_type != initial.type_ {
            return Err(format!(
                "native collection reduce reducer must return {}, got {output_type}",
                initial.type_
            ));
        }
        self.require_direct_callable(if reverse { "reduceRight" } else { "reduce" }, &action)?;
        let action_slot = self.allocate_stack_object("reduce_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let cursor_slot = self.allocate_stack_object("reduce_cursor", 8);
        let remaining_slot = self.allocate_stack_object("reduce_remaining", 8);
        if reverse {
            self.initialize_collection_loop_slots_reverse(
                collection_slot,
                cursor_slot,
                remaining_slot,
                &element_type,
            );
        } else {
            self.initialize_collection_loop_slots(
                collection_slot,
                cursor_slot,
                remaining_slot,
                &element_type,
            );
        }

        // plan-86-B (B1): reclaim the superseded loop item and accumulator each
        // iteration instead of leaking them (the pre-fix native `reduce` never
        // freed either, so a `List OF String` fold over N passes grew arena RSS by
        // ~one block per element per pass — the 3.5× penalty that made native
        // `reduce` slower than interpreted `reduceRight` doing the same fold).
        //
        // The bug-307 hazard the leak avoided is real but detectable at runtime:
        // the reducer may adopt the item (`FUNC(acc, x) RETURN x`) or return the
        // accumulator unchanged/in-place, in which case the block is still live.
        // MFBASIC value semantics guarantees a returned String never *partially*
        // aliases an input (a slice/concat produces a fresh owned block), so a
        // pointer-equality check against the item and the old accumulator is a
        // sufficient and exact aliasing test. We free only blocks we own and that
        // the reducer did not carry forward.
        //
        // `acc_owned` tracks whether the current accumulator is a block this loop
        // is responsible for freeing. The seed starts `owned = 0`: its ownership
        // stays with the caller (value semantics), exactly as before this fix, so
        // it is never freed here and the returned result is never double-freed.
        // This machinery is emitted only when a String block is actually at risk
        // of leaking (String accumulator and/or String element); scalar folds keep
        // their prior byte-identical codegen.
        let manages_owned = initial.type_ == "String" || element_type == "String";
        let (item_slot, acc_owned_slot, new_slot, new_owned_slot) = if manages_owned {
            let item_slot = self.allocate_stack_object("reduce_item", 8);
            let acc_owned_slot = self.allocate_stack_object("reduce_acc_owned", 8);
            let new_slot = self.allocate_stack_object("reduce_new", 8);
            let new_owned_slot = self.allocate_stack_object("reduce_new_owned", 8);
            let zero = self.temporary_vreg();
            self.emit(abi::move_immediate(&zero, "Integer", "0"));
            self.emit(abi::store_u64(&zero, abi::stack_pointer(), acc_owned_slot));
            (
                Some(item_slot),
                Some(acc_owned_slot),
                Some(new_slot),
                Some(new_owned_slot),
            )
        } else {
            (None, None, None, None)
        };

        let loop_label = self.label("reduce_call_loop");
        let ok_label = self.label("reduce_call_ok");
        let done = self.label("reduce_call_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, &element_type)?;
        if let Some(item_slot) = item_slot {
            // Spill the (freshly materialized, owned) item so it survives the
            // reducer call's register clobber and can be freed/compared after.
            self.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        }
        self.emit(abi::load_u64(
            &abi::argument_register(0)?,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        self.emit(abi::move_register(&abi::argument_register(1)?, &item));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&scratch17);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        // Failure path (bug-307 / plan-26-B): pass `None` (no cleanup). The
        // success path below reclaims the superseded item and accumulator via
        // runtime aliasing checks, but on a *failing* iteration the current item
        // and accumulator may still alias the reducer's live inputs or the
        // non-owned seed, and the ownership bookkeeping has not yet been committed
        // for this iteration — so freeing here could double-free or free a
        // borrowed block after the handler recovers. Leaking the at-most-one
        // in-flight item/accumulator on the rare error path is the safe choice;
        // the hot success path is where the reclamation matters (plan-86-B).
        self.emit_callback_failure_exit(None)?;
        self.emit(abi::label(&ok_label));
        if manages_owned {
            // Slots are all `Some` under `manages_owned`.
            let item_slot = item_slot.unwrap();
            let acc_owned_slot = acc_owned_slot.unwrap();
            let new_slot = new_slot.unwrap();
            let new_owned_slot = new_owned_slot.unwrap();

            // Spill the reducer output; the frees below clobber every caller-saved
            // register (the `arena_free` call), so `new` must live in a slot.
            self.emit(abi::store_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                new_slot,
            ));

            // new_owned = (new == old_acc) ? old_owned : 1.
            //   - fresh result  → we own it (free it next iteration / not at all if final)
            //   - returned acc unchanged/in-place → inherit the old ownership
            //   - adopted item  → new != old_acc, so owned = 1 (the item block is owned)
            let r_owned = self.temporary_vreg();
            let r_new = self.temporary_vreg();
            let r_acc = self.temporary_vreg();
            let owned_done = self.label("reduce_new_owned_done");
            self.emit(abi::move_immediate(&r_owned, "Integer", "1"));
            self.emit(abi::load_u64(&r_new, abi::stack_pointer(), new_slot));
            self.emit(abi::load_u64(
                &r_acc,
                abi::stack_pointer(),
                accumulator_slot,
            ));
            self.emit(abi::compare_registers(&r_new, &r_acc));
            self.emit(abi::branch_ne(&owned_done));
            self.emit(abi::load_u64(
                &r_owned,
                abi::stack_pointer(),
                acc_owned_slot,
            ));
            self.emit(abi::label(&owned_done));
            self.emit(abi::store_u64(
                &r_owned,
                abi::stack_pointer(),
                new_owned_slot,
            ));

            // Free the loop item unless the reducer adopted it as the new
            // accumulator (item == new). Only String items own a standalone block;
            // fixed-width items materialize nothing.
            if element_type == "String" {
                let r_item = self.temporary_vreg();
                let r_new2 = self.temporary_vreg();
                let item_kept = self.label("reduce_item_kept");
                self.emit(abi::load_u64(&r_item, abi::stack_pointer(), item_slot));
                self.emit(abi::load_u64(&r_new2, abi::stack_pointer(), new_slot));
                self.emit(abi::compare_registers(&r_item, &r_new2));
                self.emit(abi::branch_eq(&item_kept));
                self.free_collection_loop_item(item_slot, "String")?;
                self.emit(abi::label(&item_kept));
            }

            // Free the superseded accumulator when this loop owns it (owned != 0)
            // and the reducer produced a distinct result (old_acc != new). This
            // frees a String block from a stack slot — the same operation
            // `free_collection_loop_item` performs, applied to the accumulator.
            if initial.type_ == "String" {
                let r_o = self.temporary_vreg();
                let r_a = self.temporary_vreg();
                let r_n = self.temporary_vreg();
                let acc_kept = self.label("reduce_acc_kept");
                self.emit(abi::load_u64(&r_o, abi::stack_pointer(), acc_owned_slot));
                self.emit(abi::compare_immediate(&r_o, "0"));
                self.emit(abi::branch_eq(&acc_kept));
                self.emit(abi::load_u64(&r_a, abi::stack_pointer(), accumulator_slot));
                self.emit(abi::load_u64(&r_n, abi::stack_pointer(), new_slot));
                self.emit(abi::compare_registers(&r_a, &r_n));
                self.emit(abi::branch_eq(&acc_kept));
                self.free_collection_loop_item(accumulator_slot, "String")?;
                self.emit(abi::label(&acc_kept));
            }

            // Commit the new accumulator and its ownership for the next iteration.
            let r_commit = self.temporary_vreg();
            self.emit(abi::load_u64(&r_commit, abi::stack_pointer(), new_slot));
            self.emit(abi::store_u64(
                &r_commit,
                abi::stack_pointer(),
                accumulator_slot,
            ));
            let r_commit_owned = self.temporary_vreg();
            self.emit(abi::load_u64(
                &r_commit_owned,
                abi::stack_pointer(),
                new_owned_slot,
            ));
            self.emit(abi::store_u64(
                &r_commit_owned,
                abi::stack_pointer(),
                acc_owned_slot,
            ));
        } else {
            self.emit(abi::store_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                accumulator_slot,
            ));
        }
        if reverse {
            self.advance_collection_loop_reverse(
                cursor_slot,
                remaining_slot,
                &loop_label,
                &element_type,
            );
        } else {
            self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label, &element_type);
        }
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(
            &result,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        Ok(ValueResult {
            type_: initial.type_,
            location: Operand::from(result.render()),
            text: format!(
                "{}({}, {}, {})",
                if reverse { "reduceRight" } else { "reduce" },
                collection.type_,
                initial.text,
                action.text
            ),
        })
    }
}
