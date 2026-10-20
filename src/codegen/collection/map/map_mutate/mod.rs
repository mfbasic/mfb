// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::list_element_is_fixed_width;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;

mod set_in_place;

/// plan-121-C: where a **growing** map/set mutation must put its new block when
/// the collection is *inlined* in a record or `STATE` field.
///
/// A growing operation reallocates, and for an inlined field the thing that must
/// grow is the **record** block, not the collection alone — the collection has no
/// allocation of its own to replace. `lower_map_set_in_place` therefore needs to
/// know three things at each of its two grow sites, and nothing else about the
/// container:
///
/// * allocate `fieldOffset + <the map size it already computed>` rather than just
///   the map size;
/// * the new map buffer is `newRecord + fieldOffset`, and the bytes before it are
///   the record's fixed slots, copied across verbatim;
/// * free the **old record block**, not the old map buffer —
///   `emit_free_pre_grow_buffer` on an inlined sub-block would `free()` a pointer
///   into the middle of the record block.
///
/// Passed as `Option`, and `None` is the plain-local case every existing caller
/// wants: the parameter is explicit rather than builder state precisely so that
/// "which container am I growing" is greppable at every call site instead of
/// being ambient (plan-121-C Correction C2).
#[derive(Debug, Clone, Copy)]
pub(crate) struct InlineGrow {
    /// Frame slot holding the owning **record** block pointer. Repointed on a
    /// realloc, which is what makes the grown block visible to `rec.field`.
    pub(crate) block_slot: usize,
    /// Frame slot holding the field's block-relative offset. Invariant across the
    /// realloc — the prefix is copied verbatim — so it is read once by the caller
    /// and handed in.
    pub(crate) field_off_slot: usize,
}

/// bug-760 B1: whether a value of `type_` is a `Map` or `Set` block — the shapes
/// whose entries can be tombstones.
pub(crate) fn is_map_or_set_type(type_: &ParameterType) -> bool {
    typed_map_type_parts(type_).is_some()
        || crate::codegen::engine::types::typed_set_element_type(type_).is_some()
}

/// bug-768: whether a `Map`/`Set` of `type_` can hold tombstones at all — only
/// a key (element) type with a hash probe ever leaves one
/// (`lower_map_remove_key_in_place` ANDs its `tombstone` with
/// `map_key_probe_eligible`; a scan would match a dead key).
pub(crate) fn map_or_set_may_tombstone(type_: &ParameterType) -> bool {
    typed_map_type_parts(type_)
        .map(|(key, _)| key)
        .or_else(|| crate::codegen::engine::types::typed_set_element_type(type_))
        .is_some_and(CodeBuilder::map_key_probe_eligible)
}

impl CodeBuilder<'_> {
    /// bug-768: the field-index paths from a block of record `record_type` to
    /// every inlined `Map`/`Set` field that can hold tombstones, through inlined
    /// record fields only — exactly the places an in-place field removal reaches
    /// (`InPlaceDest::Inlined { field_index, path }`). A pointer field is never an
    /// in-place field destination, and an inlined union or collection is copied
    /// in whole from a value that was settled when it was read.
    pub(crate) fn tombstone_field_paths(&self, record_type: &ParameterType) -> Vec<Vec<usize>> {
        let mut out = Vec::new();
        self.collect_tombstone_field_paths(record_type, &mut Vec::new(), &mut out);
        out
    }

    fn collect_tombstone_field_paths(
        &self,
        record_type: &ParameterType,
        path: &mut Vec<usize>,
        out: &mut Vec<Vec<usize>>,
    ) {
        let Some(fields) = self.type_model.record_fields.get(record_type) else {
            return;
        };
        for (index, (_, field_type)) in fields.iter().enumerate() {
            // An inlined record is memcpy-copyable, so it cannot contain itself:
            // the walk ends.
            if !self.record_field_is_inlined(field_type) {
                continue;
            }
            path.push(index);
            if map_or_set_may_tombstone(field_type) {
                out.push(path.clone());
            } else {
                self.collect_tombstone_field_paths(field_type, path, out);
            }
            path.pop();
        }
    }

    /// bug-768: settle `value` — a `Map`/`Set`, or a record (a record local or
    /// global, a record field, a `STATE` payload) whose inlined fields may hold
    /// tombstones — before a consumer that is not tombstone-aware sees it. A map
    /// or set is settled as a binding is ([`Self::emit_map_settle_value`]); a
    /// record has each sub-block on [`Self::tombstone_field_paths`] settled in
    /// place, each a dead-count check when it holds none. Nothing moves, so the
    /// record's stored offsets and size stay valid. Any other value is returned
    /// untouched, emitting nothing.
    pub(crate) fn emit_tombstone_holder_settle(&mut self, value: ValueResult) -> ValueResult {
        if map_or_set_may_tombstone(&value.type_) {
            return self.emit_map_settle_value(value);
        }
        if !self.type_model.record_fields.contains_key(&value.type_) {
            return value;
        }
        let paths = self.tombstone_field_paths(&value.type_);
        if paths.is_empty() {
            return value;
        }
        let base = self.temporary_vreg();
        let skip = self.label("record_settle_skip");
        self.emit(abi::move_register(&base, &value.location));
        self.emit(abi::compare_immediate(&base, "0"));
        self.emit(abi::branch_eq(&skip));
        for path in paths {
            let sub_block = self.temporary_vreg();
            self.emit(abi::move_register(&sub_block, &base));
            for index in path {
                // An inlined field's slot holds its offset from the start of the
                // record that declares it (`lower_field_access_of`).
                let offset = self.temporary_vreg();
                self.emit(abi::load_u64(&offset, &sub_block, 8 * index));
                self.emit(abi::add_registers(&sub_block, &sub_block, &offset));
            }
            self.emit_map_settle_value(ValueResult {
                origin: None,
                type_: ParameterType::Nothing,
                location: Operand::from(sub_block),
                text: String::new(),
            });
        }
        self.emit(abi::label(&skip));
        ValueResult {
            location: Operand::from(base),
            ..value
        }
    }
}

impl CodeBuilder<'_> {
    /// bug-760 B1: settle the `Map`/`Set` block `value` (read out of a binding)
    /// before a consumer that is not tombstone-aware sees it: when its dead count
    /// is not 0, `_mfb_rt_map_settle` drops the tombstones in place. The block
    /// does not move; the result names a fresh register holding it. A null block
    /// (a binding not yet initialized) is passed through untouched.
    pub(crate) fn emit_map_settle_value(&mut self, value: ValueResult) -> ValueResult {
        let map = self.temporary_vreg();
        let dead = self.temporary_vreg();
        let skip = self.label("map_settle_skip");
        self.emit(abi::move_register(&map, &value.location));
        self.emit(abi::compare_immediate(&map, "0"));
        self.emit(abi::branch_eq(&skip));
        self.emit(abi::load_u32(&dead, &map, COLLECTION_OFFSET_MAP_STATE));
        self.emit(abi::shift_right_immediate(
            &dead,
            &dead,
            MAP_DEAD_ENTRIES_SHIFT,
        ));
        self.emit(abi::compare_immediate(&dead, "0"));
        self.emit(abi::branch_eq(&skip));
        self.emit(abi::move_register(abi::c_arg(0), &map));
        self.emit(abi::branch_link(MAP_SETTLE_SYMBOL));
        self.push_internal_call_relocation(MAP_SETTLE_SYMBOL);
        self.emit(abi::move_register(&map, abi::mfb_return(0)));
        self.emit(abi::label(&skip));
        ValueResult {
            location: Operand::from(map),
            ..value
        }
    }

    /// bug-760 B1: settle the `Map`/`Set` whose block pointer is in `map_slot`
    /// (see [`Self::emit_map_settle_value`]) — for an in-place arm that walks every
    /// entry of its destination. The slot is not rewritten: the block does not
    /// move.
    pub(crate) fn emit_map_settle_slot(&mut self, map_slot: usize) {
        let map = self.temporary_vreg();
        self.emit(abi::load_u64(&map, abi::stack_pointer(), map_slot));
        let value = ValueResult {
            origin: None,
            type_: ParameterType::Nothing,
            location: Operand::from(map),
            text: String::new(),
        };
        self.emit_map_settle_value(value);
    }

    /// bug-760 B1: finish an in-place removal of entry `index_slot` of the map in
    /// `map_slot`, whose bucket [`Self::emit_map_mark_removed`] has deleted and
    /// flagged — `_mfb_rt_map_retire` truncates it at the back or leaves it as a
    /// tombstone, compacting once the tombstones outnumber the live entries.
    pub(crate) fn emit_map_retire(&mut self, map_slot: usize, index_slot: usize) {
        self.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            index_slot,
        ));
        self.emit(abi::branch_link(MAP_RETIRE_SYMBOL));
        self.push_internal_call_relocation(MAP_RETIRE_SYMBOL);
    }

    /// Set `key -> value` in the map whose buffer pointer lives in `map_slot`,
    /// **mutating the buffer in place** (plan-02 §4.3). Linear-scans for the key
    /// (linear scan): on a hit, overwrites the value bytes when the new value fits
    /// the old slot (`newLen <= oldLen`), else appends the new value to the spare
    /// data tail and repoints the entry (old value becomes dead slack, tightened on
    /// copy — amortized O(1) per set even when values grow). On a miss, writes the
    /// key+value into a
    /// spare lookup slot and the spare data tail (the entry packed exactly like
    /// `emit_write_collection_entry` — key then value, each aligned), bumping
    /// `count`/`dataLength`; when the live buffer is full it grows geometrically
    /// (copying entries + data verbatim, capacity-based base) and then writes. The
    /// caller guarantees unique ownership and not an active `FOR EACH` iterable.
    /// plan-121-C, step 1 of the inlined grow: widen the allocation about to
    /// happen so it holds the **record** block, not just the map.
    ///
    /// `lower_map_set_in_place` has already computed the new map size into
    /// `return_register()` (header + entries + data + bucket region). For an
    /// inlined field the block that must be allocated is `fieldOffset + that`, so
    /// this adds the prefix — through the same checked arithmetic the size
    /// computation uses, because `fieldOffset` is runtime-derived and a wrap here
    /// would under-allocate the record.
    pub(crate) fn emit_inline_grow_extend_size(&mut self, inline: &InlineGrow, overflow: &str) {
        let off = self.temporary_vreg();
        self.emit(abi::load_u64(
            &off,
            abi::stack_pointer(),
            inline.field_off_slot,
        ));
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &off,
            overflow,
        );
    }

    /// plan-121-C, step 2: split the freshly allocated record block into the
    /// record pointer and the map sub-block pointer, and copy the prefix across.
    ///
    /// The allocation returned the new **record** block. Everything before
    /// `fieldOffset` — the record's fixed slots and any earlier inlined sub-blocks
    /// — is copied verbatim, exactly as `lower_inline_list_append_in_place` does,
    /// which is why `fieldOffset` is invariant across the realloc and can be read
    /// once. `new_buf_slot` is then made to hold `newRecord + fieldOffset` so every
    /// line of the map lowering after this point writes through the sub-block
    /// without knowing it moved.
    ///
    /// Returns the slot holding the new record pointer, which the caller publishes
    /// only after the old block has been freed.
    pub(crate) fn emit_inline_grow_split(
        &mut self,
        inline: &InlineGrow,
        new_buf_slot: usize,
    ) -> usize {
        let new_rec_slot = self.allocate_stack_object("inline_grow_newrec", 8);
        let new_rec = self.temporary_vreg();
        let old_rec = self.temporary_vreg();
        let off = self.temporary_vreg();
        let scratch = self.temporary_vreg();

        // The allocator's result is the new record block.
        self.emit(abi::load_u64(&new_rec, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&new_rec, abi::stack_pointer(), new_rec_slot));

        // Copy the fixed prefix [0, fieldOffset) verbatim.
        self.emit(abi::load_u64(
            &off,
            abi::stack_pointer(),
            inline.field_off_slot,
        ));
        self.emit(abi::load_u64(
            &old_rec,
            abi::stack_pointer(),
            inline.block_slot,
        ));
        self.emit_block_copy_advance(&new_rec, &old_rec, &off, &scratch, "inline_grow_prefix");

        // `new_buf_slot` becomes the sub-block address inside the new record.
        // `emit_block_copy_advance` leaves its cursors past the copied range, so
        // re-derive both operands rather than reusing them.
        self.emit(abi::load_u64(&new_rec, abi::stack_pointer(), new_rec_slot));
        self.emit(abi::load_u64(
            &off,
            abi::stack_pointer(),
            inline.field_off_slot,
        ));
        self.emit(abi::add_registers(&new_rec, &new_rec, &off));
        self.emit(abi::store_u64(&new_rec, abi::stack_pointer(), new_buf_slot));
        new_rec_slot
    }

    /// plan-121-C, step 3: free the **old record block**, and publish the new one.
    ///
    /// This is the step `emit_free_pre_grow_buffer` cannot do for an inlined field:
    /// the old map buffer is not its own allocation, it is bytes inside the old
    /// record block, so freeing it would hand `arena_free` a pointer into the
    /// middle of a live allocation. What must be released is the whole old record,
    /// whose size is `fieldOffset + <the old map block's size>` — the same shape
    /// `lower_inline_list_append_in_place` frees, and the reason
    /// `record_collection_last_inlined` insists the field is *last*: nothing
    /// follows the sub-block, so the record ends where it ends.
    pub(crate) fn emit_inline_grow_free_old(
        &mut self,
        inline: &InlineGrow,
        map_slot: usize,
        map_type: &ParameterType,
        new_rec_slot: usize,
    ) -> Result<(), String> {
        let old_rec = self.temporary_vreg();
        let old_sub = self.temporary_vreg();
        let size = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        let off = self.temporary_vreg();

        // Size the old map sub-block where it still lies, then add the prefix.
        self.emit(abi::load_u64(&old_sub, abi::stack_pointer(), map_slot));
        self.emit_flat_block_size(map_type, &old_sub, &size, &scratch)?;
        self.emit(abi::load_u64(
            &off,
            abi::stack_pointer(),
            inline.field_off_slot,
        ));
        self.emit(abi::add_registers(&size, &size, &off));

        self.emit(abi::load_u64(
            &old_rec,
            abi::stack_pointer(),
            inline.block_slot,
        ));
        self.emit(abi::move_register(abi::c_arg(0), &old_rec));
        self.emit(abi::move_register(abi::c_arg(1), &size));
        self.emit_arena_free_call();

        // Publish the grown record. Only now: everything above still had to read
        // the old block.
        let new_rec = self.temporary_vreg();
        self.emit(abi::load_u64(&new_rec, abi::stack_pointer(), new_rec_slot));
        self.emit(abi::store_u64(
            &new_rec,
            abi::stack_pointer(),
            inline.block_slot,
        ));
        Ok(())
    }

    /// bug-733: the first step of every in-place grow of a `Map`/`Set` — reclaim
    /// the dead key and value bytes instead of copying them. Calls
    /// `_mfb_rt_map_reclaim` (see `lower_map_reclaim_helper`, which allocates
    /// nothing and cannot fail): when it repacked, branch to `retry` (the caller's
    /// room check, which recomputes from the new `dataLength`); otherwise fall
    /// through to the grow. `map_slot` may hold a record field's sub-block
    /// address: the block does not move.
    ///
    /// bug-760 B1: the reclaim also drops tombstones, which moves entries; a
    /// caller that holds an entry index in a slot across it names the slot in
    /// `tracked_index_slot` and finds it rewritten to the entry's new index.
    pub(crate) fn emit_map_reclaim_before_grow(
        &mut self,
        map_slot: usize,
        map_type: &ParameterType,
        retry: &str,
        tracked_index_slot: Option<usize>,
    ) -> Result<(), String> {
        let (key_type, value_type) = match typed_map_type_parts(map_type) {
            Some((key, value)) => (key.clone(), value.clone()),
            None => {
                let element = crate::codegen::engine::types::typed_set_element_type(map_type)
                    .cloned()
                    .ok_or_else(|| format!("native map reclaim of non-map {map_type}"))?;
                (element, ParameterType::Boolean)
            }
        };
        let key_align = self.collection_payload_alignment(&key_type);
        let value_align = self.collection_payload_alignment(&value_type);
        let grow = self.label("map_reclaim_grow");
        let status = self.temporary_vreg();
        self.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), map_slot));
        self.emit(abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            &key_align.to_string(),
        ));
        self.emit(abi::move_immediate(
            abi::c_arg(2),
            "Integer",
            &value_align.to_string(),
        ));
        match tracked_index_slot {
            Some(slot) => self.emit(abi::add_immediate(
                abi::c_arg(3),
                abi::stack_pointer(),
                slot,
            )),
            None => self.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0")),
        }
        self.emit(abi::branch_link(MAP_RECLAIM_SYMBOL));
        self.push_internal_call_relocation(MAP_RECLAIM_SYMBOL);
        self.emit(abi::move_register(&status, abi::mfb_return(0)));
        self.emit(abi::compare_immediate(&status, "0"));
        self.emit(abi::branch_ne(retry));
        self.emit(abi::label(&grow));
        Ok(())
    }

    /// bug-732: the index of the entry of the map in `map_slot` whose key equals
    /// the one in `key_slot`, into a fresh slot; branches to `not_found` when there
    /// is none. A probe-eligible key type (`map_key_probe_eligible`) goes through
    /// the hash index (`emit_map_probe`, which builds it lazily), so the lookup
    /// costs O(1) on average; any other key type compares entry by entry, which is
    /// that type's lookup cost everywhere. Writes nothing.
    pub(crate) fn emit_map_find_index(
        &mut self,
        map_slot: usize,
        key_slot: usize,
        key_type: &ParameterType,
        not_found: &str,
    ) -> Result<usize, String> {
        let index_slot = self.allocate_stack_object("map_find_index", 8);
        if Self::map_key_probe_eligible(key_type) {
            // bug-770 B.4: the probe yields the index (no division by ENTRY_SIZE).
            let entry_slot = self.allocate_stack_object("map_find_entry", 8);
            self.emit_map_probe_into(
                map_slot,
                key_slot,
                key_type,
                not_found,
                entry_slot,
                Some(index_slot),
                None,
            )?;
            return Ok(index_slot);
        }
        let map = self.temporary_vreg();
        let key = self.temporary_vreg();
        let count = self.temporary_vreg();
        let index = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let key_off = self.temporary_vreg();
        let key_len = self.temporary_vreg();
        let scan_loop = self.label("mrk_scan_loop");
        let scan_match = self.label("mrk_scan_match");
        let scan_next = self.label("mrk_scan_next");
        self.emit(abi::load_u64(&map, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&key, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(&count, &map, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::add_immediate(&entry, &map, COLLECTION_HEADER_SIZE));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(not_found));
        self.emit(abi::load_u64(
            &key_off,
            &entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &key_len,
            &entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        // arg7 = on-MATCH, arg8 = on-NO-MATCH (per `lower_map_remove_key`).
        self.emit_collection_payload_matches_value_branch(
            key_type,
            &ParameterType::named(""),
            &map,
            &key_off,
            &key_len,
            &key,
            &scan_match,
            &scan_next,
        )?;
        self.emit(abi::label(&scan_next));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
        self.emit(abi::branch(&scan_loop));
        // Keys are unique: the first match is the only one.
        self.emit(abi::label(&scan_match));
        self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
        Ok(index_slot)
    }

    /// bug-732: claim entry `index_slot` of the map in `map_slot` for removal —
    /// delete its bucket from the hash index while every entry is still where the
    /// index says (`_mfb_rt_map_bucket_delete`), then mark it
    /// `COLLECTION_ENTRY_FLAG_REMOVED`. Nothing moves until
    /// [`Self::emit_map_compact_from`]. A value the entry owns must be freed first.
    pub(crate) fn emit_map_mark_removed(&mut self, map_slot: usize, index_slot: usize) {
        self.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            index_slot,
        ));
        self.emit(abi::branch_link(MAP_BUCKET_DELETE_SYMBOL));
        self.push_internal_call_relocation(MAP_BUCKET_DELETE_SYMBOL);
        let map = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let flag = self.temporary_vreg();
        self.emit(abi::load_u64(&map, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&entry, abi::stack_pointer(), index_slot));
        self.emit(abi::move_immediate(
            &flag,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&entry, &entry, &flag));
        self.emit(abi::add_registers(&entry, &entry, &map));
        self.emit(abi::move_immediate(
            &flag,
            "Byte",
            &COLLECTION_ENTRY_FLAG_REMOVED.to_string(),
        ));
        self.emit(abi::store_u8(
            &flag,
            &entry,
            COLLECTION_HEADER_SIZE + COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
    }

    /// bug-732: drop the entries [`Self::emit_map_mark_removed`] claimed, from the
    /// index in `from_slot` (the smallest claimed) on, keeping the survivors'
    /// order and the hash index (`_mfb_rt_map_compact_from`). Costs the entries
    /// from `from` to the end — the ones that move — and nothing before.
    pub(crate) fn emit_map_compact_from(&mut self, map_slot: usize, from_slot: usize) {
        self.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            from_slot,
        ));
        self.emit(abi::branch_link(MAP_COMPACT_FROM_SYMBOL));
        self.push_internal_call_relocation(MAP_COMPACT_FROM_SYMBOL);
    }

    /// Remove `key` from the map (or element from the set) whose block pointer is
    /// in `map_slot`, **in place** (plan-86 D1, bug-732). The key is found by
    /// [`Self::emit_map_find_index`]; a missing key leaves the map unchanged. A
    /// value that owns a graph is freed (plan-134-H) and the entry's bucket is
    /// deleted from the hash index. Then:
    ///
    /// * `tombstone` (bug-760 B1) — `_mfb_rt_map_retire` truncates the entry when
    ///   it is the last, else leaves it as a tombstone, compacting once the
    ///   tombstones outnumber the live entries: O(1) amortized plus the lookup,
    ///   wherever the key sits, so removing the oldest key of a FIFO no longer
    ///   moves every entry. Only for a probe-eligible key type (a scanned key type
    ///   would match a tombstone's key) at a place whose every other read settles
    ///   (`lower_value`'s map settle): a local, a global or a by-reference capture,
    ///   and (bug-768) a record or `STATE` field, whose field reads and whole-owner
    ///   reads settle the sub-block (`emit_tombstone_holder_settle`).
    /// * otherwise (a scanned key type) — the later entries slide down one slot
    ///   with their buckets renumbered (`_mfb_rt_map_compact_from`), leaving no
    ///   tombstone.
    ///
    /// Either way insertion order is kept and the index stays built, and the
    /// removed key and value bytes stay behind in the data region as dead slack —
    /// the same model `set` uses when it overwrites a value — for the reclaim.
    ///
    /// Every access to `map_slot` is a load, so a record field's map can be handed
    /// in as its sub-block address (plan-145-B).
    pub(crate) fn lower_map_remove_key_in_place(
        &mut self,
        map_slot: usize,
        key_slot: usize,
        map_type: &ParameterType,
        key_type: &ParameterType,
        tombstone: bool,
    ) -> Result<ValueResult, String> {
        let tombstone = tombstone && Self::map_key_probe_eligible(key_type);
        let done = self.label("mrk_done");
        let found_slot = self.emit_map_find_index(map_slot, key_slot, key_type, &done)?;
        // plan-134-H: the removed entry's value owns a graph nothing else reaches; free it
        // before the entry is dropped.
        if let Some((_, value_type)) = typed_map_type_parts(map_type) {
            let value_type = value_type.clone();
            if self.owns_graph(&value_type) {
                let entry_slot = self.allocate_stack_object("mrk_removed_entry", 8);
                let map = self.temporary_vreg();
                let entry = self.temporary_vreg();
                let size = self.temporary_vreg();
                self.emit(abi::load_u64(&map, abi::stack_pointer(), map_slot));
                self.emit(abi::load_u64(&entry, abi::stack_pointer(), found_slot));
                self.emit(abi::move_immediate(
                    &size,
                    "Integer",
                    &COLLECTION_ENTRY_SIZE.to_string(),
                ));
                self.emit(abi::multiply_registers(&entry, &entry, &size));
                self.emit(abi::add_registers(&entry, &entry, &map));
                self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
                self.emit(abi::store_u64(&entry, abi::stack_pointer(), entry_slot));
                self.emit_drop_entry_value(map_slot, entry_slot, &value_type)?;
            }
        }
        self.emit_map_mark_removed(map_slot, found_slot);
        if tombstone {
            self.emit_map_retire(map_slot, found_slot);
        } else {
            self.emit_map_compact_from(map_slot, found_slot);
        }
        self.emit(abi::label(&done));
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), map_slot));
        Ok(ValueResult {
            origin: None,
            type_: map_type.clone(),
            location: Operand::from(result),
            text: format!("removeKey_in_place({map_type}, {key_type})"),
        })
    }
}
