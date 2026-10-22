//! plan-176-Q: in-place set algebra — union, intersection, difference and symmetric difference.
//! The shared map/scratch emitters live in `collection/assign/builder_inplace_setmap/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// `s = collections::union(s, t)`.
    pub(crate) fn try_inplace_union_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        if self.self_update_scratch.is_none() {
            return Ok(false);
        }
        let Some((target, element_type, alias)) = self.resolve_set_op(site, value, "union") else {
            return Ok(false);
        };
        // plan-145-E: the inserts grow the set.
        if !self.field_realloc_admitted(site, &target.args[1..]) {
            return Ok(false);
        }
        let opened = self.open_inplace_dest(&target.dest)?;
        if !alias {
            let set_type = target.collection_type.clone();
            let other = self.lower_other_collection(&target.args[1], &set_type, "union")?;
            let dest = self.inplace_collection_slot(&opened)?;
            let grow = self.inplace_inline_grow(&opened)?;
            let key_area = match self.emit_borrow_area_size(other, Payload::Key, &element_type) {
                Some(size) => Some(self.emit_scratch_areas(&[size])?[0]),
                None => None,
            };
            let (entries, bytes) = self.emit_map_batch_room(other, None);
            self.emit_map_reserve(dest, entries, bytes, &set_type, grow)?;
            self.emit_add_all(dest, other, None, &set_type, &element_type, key_area, grow)?;
        }
        self.close_field_dest(&target.dest, &opened)?;
        self.clear_self_update_constant(site.name);
        Ok(true)
    }

    /// `s = collections::intersection(s, t)`: every element of `s` is tested, so
    /// one membership pass marks `s` and one compaction keeps the members.
    pub(crate) fn try_inplace_intersection_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        if self.self_update_scratch.is_none() {
            return Ok(false);
        }
        let Some((target, element_type, alias)) = self.resolve_set_op(site, value, "intersection")
        else {
            return Ok(false);
        };
        let set_type = target.collection_type.clone();
        let opened = self.open_inplace_dest(&target.dest)?;
        // `intersection(s, s)` is `s`.
        if !alias {
            let other = self.lower_other_collection(&target.args[1], &set_type, "intersection")?;
            let dest = self.inplace_collection_slot(&opened)?;
            // bug-760 B1: every entry of `s` is marked and compacted — drop its
            // tombstones first (bug-768: a field's set may hold them too).
            self.emit_map_settle_slot(dest);
            let marks_size = self.emit_map_counts(&[dest]);
            let mut sizes = vec![marks_size];
            let area_size = self.emit_borrow_area_size(dest, Payload::Key, &element_type);
            sizes.extend(area_size);
            let areas = self.emit_scratch_areas(&sizes)?;
            let marks = areas[0];
            let key_area = areas.get(1).copied();
            self.emit_membership_marks(
                dest,
                other,
                &set_type,
                &element_type,
                marks,
                key_area,
                "inplace_intersection_mark",
            )?;
            self.lower_set_compact_in_place(dest, marks);
        }
        self.close_field_dest(&target.dest, &opened)?;
        self.clear_self_update_constant(site.name);
        Ok(true)
    }

    /// `s = collections::difference(s, t)` (bug-732). Each element of `t` is
    /// looked up in `s` (`emit_map_find_index`); a hit is claimed for removal,
    /// deleting its bucket, and one compaction then drops the claimed entries from
    /// the first of them on. The call costs `t`'s elements plus the entries of
    /// `s` that move — never a pass over all of `s`, which marking every element
    /// of `s` against `t` was.
    pub(crate) fn try_inplace_difference_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        if self.self_update_scratch.is_none() {
            return Ok(false);
        }
        let Some((target, element_type, alias)) = self.resolve_set_op(site, value, "difference")
        else {
            return Ok(false);
        };
        let set_type = target.collection_type.clone();
        let opened = self.open_inplace_dest(&target.dest)?;
        if alias {
            // `difference(s, s)` is empty.
            let dest = self.inplace_collection_slot(&opened)?;
            self.lower_set_clear_in_place(dest);
        } else {
            let other = self.lower_other_collection(&target.args[1], &set_type, "difference")?;
            let dest = self.inplace_collection_slot(&opened)?;
            let key_area = match self.emit_borrow_area_size(other, Payload::Key, &element_type) {
                Some(size) => Some(self.emit_scratch_areas(&[size])?[0]),
                None => None,
            };
            let tombstone = Self::map_key_probe_eligible(&element_type);
            let from_slot =
                self.emit_claim_members(dest, other, &element_type, key_area, None, tombstone)?;
            if !tombstone {
                self.emit_compact_claimed(dest, from_slot);
            }
        }
        self.close_field_dest(&target.dest, &opened)?;
        self.clear_self_update_constant(site.name);
        Ok(true)
    }

    /// bug-732: for each element of the set in `probe_slot`, find it in the set in
    /// `dest_slot` and, on a hit, claim that entry for removal
    /// (`emit_map_mark_removed`). Returns a slot holding the smallest claimed index
    /// — or `dest`'s count when nothing was claimed. When `absent_marks_slot` is
    /// given, mark byte `j` records whether element `j` of the probe set was
    /// absent from `dest` (1) or claimed (0). Costs the probe set's elements.
    ///
    /// bug-760 B1: with `tombstone`, each claim is retired at once
    /// (`emit_map_retire`) — a tombstone, or a truncation at the back — exactly as
    /// an in-place `remove` retires its entry, so no compaction follows and the
    /// call costs the probe set's elements whatever entries of `dest` they hit.
    fn emit_claim_members(
        &mut self,
        dest_slot: usize,
        probe_slot: usize,
        element_type: &ParameterType,
        key_area: Option<usize>,
        absent_marks_slot: Option<usize>,
        tombstone: bool,
    ) -> Result<usize, String> {
        let from_slot = self.allocate_stack_object("inplace_claim_from", 8);
        let base = self.temporary_vreg();
        let count = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), dest_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), from_slot));
        let (index_slot, top, done) = self.emit_map_loop_head(probe_slot, "inplace_claim");
        let next = self.label("inplace_claim_next");
        let key_slot = self.emit_map_entry_borrowed(
            probe_slot,
            index_slot,
            Payload::Key,
            element_type,
            key_area,
            "inplace_claim_key",
        )?;
        if let Some(marks) = absent_marks_slot {
            let one = self.temporary_vreg();
            self.emit(abi::move_immediate(&one, "Integer", "1"));
            self.emit_store_mark(marks, index_slot, &one);
        }
        let found = self.emit_map_find_index(dest_slot, key_slot, element_type, &next)?;
        if let Some(marks) = absent_marks_slot {
            let zero = self.temporary_vreg();
            self.emit(abi::move_immediate(&zero, "Integer", "0"));
            self.emit_store_mark(marks, index_slot, &zero);
        }
        self.emit_map_mark_removed(dest_slot, found);
        if tombstone {
            self.emit_map_retire(dest_slot, found);
        }
        let lowest = self.label("inplace_claim_lowest");
        let from = self.temporary_vreg();
        let index = self.temporary_vreg();
        self.emit(abi::load_u64(&from, abi::stack_pointer(), from_slot));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), found));
        self.emit(abi::compare_registers(&index, &from));
        self.emit(abi::branch_ge(&lowest));
        self.emit(abi::store_u64(&index, abi::stack_pointer(), from_slot));
        self.emit(abi::label(&lowest));
        self.emit(abi::label(&next));
        self.emit_map_loop_next(index_slot, &top, &done);
        Ok(from_slot)
    }

    /// Drop the entries `emit_claim_members` claimed — none when `from` is still
    /// the count.
    fn emit_compact_claimed(&mut self, dest_slot: usize, from_slot: usize) {
        let skip = self.label("inplace_claim_none");
        let base = self.temporary_vreg();
        let count = self.temporary_vreg();
        let from = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), dest_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&from, abi::stack_pointer(), from_slot));
        self.emit(abi::compare_registers(&from, &count));
        self.emit(abi::branch_ge(&skip));
        self.emit_map_compact_from(dest_slot, from_slot);
        self.emit(abi::label(&skip));
    }

    /// `s = collections::symmetricDifference(s, t)`.
    pub(crate) fn try_inplace_symmetric_difference_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        if self.self_update_scratch.is_none() {
            return Ok(false);
        }
        let Some((target, element_type, alias)) =
            self.resolve_set_op(site, value, "symmetricDifference")
        else {
            return Ok(false);
        };
        let set_type = target.collection_type.clone();
        // plan-145-E: the inserts grow the set.
        if !self.field_realloc_admitted(site, &target.args[1..]) {
            return Ok(false);
        }
        let opened = self.open_inplace_dest(&target.dest)?;
        if alias {
            let dest = self.inplace_collection_slot(&opened)?;
            self.lower_set_clear_in_place(dest);
        } else {
            let other =
                self.lower_other_collection(&target.args[1], &set_type, "symmetricDifference")?;
            let dest = self.inplace_collection_slot(&opened)?;
            let grow = self.inplace_inline_grow(&opened)?;
            // bug-732: only `t` is walked. The room for every element of `t` is
            // reserved first — the only allocation, before the first write — then
            // each element of `t` is looked up in `s`: a hit is claimed for
            // removal, a miss marked for the add. One compaction drops the claimed
            // entries (from the first of them on) and the misses are added in
            // `t`'s order after `s`'s survivors, which is the copying result.
            let marks_size = self.emit_map_counts(&[other]);
            let mut sizes = vec![marks_size];
            sizes.extend(self.emit_borrow_area_size(other, Payload::Key, &element_type));
            let areas = self.emit_scratch_areas(&sizes)?;
            let marks_t = areas[0];
            let key_area = areas.get(1).copied();
            let (entries, bytes) = self.emit_map_batch_room(other, None);
            self.emit_map_reserve(dest, entries, bytes, &set_type, grow)?;
            let tombstone = Self::map_key_probe_eligible(&element_type);
            let from_slot = self.emit_claim_members(
                dest,
                other,
                &element_type,
                key_area,
                Some(marks_t),
                tombstone,
            )?;
            if !tombstone {
                self.emit_compact_claimed(dest, from_slot);
            }
            self.emit_add_all(
                dest,
                other,
                Some(marks_t),
                &set_type,
                &element_type,
                key_area,
                grow,
            )?;
        }
        self.close_field_dest(&target.dest, &opened)?;
        self.clear_self_update_constant(site.name);
        Ok(true)
    }
}
