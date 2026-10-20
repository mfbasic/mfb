//! plan-142-D: in-place arms for the `Set` algebra and the two `Map` self-updates
//! — `union`, `intersection`, `difference`, `symmetricDifference`, `merge`,
//! `mapValues`.
//!
//! Each reproduces its copying body (`__collections_union` & co.) operation for
//! operation, so the result — including its iteration order, which is insertion
//! order — is the copying result:
//!
//! * `union(s, t)`: `s`, then each element of `t` not already present, in `t`'s
//!   order — `add` in place per element of `t`.
//! * `intersection(s, t)`: the elements of `s` that are in `t`, in `s`'s order —
//!   one membership pass writing a mark per entry, then one
//!   `lower_map_compact_in_place` (every element of `s` is tested).
//! * `difference(s, t)`: the elements of `s` not in `t`, in `s`'s order — each
//!   element of `t` is looked up in `s` and a hit claimed for removal, then one
//!   compaction from the first claimed entry (bug-732: the call costs `t` and the
//!   entries that move, not all of `s`).
//! * `symmetricDifference(s, t)`: `s`'s elements not in `t`, then `t`'s not in `s` —
//!   the same claim pass over `t`, marking each miss, then the compaction, then
//!   the marked adds.
//! * `merge(m, n, preferB)`: for each entry of `n`, `set` it when `preferB` or its
//!   key is absent.
//! * `mapValues(m, f)` (`f` returning `m`'s value type): `f` for every value first
//!   (results in the self-update scratch), then each written back into its own
//!   entry (`emit_map_entry_value_write` — no key is read).
//!
//! A `String` key or value read from a map is rebuilt in the self-update scratch
//! (`emit_map_entry_borrowed`) rather than allocated, so the arms allocate nothing
//! per element.
//!
//! Every batch of inserts is preceded by one `emit_map_reserve`, and every callback
//! runs before the first write, so a failure — a callback's error, or running out
//! of memory — leaves the binding as it was. A self-alias (`union(s, s)`,
//! `merge(m, m, p)`) is the identity (or, for `difference`/`symmetricDifference`,
//! the empty set) and is lowered as such, after evaluating any other operand.

use crate::codegen::collection::assign::inplace_dest::*;
use crate::codegen::collection::assign::self_update::SelfUpdateSite;
use crate::codegen::collection::map::map_mutate::InlineGrow;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::engine::types::{
    typed_callable_return_type, typed_map_type_parts, typed_set_element_type,
};
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

mod map_ops;
mod set_ops;

/// Which payload of a map entry.
#[derive(Clone, Copy)]
enum Payload {
    Key,
    Value,
}

impl CodeBuilder<'_> {
    /// Load the key or value of entry `index_slot` of the map in `map_slot` into a
    /// fresh slot (a `String` payload is materialized — free it with
    /// `free_collection_loop_item`).
    fn emit_map_entry_payload(
        &mut self,
        map_slot: usize,
        index_slot: usize,
        which: Payload,
        type_: &ParameterType,
        name: &str,
    ) -> Result<usize, String> {
        let (offset_field, length_field) = match which {
            Payload::Key => (
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ),
            Payload::Value => (
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ),
        };
        let base = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let offset = self.temporary_vreg();
        let length = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&entry, abi::stack_pointer(), index_slot));
        self.emit(abi::move_immediate(
            &offset,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&entry, &entry, &offset));
        self.emit(abi::add_registers(&entry, &entry, &base));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&offset, &entry, offset_field));
        self.emit(abi::load_u64(&length, &entry, length_field));
        let value = self.emit_load_map_payload(type_, &base, &offset, &length)?;
        let slot = self.allocate_stack_object(name, 8);
        self.emit(abi::store_u64(&value, abi::stack_pointer(), slot));
        Ok(slot)
    }

    /// The longest key or value payload among the entries of the map in
    /// `map_slot`, into a fresh slot.
    fn emit_map_max_payload_len(&mut self, map_slot: usize, which: Payload) -> usize {
        let field = match which {
            Payload::Key => COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            Payload::Value => COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        };
        let max_slot = self.allocate_stack_object("inplace_max_payload", 8);
        let base = self.temporary_vreg();
        let count = self.temporary_vreg();
        let index = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let len = self.temporary_vreg();
        let max = self.temporary_vreg();
        let top = self.label("inplace_max_payload_loop");
        let keep = self.label("inplace_max_payload_keep");
        let done = self.label("inplace_max_payload_done");
        self.emit(abi::load_u64(&base, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(&entry, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&max, "Integer", "0"));
        self.emit(abi::label(&top));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&done));
        self.emit(abi::load_u64(&len, &entry, field));
        self.emit(abi::compare_registers(&len, &max));
        self.emit(abi::branch_ls(&keep));
        self.emit(abi::move_register(&max, &len));
        self.emit(abi::label(&keep));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&top));
        self.emit(abi::label(&done));
        self.emit(abi::store_u64(&max, abi::stack_pointer(), max_slot));
        max_slot
    }

    /// Reserve the self-update scratch as consecutive areas of the byte counts in
    /// `sizes` (each rounded up to 8), returning a slot per area holding its
    /// address. A `String` area is sized `maxLength + 9` by the caller.
    fn emit_scratch_areas(&mut self, sizes: &[usize]) -> Result<Vec<usize>, String> {
        let need_slot = self.allocate_stack_object("inplace_areas_need", 8);
        let mask = self.temporary_vreg();
        let total = self.temporary_vreg();
        self.emit(abi::move_immediate(&mask, "Integer", &(!7u64).to_string()));
        self.emit(abi::move_immediate(&total, "Integer", "0"));
        for &size in sizes {
            let part = self.temporary_vreg();
            self.emit(abi::load_u64(&part, abi::stack_pointer(), size));
            self.emit(abi::add_immediate(&part, &part, 7));
            self.emit(abi::and_registers(&part, &part, &mask));
            self.emit(abi::add_registers(&total, &total, &part));
        }
        self.emit(abi::store_u64(&total, abi::stack_pointer(), need_slot));
        let data_slot = self.emit_reserve_self_update_scratch(need_slot)?;
        let mut areas = Vec::with_capacity(sizes.len());
        let cursor = self.temporary_vreg();
        let mask = self.temporary_vreg();
        self.emit(abi::load_u64(&cursor, abi::stack_pointer(), data_slot));
        self.emit(abi::move_immediate(&mask, "Integer", &(!7u64).to_string()));
        for &size in sizes {
            let slot = self.allocate_stack_object("inplace_area", 8);
            self.emit(abi::store_u64(&cursor, abi::stack_pointer(), slot));
            let part = self.temporary_vreg();
            self.emit(abi::load_u64(&part, abi::stack_pointer(), size));
            self.emit(abi::add_immediate(&part, &part, 7));
            self.emit(abi::and_registers(&part, &part, &mask));
            self.emit(abi::add_registers(&cursor, &cursor, &part));
            areas.push(slot);
        }
        Ok(areas)
    }

    /// A slot holding `length + 9` for the length in `length_slot` — the size of a
    /// borrowed `String` area.
    fn emit_string_area_size(&mut self, length_slot: usize) -> usize {
        let slot = self.allocate_stack_object("inplace_string_area", 8);
        let len = self.temporary_vreg();
        self.emit(abi::load_u64(&len, abi::stack_pointer(), length_slot));
        self.emit(abi::add_immediate(&len, &len, 9));
        self.emit(abi::store_u64(&len, abi::stack_pointer(), slot));
        slot
    }

    /// The key or value of entry `index_slot` of the map in `map_slot`, as a value
    /// in a fresh slot that nothing needs to free: a `String` payload is rebuilt as
    /// `[length][bytes][NUL]` in the scratch area at the address in `area_slot`
    /// (reused per entry — the value is dead once the next entry is read); any
    /// other payload is the load's own alias into the map.
    fn emit_map_entry_borrowed(
        &mut self,
        map_slot: usize,
        index_slot: usize,
        which: Payload,
        type_: &ParameterType,
        area_slot: Option<usize>,
        name: &str,
    ) -> Result<usize, String> {
        if *type_ != ParameterType::String {
            return self.emit_map_entry_payload(map_slot, index_slot, which, type_, name);
        }
        let area_slot =
            area_slot.ok_or("native in-place map loop: a String payload needs an area")?;
        let (offset_field, length_field) = match which {
            Payload::Key => (
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ),
            Payload::Value => (
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ),
        };
        let base = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let src = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let len = self.temporary_vreg();
        let copy = self.temporary_vreg();
        let zero = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&entry, abi::stack_pointer(), index_slot));
        self.emit(abi::move_immediate(
            &len,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&entry, &entry, &len));
        self.emit(abi::add_registers(&entry, &entry, &base));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer_for(&src, &base, &ParameterType::named(""));
        self.emit(abi::load_u64(&len, &entry, offset_field));
        self.emit(abi::add_registers(&src, &src, &len));
        self.emit(abi::load_u64(&len, &entry, length_field));
        self.emit(abi::load_u64(&dst, abi::stack_pointer(), area_slot));
        self.emit(abi::store_u64(&len, &dst, 0));
        self.emit(abi::add_immediate(&dst, &dst, 8));
        self.emit_block_copy_advance(&dst, &src, &len, &copy, "inplace_borrow_str");
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::store_u8(&zero, &dst, 0));
        let slot = self.allocate_stack_object(name, 8);
        let area = self.temporary_vreg();
        self.emit(abi::load_u64(&area, abi::stack_pointer(), area_slot));
        self.emit(abi::store_u64(&area, abi::stack_pointer(), slot));
        Ok(slot)
    }

    /// Write the value in `value_slot` as the value of entry `index_slot` of the
    /// map in `map_slot`, in place: over the old payload when it fits, else at the
    /// aligned data tail (room reserved by the caller), repointing the entry. The
    /// key and the entry's position — the iteration order — are unchanged.
    fn emit_map_entry_value_write(
        &mut self,
        map_slot: usize,
        index_slot: usize,
        value_slot: usize,
        value_type: &ParameterType,
    ) -> Result<(), String> {
        let payload = PayloadSlot {
            slot: value_slot,
            type_: value_type.clone(),
        };
        let need_slot = self.emit_payload_length_to_stack(&payload, "inplace_mvwrite_need")?;
        let offset_slot = self.allocate_stack_object("inplace_mvwrite_off", 8);
        let alignment = self.collection_payload_alignment(value_type);
        let base = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let need = self.temporary_vreg();
        let old = self.temporary_vreg();
        let offset = self.temporary_vreg();
        let end = self.temporary_vreg();
        let align_scratch = self.temporary_vreg();
        let tail = self.label("inplace_mvwrite_tail");
        let write = self.label("inplace_mvwrite_write");
        self.emit(abi::load_u64(&base, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&entry, abi::stack_pointer(), index_slot));
        self.emit(abi::move_immediate(
            &old,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&entry, &entry, &old));
        self.emit(abi::add_registers(&entry, &entry, &base));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&need, abi::stack_pointer(), need_slot));
        self.emit(abi::load_u64(
            &old,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::compare_registers(&need, &old));
        self.emit(abi::branch_hi(&tail));
        // Fits: overwrite where it lies.
        self.emit(abi::load_u64(
            &offset,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::branch(&write));
        // Longer: the aligned data tail.
        self.emit(abi::label(&tail));
        self.emit(abi::load_u64(&offset, &base, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_align_offset_register(&offset, alignment, &align_scratch);
        self.emit(abi::add_registers(&end, &offset, &need));
        self.emit(abi::store_u64(&end, &base, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64(
            &offset,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::label(&write));
        self.emit(abi::store_u64(
            &need,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(&offset, abi::stack_pointer(), offset_slot));
        self.emit_copy_payload_to_collection(
            map_slot,
            need_slot,
            &payload,
            offset_slot,
            &ParameterType::named(""),
        )
    }

    /// Loop header/footer over the entries of the map in `map_slot`: `index_slot`
    /// runs `0..count`, re-read every iteration (the body may call). Returns
    /// `(index_slot, top, done)`; the body must end with `emit_map_loop_next`.
    fn emit_map_loop_head(&mut self, map_slot: usize, name: &str) -> (usize, String, String) {
        let index_slot = self.allocate_stack_object(&format!("{name}_index"), 8);
        let top = self.label(&format!("{name}_loop"));
        let done = self.label(&format!("{name}_done"));
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index_slot));
        self.emit(abi::label(&top));
        let base = self.temporary_vreg();
        let count = self.temporary_vreg();
        let index = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&done));
        (index_slot, top, done)
    }

    fn emit_map_loop_next(&mut self, index_slot: usize, top: &str, done: &str) {
        let index = self.temporary_vreg();
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::branch(top));
        self.emit(abi::label(done));
    }

    /// Byte `index` of the marks at the address in `marks_slot`, as a branch:
    /// `keep` when non-zero, else `skip`.
    fn emit_mark_branch(&mut self, marks_slot: usize, index_slot: usize, keep: &str, skip: &str) {
        let marks = self.temporary_vreg();
        let index = self.temporary_vreg();
        self.emit(abi::load_u64(&marks, abi::stack_pointer(), marks_slot));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::add_registers(&marks, &marks, &index));
        self.emit(abi::load_u8(&marks, &marks, 0));
        self.emit(abi::compare_immediate(&marks, "0"));
        self.emit(abi::branch_ne(keep));
        self.emit(abi::branch(skip));
    }

    /// Store `value` (0/1) as mark `index`.
    fn emit_store_mark(&mut self, marks_slot: usize, index_slot: usize, value: &VirtualRegister) {
        let marks = self.temporary_vreg();
        let index = self.temporary_vreg();
        self.emit(abi::load_u64(&marks, abi::stack_pointer(), marks_slot));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::add_registers(&marks, &marks, &index));
        self.emit(abi::store_u8(value, &marks, 0));
    }

    /// The room a batch of inserts taken from the map in `source_slot` can need:
    /// one entry and `keyLength + valueLength + 16` bytes (payloads plus their
    /// worst-case alignment) per entry — or per *marked* entry, when `marks_slot`
    /// is given. Returns `(entries_slot, bytes_slot)`.
    fn emit_map_batch_room(
        &mut self,
        source_slot: usize,
        marks_slot: Option<usize>,
    ) -> (usize, usize) {
        let entries_slot = self.allocate_stack_object("inplace_batch_entries", 8);
        let bytes_slot = self.allocate_stack_object("inplace_batch_bytes", 8);
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            entries_slot,
        ));
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), bytes_slot));
        let (index_slot, top, done) = self.emit_map_loop_head(source_slot, "inplace_batch");
        let add = self.label("inplace_batch_add");
        let skip = self.label("inplace_batch_skip");
        match marks_slot {
            Some(marks) => self.emit_mark_branch(marks, index_slot, &add, &skip),
            None => self.emit(abi::branch(&add)),
        }
        self.emit(abi::label(&add));
        let base = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let length = self.temporary_vreg();
        let total = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&entry, abi::stack_pointer(), index_slot));
        self.emit(abi::move_immediate(
            &length,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&entry, &entry, &length));
        self.emit(abi::add_registers(&entry, &entry, &base));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&total, abi::stack_pointer(), bytes_slot));
        self.emit(abi::load_u64(
            &length,
            &entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers(&total, &total, &length));
        self.emit(abi::load_u64(
            &length,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&total, &total, &length));
        self.emit(abi::add_immediate(&total, &total, 16));
        self.emit(abi::store_u64(&total, abi::stack_pointer(), bytes_slot));
        let entries = self.temporary_vreg();
        self.emit(abi::load_u64(&entries, abi::stack_pointer(), entries_slot));
        self.emit(abi::add_immediate(&entries, &entries, 1));
        self.emit(abi::store_u64(&entries, abi::stack_pointer(), entries_slot));
        self.emit(abi::label(&skip));
        self.emit_map_loop_next(index_slot, &top, &done);
        (entries_slot, bytes_slot)
    }

    /// Lower the other `Set`/`Map` operand into a fresh slot.
    fn lower_other_collection(
        &mut self,
        value: &NirValue,
        expected: &ParameterType,
        name: &str,
    ) -> Result<usize, String> {
        let lowered = self.lower_value(value)?;
        if lowered.type_ != *expected {
            return Err(format!(
                "native in-place {name}: the other operand must be {expected}, got {}",
                lowered.type_
            ));
        }
        let slot = self.allocate_stack_object(&format!("inplace_{name}_other"), 8);
        self.emit(abi::store_u64(
            &lowered.location,
            abi::stack_pointer(),
            slot,
        ));
        Ok(slot)
    }

    /// A `TRUE` word in a fresh slot — every `Set` element's value.
    fn emit_true_slot(&mut self) -> usize {
        let slot = self.allocate_stack_object("inplace_set_true", 8);
        let value = self.temporary_vreg();
        self.emit(abi::move_immediate(&value, "Boolean", "true"));
        self.emit(abi::store_u64(&value, abi::stack_pointer(), slot));
        slot
    }

    /// `add` every element of the set in `source_slot` (or every marked one) to
    /// the set in `dest_slot`, in the source's order. `inline` is each insert's
    /// `InlineGrow` at a field (plan-145-E); every insert reads the set through
    /// `dest_slot`, which a grow repoints.
    #[allow(clippy::too_many_arguments)]
    fn emit_add_all(
        &mut self,
        dest_slot: usize,
        source_slot: usize,
        marks_slot: Option<usize>,
        set_type: &ParameterType,
        element_type: &ParameterType,
        key_area: Option<usize>,
        inline: Option<InlineGrow>,
    ) -> Result<(), String> {
        let true_slot = self.emit_true_slot();
        let (index_slot, top, done) = self.emit_map_loop_head(source_slot, "inplace_add_all");
        let add = self.label("inplace_add_all_one");
        let skip = self.label("inplace_add_all_skip");
        match marks_slot {
            Some(marks) => self.emit_mark_branch(marks, index_slot, &add, &skip),
            None => self.emit(abi::branch(&add)),
        }
        self.emit(abi::label(&add));
        let key_slot = self.emit_map_entry_borrowed(
            source_slot,
            index_slot,
            Payload::Key,
            element_type,
            key_area,
            "inplace_add_all_key",
        )?;
        self.lower_map_set_in_place(
            dest_slot,
            key_slot,
            true_slot,
            set_type,
            element_type,
            &ParameterType::Boolean,
            inline,
        )?;
        self.emit(abi::label(&skip));
        self.emit_map_loop_next(index_slot, &top, &done);
        Ok(())
    }

    /// One mark per entry of the set in `scan_slot`: whether its element is a
    /// member of the set in `probe_slot`.
    #[allow(clippy::too_many_arguments)]
    fn emit_membership_marks(
        &mut self,
        scan_slot: usize,
        probe_slot: usize,
        probe_type: &ParameterType,
        element_type: &ParameterType,
        marks_slot: usize,
        key_area: Option<usize>,
        name: &str,
    ) -> Result<(), String> {
        let (index_slot, top, done) = self.emit_map_loop_head(scan_slot, name);
        let key_slot = self.emit_map_entry_borrowed(
            scan_slot,
            index_slot,
            Payload::Key,
            element_type,
            key_area,
            name,
        )?;
        let member =
            self.emit_key_membership(probe_slot, key_slot, element_type, name, probe_type)?;
        let mark = self.temporary_vreg();
        self.emit(abi::move_register(&mark, &member.location));
        self.emit_store_mark(marks_slot, index_slot, &mark);
        self.emit_map_loop_next(index_slot, &top, &done);
        Ok(())
    }

    /// The sum of the counts of the maps in `map_slots`, into a fresh slot.
    fn emit_map_counts(&mut self, map_slots: &[usize]) -> usize {
        let slot = self.allocate_stack_object("inplace_map_counts", 8);
        let total = self.temporary_vreg();
        self.emit(abi::move_immediate(&total, "Integer", "0"));
        for &map in map_slots {
            let base = self.temporary_vreg();
            let count = self.temporary_vreg();
            self.emit(abi::load_u64(&base, abi::stack_pointer(), map));
            self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::add_registers(&total, &total, &count));
        }
        self.emit(abi::store_u64(&total, abi::stack_pointer(), slot));
        slot
    }

    /// A borrowed-`String` scratch area size for the keys (or values) of the map
    /// in `map_slot`, or `None` when that payload is not a `String`.
    fn emit_borrow_area_size(
        &mut self,
        map_slot: usize,
        which: Payload,
        type_: &ParameterType,
    ) -> Option<usize> {
        if *type_ != ParameterType::String {
            return None;
        }
        let max = self.emit_map_max_payload_len(map_slot, which);
        Some(self.emit_string_area_size(max))
    }

    fn clear_self_update_constant(&mut self, name: &str) {
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
    }

    /// The shared prologue of the four set operations: the container gates, `G9`
    /// (a `Set`), and whether the other operand is the binding itself.
    fn resolve_set_op<'v>(
        &self,
        site: &SelfUpdateSite<'_>,
        value: &'v NirValue,
        builtin: &str,
    ) -> Option<(SelfUpdateTarget<'v>, ParameterType, bool)> {
        let target = self.resolve_self_update(site, value, builtin, 2)?;
        let element_type = typed_set_element_type(&target.collection_type).cloned()?;
        let alias = site.is_self(&target.args[1]);
        Some((target, element_type, alias))
    }

    /// `result_slot = results[index]` — word `index` of the array whose address is
    /// in `results_slot`.
    fn emit_load_result_word(
        &mut self,
        results_slot: usize,
        index_slot: usize,
        result_slot: usize,
    ) {
        let results = self.temporary_vreg();
        let index = self.temporary_vreg();
        self.emit(abi::load_u64(&results, abi::stack_pointer(), results_slot));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::shift_left_immediate(&index, &index, 3));
        self.emit(abi::add_registers(&results, &results, &index));
        self.emit(abi::load_u64(&results, &results, 0));
        self.emit(abi::store_u64(&results, abi::stack_pointer(), result_slot));
    }
}

/// Whether a map value of this type is replaced by one of its own size (so a
/// write-back never grows the data region).
fn value_type_is_fixed(type_: &ParameterType) -> bool {
    crate::codegen::collection::layout::kind2_payload_size(type_).is_some()
}
