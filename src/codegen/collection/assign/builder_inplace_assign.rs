// --- codegen tier imports (migration) ---
use crate::codegen::collection::assign::inplace_dest::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::control::{nir_value_reads_local, string_self_append_operands};
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    /// Recognize `name = collections::append(name, item)` for a single element
    /// appended to a uniquely-owned `MUT` list local, and lower it as an in-place
    /// grow (plan-01 §4.2). Returns `true` when handled (the local's slot was
    /// updated in place); `false` to fall back to the general reassignment path.
    ///
    /// Soundness: under MFBASIC value semantics every binding owns its buffer and
    /// copy-insertion deep-copies any aliasing assignment, so the local's buffer
    /// has no live alias. `FOR EACH` snapshots the buffer pointer and count at
    /// loop entry, and in-place append only writes *beyond* that snapshot count
    /// without moving existing entries or payloads, so iteration is unaffected.
    /// Reference (`by_ref`) locals are excluded — their slot holds a pointer to
    /// the parent slot, not the buffer — and bulk `append(list, otherList)` is
    /// excluded (the item must be a single element).
    pub(crate) fn try_inplace_append_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        // Container (plan-121-A's shared seam): `name = append(name, …)` on a
        // uniquely-owned `MUT` local. Discharges G1 `by_ref`, G2 the call shape,
        // G3/G4 target and arity, G5/G6 the self-update, G7 the live `FOR EACH`
        // hazard (the grow frees the buffer the loop snapshotted — bug-142),
        // G8 the local exists, and G10 the collection layout.
        let Some(target) =
            self.resolve_inplace_plain_local(name, value, stack_offset, by_ref, "append", 2)
        else {
            return Ok(false);
        };
        let list_type = target.collection_type.clone();
        // G9 — `append` mutates a List.
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&list_type).cloned()
        else {
            return Ok(false);
        };
        // G11: commit only for a statically-known single element of the list's
        // element type. A bulk `append(list, otherList)` has item type ==
        // list_type and falls through to the general (concatenating) path.
        match self.static_item_type(&target.args[1]) {
            Some(item_type) if item_type == element_type => {}
            _ => return Ok(false),
        }
        let item = self.lower_value(&target.args[1])?;
        // Observation boundary: an in-place appended `Float` must be finite
        // (plan-17).
        self.observe_float(&target.args[1], &item)?;
        // Materialize a `d`-native float before the payload spill (plan-01).
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object("inplace_append_item", 8);
        self.emit(abi::store_u64(
            &item.location,
            abi::stack_pointer(),
            item_slot,
        ));
        self.lower_list_append_in_place(
            target.dest.block_slot(),
            item_slot,
            &list_type,
            &element_type,
        )?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// bug-430 (case B): recognize the idiomatic MUT-record update
    /// `rec = WITH rec { coll := collections::append(rec.coll, x) }` on a
    /// uniquely-owned MUT record local whose `coll` is the last inlined field, and
    /// grow it IN PLACE inside the record's existing block instead of rebuilding
    /// the whole record and re-inlining the accumulated buffer (the O(n^2) path).
    /// The local's slot holds the record pointer, so the grow helper repoints it
    /// directly on a realloc — no resource/STATE indirection. `x` may be a single
    /// element (`T`) or a whole list (`List OF T`, concatenation). Any other shape
    /// falls through to the whole-record rebuild. Returns `true` when handled.
    ///
    /// Records are immutable values whose only update form is `WITH` (§4.2); this
    /// does NOT add an `a.field = v` statement — it optimizes the value-preserving
    /// `a = WITH a { … }` reassignment of a uniquely-owned mutable binding, exactly
    /// as `try_inplace_append_assign` optimizes `a = append(a, x)` for a list.
    pub(crate) fn try_inplace_record_field_append(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        // Container (plan-121-A's shared seam): the record-field self-update
        // `rec = WITH rec { field := append(rec.field, …) }`. Discharges G1
        // `by_ref`, G2 the `WithUpdate`-then-`Call` shape, G13 the self-update,
        // G14 the single updated field (a second field would make the elided
        // whole-record rebuild lose that field's new value), G15 the live
        // `FOR EACH` over this field, G17 the last-inlined requirement, G10 the
        // layout, and G3/G4 the call target and arity.
        let Some(target) = self.resolve_inplace_record_field(name, value, by_ref, "append", 2)
        else {
            return Ok(false);
        };
        let field_type_parsed = target.field_type.clone();
        // G9 — `append` mutates a List. (Subsumed by G17, which already refuses a
        // non-`List` field; kept because the lowering needs the element type.)
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&field_type_parsed).cloned()
        else {
            return Ok(false);
        };
        // G18 — the appended-to source must be exactly this same field
        // (self-append). `WITH rec { a := append(rec.b, x) }` writes a's buffer
        // from b's and must rebuild.
        if !self.value_is_record_field(&target.args[0], name, target.field) {
            return Ok(false);
        }
        // G11 — single element (item type == element type) vs bulk concatenation.
        let bulk = match self.static_item_type(&target.args[1]) {
            Some(t) if t == element_type => false,
            Some(t) if t == field_type_parsed => true,
            _ => return Ok(false),
        };
        // G12 — exclude the self-alias `append(field, field)`: the grow frees the
        // old block out from under the RHS copy.
        if self.value_is_record_field(&target.args[1], name, target.field) {
            return Ok(false);
        }

        // Evaluate the appended value and spill it for the grow helper.
        let rhs = self.lower_value(&target.args[1])?;
        self.observe_float(&target.args[1], &rhs)?;
        let rhs = self.materialize_value(rhs)?;
        let rhs_slot = self.allocate_stack_object("inplace_recfield_rhs", 8);
        self.emit(abi::store_u64(
            &rhs.location,
            abi::stack_pointer(),
            rhs_slot,
        ));

        // The local slot holds the record pointer; the grow helper repoints it on
        // a realloc, so the reassignment `rec = …` needs no further store — and
        // therefore no `close_inplace_dest` write-back, unlike the STATE sibling.
        let dest = InPlaceDest::Inlined {
            block_slot: stack_offset,
            field_index: target.field_index,
            write_back: None,
        };
        self.lower_inplace_inlined_list_grow(
            &dest,
            bulk,
            &field_type_parsed,
            &element_type,
            rhs_slot,
        )?;
        self.close_inplace_dest(&dest)?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// Recognize `name = collections::append(name, sublist)` — a *bulk*
    /// list-into-list append — on a uniquely-owned `MUT` list local, and lower it
    /// as an in-place batch grow (plan-25-B B1): the sublist's elements are
    /// appended into `name`'s spare capacity (geometric grow only when the whole
    /// batch does not fit), amortized O(count(sublist)) per call instead of the
    /// value-semantic rebuild that copies the whole accumulated list every call
    /// (the O(n²) `flatten`/`append_batch` path). It is the list-RHS sibling of
    /// [`Self::try_inplace_append_assign`]: that helper commits only for a single
    /// element of the list's *element* type, so a `List OF T` RHS falls through to
    /// here. Soundness is identical to the single-element append (value
    /// semantics plus copy insertion give the buffer no live alias; the grow
    /// writes only beyond
    /// the live count). The `append(name, name)` self-alias — where the grow would
    /// free the RHS out from under the copy — is excluded and takes the value
    /// path. Returns `true` when handled.
    /// plan-86 C2: recognize `name = collections::add(name, item)` on a
    /// uniquely-owned `MUT` `Set` local and insert `item` into the live buffer in
    /// place, skipping `lower_set_add`'s `copy_collection_tight`. That whole-set
    /// copy (and the bucket-index rebuild it forces on the next probe) is what
    /// makes the interpreted set-algebra bodies
    /// (`union`/`toSet`/`intersection`/`difference`/`symmetricDifference`, each a
    /// `FOR EACH … result = add(result, x)` loop) O(n²); in place, each add is
    /// amortized O(1) and the whole op is O(n). The set-add sibling of
    /// [`Self::try_inplace_append_assign`]; soundness is identical (value semantics
    /// give the named local no live alias — every bind/assign copies — and the
    /// `add` is idempotent/order-stable). A live `FOR EACH` over `name` is excluded
    /// (the grow path would free the iterated buffer, bug-142).
    pub(crate) fn try_inplace_set_add_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        if by_ref {
            return Ok(false);
        }
        let NirValue::Call { target, args, .. } = value else {
            return Ok(false);
        };
        if crate::codegen::builtins::native_builtin_target(target) != Some("add") || args.len() != 2
        {
            return Ok(false);
        }
        let NirValue::Local(arg0) = &args[0] else {
            return Ok(false);
        };
        if arg0 != name {
            return Ok(false);
        }
        if self.for_each_iterable_locals.iter().any(|n| n == name) {
            return Ok(false);
        }
        let Some(local) = self.locals.get(name) else {
            return Ok(false);
        };
        let set_type = local.type_.clone();
        let Some(element_type) =
            crate::codegen::engine::types::typed_set_element_type(&set_type).cloned()
        else {
            return Ok(false);
        };
        if crate::codegen::engine::builder::CollectionTypeLayout::from_type(&set_type).is_none() {
            return Ok(false);
        }
        match self.static_item_type(&args[1]) {
            Some(item_type) if item_type == element_type => {}
            _ => return Ok(false),
        }
        let item = self.lower_value(&args[1])?;
        // Observation boundary: an in-place added `Float` must be finite (plan-17).
        self.observe_float(&args[1], &item)?;
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object("inplace_set_add_item", 8);
        self.store_value_at(&item, abi::stack_pointer(), item_slot);
        // The per-element value is a 1-byte `Boolean` TRUE (a Set is a Map to true).
        let true_slot = self.allocate_stack_object("inplace_set_add_true", 8);
        let true_reg = self.allocate_register();
        self.emit(abi::move_immediate(&true_reg, "Boolean", "true"));
        self.emit(abi::store_u64(&true_reg, abi::stack_pointer(), true_slot));
        self.lower_map_set_in_place(
            stack_offset,
            item_slot,
            true_slot,
            &set_type,
            &element_type,
            &ParameterType::Boolean,
            None,
        )?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// plan-86 D1: recognize `name = collections::removeKey(name, k)` on a
    /// uniquely-owned MUT map local and delete the entry IN PLACE via
    /// `lower_map_remove_key_in_place` (entry-table compaction, no alloc/copy),
    /// instead of the out-of-place fresh-map rebuild. Value semantics keep the
    /// named local's buffer un-aliased (copy-on-bind); the `by_ref` and
    /// live-FOR-EACH guards match the set-add path (a compaction shift is observable
    /// to a live iterator, bug-142). Also covers `Set` remove (same lowering).
    pub(crate) fn try_inplace_remove_key_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        if by_ref {
            return Ok(false);
        }
        let NirValue::Call { target, args, .. } = value else {
            return Ok(false);
        };
        if crate::codegen::builtins::native_builtin_target(target) != Some("removeKey")
            || args.len() != 2
        {
            return Ok(false);
        }
        let NirValue::Local(arg0) = &args[0] else {
            return Ok(false);
        };
        if arg0 != name {
            return Ok(false);
        }
        if self.for_each_iterable_locals.iter().any(|n| n == name) {
            return Ok(false);
        }
        let Some(local) = self.locals.get(name) else {
            return Ok(false);
        };
        let map_type = local.type_.clone();
        let Some((key_type, _value_type)) =
            crate::codegen::engine::types::typed_map_type_parts(&map_type)
                .map(|(k, v)| (k.clone(), v.clone()))
        else {
            return Ok(false);
        };
        if crate::codegen::engine::builder::CollectionTypeLayout::from_type(&map_type).is_none() {
            return Ok(false);
        }
        match self.static_item_type(&args[1]) {
            Some(kt) if kt == key_type => {}
            _ => return Ok(false),
        }
        let key = self.lower_value(&args[1])?;
        let key = self.materialize_value(key)?;
        let key_slot = self.allocate_stack_object("inplace_remove_key", 8);
        self.store_value_at(&key, abi::stack_pointer(), key_slot);
        self.lower_map_remove_key_in_place(stack_offset, key_slot, &map_type, &key_type)?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// plan-121-C: recognize `rec = WITH rec { field := removeKey(rec.field, k) }`
    /// on a uniquely-owned `MUT` record local and delete the entry inside the
    /// record's own block.
    ///
    /// This is the plain-local [`Self::try_inplace_remove_key_assign`] with the
    /// container swapped, and that swap is the whole of it: the *lowering* is
    /// unchanged, because `lower_map_remove_key_in_place` only ever **loads** the
    /// slot it is given (every `map_slot` access in it is an `abi::load_u64`), so
    /// handing it the address of the inlined sub-block
    /// ([`Self::open_inplace_inlined_subblock`]) mutates the map where it lies.
    ///
    /// That is only sound because `removeKey` cannot reallocate — it compacts the
    /// entry table and clears `BUCKETS_READY` so the next probe rebuilds the
    /// index. A growing operation on a record field must instead grow the
    /// **record** block; see the helper's own doc for why the two cannot share
    /// this route.
    ///
    /// The whole-record rebuild is elided by returning `true`: the record block is
    /// mutated in place, so `rec = …` has nothing left to store. `G14`
    /// (`updates.len() == 1`) is what makes that sound, and it is enforced in
    /// `resolve_inplace_record_field` — a second updated field would have its new
    /// value dropped.
    pub(crate) fn try_inplace_record_field_remove_key_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        // Container: `G1`, `G2`, `G13`, `G14`, `G15`, `G17`, `G10`, `G3`, `G4`.
        let Some(target) = self.resolve_inplace_record_field(name, value, by_ref, "removeKey", 2)
        else {
            return Ok(false);
        };
        let map_type = target.field_type.clone();
        // `G9` — `removeKey` mutates a Map. The container matcher admits any
        // collection kind (plan-121-C Correction C1), so each arm states its own.
        let Some((key_type, _value_type)) =
            crate::codegen::engine::types::typed_map_type_parts(&map_type)
                .map(|(k, v)| (k.clone(), v.clone()))
        else {
            return Ok(false);
        };
        // `G18` — the map being deleted from must be exactly this same field.
        if !self.value_is_record_field(&target.args[0], name, target.field) {
            return Ok(false);
        }
        // `G11` — the key must be the map's key type.
        match self.static_item_type(&target.args[1]) {
            Some(kt) if kt == key_type => {}
            _ => return Ok(false),
        }

        let dest = InPlaceDest::Inlined {
            block_slot: stack_offset,
            field_index: target.field_index,
            write_back: None,
        };
        let key = self.lower_value(&target.args[1])?;
        let key = self.materialize_value(key)?;
        let key_slot = self.allocate_stack_object("inplace_recfield_remove_key", 8);
        self.store_value_at(&key, abi::stack_pointer(), key_slot);
        // `O-order-4`: the sub-block address is taken AFTER the operand is lowered,
        // because lowering the key can itself call `_mfb_arena_alloc` (a String key
        // materializes), and an allocation may move nothing here but the ordering
        // rule exists so the address is never held across one.
        let map_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_map_remove_key_in_place(map_slot, key_slot, &map_type, &key_type)?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// plan-121-C: `rec = WITH rec { field := removeAt(rec.field, i) }` — the
    /// record-container twin of [`Self::try_inplace_remove_at_assign`].
    ///
    /// `lower_list_remove_at_in_place` only loads the slot it is given and never
    /// reallocates (it shifts down and decrements `count`), so the inlined
    /// sub-block address serves it unchanged.
    ///
    /// **`G24` applies here for exactly the reason it applies to the plain local**,
    /// and this is the first place that rule was inherited rather than
    /// rediscovered: `removeAt` is the only arm in the family that *relocates
    /// existing payloads*, and a recursive element's `get` is not an independent
    /// deep copy. The container does not change that — it is a property of the
    /// element type — so the same predicate gates both.
    pub(crate) fn try_inplace_record_field_remove_at_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        let Some(target) = self.resolve_inplace_record_field(name, value, by_ref, "removeAt", 2)
        else {
            return Ok(false);
        };
        let list_type = target.field_type.clone();
        // `G9` — `removeAt` mutates a List.
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&list_type).cloned()
        else {
            return Ok(false);
        };
        // `G24` — inherited from plan-121-B B7: this arm compacts the data region,
        // and a recursive element type is a pointer-linked graph whose `get` does
        // not produce an independent copy. See `try_inplace_remove_at_assign`.
        if crate::codegen::collection::layout::type_participates_in_cycle(
            &self.type_model,
            &element_type,
        ) {
            return Ok(false);
        }
        // `G18` — removing from exactly this same field.
        if !self.value_is_record_field(&target.args[0], name, target.field) {
            return Ok(false);
        }

        let dest = InPlaceDest::Inlined {
            block_slot: stack_offset,
            field_index: target.field_index,
            write_back: None,
        };
        let index = self.lower_value(&target.args[1])?;
        // `E1` — the index is Integer by construction.
        if index.type_ != ParameterType::Integer {
            return Err(format!(
                "native collection removeAt index must be Integer, got {}",
                index.type_
            ));
        }
        let index = self.materialize_value(index)?;
        let index_slot = self.allocate_stack_object("inplace_recfield_remove_at_index", 8);
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        let buffer_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_list_remove_at_in_place(buffer_slot, index_slot, &list_type, &element_type)?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// plan-121-C: `rec = WITH rec { field := remove(rec.field, v) }` on a
    /// record-held `Set` — the record-container twin of
    /// [`Self::try_inplace_set_remove_assign`].
    ///
    /// A `Set` is a `Map` to `TRUE`, so this reuses
    /// `lower_map_remove_key_in_place` exactly as the plain-local arm does; the
    /// only difference is where the block lives.
    pub(crate) fn try_inplace_record_field_set_remove_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        let Some(target) = self.resolve_inplace_record_field(name, value, by_ref, "remove", 2)
        else {
            return Ok(false);
        };
        let set_type = target.field_type.clone();
        // `G9` — `remove` mutates a Set. `collections::remove` is also the List
        // removal spelling in some shapes, so gating on the FIELD type is what
        // keeps a list from reaching the map lowering.
        let Some(element_type) =
            crate::codegen::engine::types::typed_set_element_type(&set_type).cloned()
        else {
            return Ok(false);
        };
        // `G18` — removing from exactly this same field.
        if !self.value_is_record_field(&target.args[0], name, target.field) {
            return Ok(false);
        }
        // `G11` — the value must be the set's element type.
        match self.static_item_type(&target.args[1]) {
            Some(vt) if vt == element_type => {}
            _ => return Ok(false),
        }

        let dest = InPlaceDest::Inlined {
            block_slot: stack_offset,
            field_index: target.field_index,
            write_back: None,
        };
        let item = self.lower_value(&target.args[1])?;
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object("inplace_recfield_set_remove", 8);
        self.store_value_at(&item, abi::stack_pointer(), item_slot);
        let set_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_map_remove_key_in_place(set_slot, item_slot, &set_type, &element_type)?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// plan-121-C: `rec = WITH rec { field := add(rec.field, v) }` on a
    /// record-held `Set` — the first **growing** operation to reach a record field.
    ///
    /// `set (Record-Fixed) add` is the worst record row in the suite at 5794x
    /// c -O0, and the reason is the pair of copies it does per call: the
    /// out-of-place `add` copies the whole set, then `WITH` rebuilds the whole
    /// record around it.
    ///
    /// Unlike the three non-growing arms, this one cannot simply hand
    /// `lower_map_set_in_place` the inlined sub-block address — that lowering
    /// reallocates, and for an inlined field there is no separate allocation to
    /// replace. It passes an [`InlineGrow`] instead, which redirects both of the
    /// lowering's grow sites to allocate `fieldOffset + mapSize`, copy the record
    /// prefix, publish the new **record** pointer, and free the old record rather
    /// than a pointer into the middle of it (Correction C2).
    ///
    /// `field_off_slot` is read **once, before the grow**: the prefix is copied
    /// verbatim, so a field's block-relative offset is invariant across the
    /// realloc — the same reason `lower_inline_list_append_in_place` hoists it.
    pub(crate) fn try_inplace_record_field_set_add_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        let Some(target) = self.resolve_inplace_record_field(name, value, by_ref, "add", 2) else {
            return Ok(false);
        };
        let set_type = target.field_type.clone();
        // `G9` — `add` mutates a Set.
        let Some(element_type) =
            crate::codegen::engine::types::typed_set_element_type(&set_type).cloned()
        else {
            return Ok(false);
        };
        // `G18` — adding to exactly this same field.
        if !self.value_is_record_field(&target.args[0], name, target.field) {
            return Ok(false);
        }
        // `G11` — the added value must be the set's element type.
        match self.static_item_type(&target.args[1]) {
            Some(vt) if vt == element_type => {}
            _ => return Ok(false),
        }
        // `G12` — exclude the self-alias `add(field, field)`.
        if self.value_is_record_field(&target.args[1], name, target.field) {
            return Ok(false);
        }

        let dest = InPlaceDest::Inlined {
            block_slot: stack_offset,
            field_index: target.field_index,
            write_back: None,
        };
        let item = self.lower_value(&target.args[1])?;
        self.observe_float(&target.args[1], &item)?;
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object("inplace_recfield_add_item", 8);
        self.store_value_at(&item, abi::stack_pointer(), item_slot);
        // A Set is a Map to TRUE.
        let true_slot = self.allocate_stack_object("inplace_recfield_add_true", 8);
        let true_reg = self.allocate_register();
        self.emit(abi::move_immediate(&true_reg, "Boolean", "true"));
        self.emit(abi::store_u64(&true_reg, abi::stack_pointer(), true_slot));

        // The field's block-relative offset, read once and held across the grow.
        let field_off_slot = self.open_inplace_inlined_field_offset(&dest)?;
        let set_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_map_set_in_place(
            set_slot,
            item_slot,
            true_slot,
            &set_type,
            &element_type,
            &ParameterType::Boolean,
            Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                block_slot: stack_offset,
                field_off_slot,
            }),
        )?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// plan-121-C: `rec = WITH rec { field := set(rec.field, k, v) }` — the last
    /// Phase 2 operation, and the one that splits by *element width* rather than by
    /// collection kind.
    ///
    /// `list (Record-Fixed) set` is the plan's headline record row at **1630x**
    /// c -O0, against `list (Record-Dynamic) append` at 0.839x on the same record.
    ///
    /// Three cases, and the middle one is why this arm is not simply "the `add`
    /// arm with a key":
    ///
    /// * **`Map`** — reuses the [`InlineGrow`] route `add` proved, because a new
    ///   key grows the map and therefore the record.
    /// * **`List` of a fixed-width element** — needs no grow at all, so it takes
    ///   the cheaper sub-block route. This is not an optimistic guess:
    ///   `lower_list_set_in_place`'s own kind-2 branch records that "the payload is
    ///   at `index * payloadSize` and is always the same size as its replacement,
    ///   **so the rebuild branch below is unreachable**". Unreachable is the word
    ///   that matters — the rebuild is the branch that would store a fresh block
    ///   into the slot, and a sub-block address must never receive one.
    /// * **`List` of a variable-width element** — declines. The replacement may be
    ///   longer than what it replaces, so the rebuild *is* reachable. That is
    ///   `list (Record-Dynamic) set`, which §"Summary" already assigns to
    ///   plan-121-F along with the rest of the String-representation work.
    pub(crate) fn try_inplace_record_field_set_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        let Some(target) = self.resolve_inplace_record_field(name, value, by_ref, "set", 3) else {
            return Ok(false);
        };
        let collection_type = target.field_type.clone();
        // `G18` — setting into exactly this same field.
        if !self.value_is_record_field(&target.args[0], name, target.field) {
            return Ok(false);
        }
        let dest = InPlaceDest::Inlined {
            block_slot: stack_offset,
            field_index: target.field_index,
            write_back: None,
        };

        // --- List ---------------------------------------------------------
        if let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&collection_type).cloned()
        {
            // Variable-width elements can outgrow the slot they replace, which makes
            // `lower_list_set_in_place`'s rebuild branch reachable — and that branch
            // installs a fresh block, which an inlined sub-block cannot receive.
            // plan-121-F owns this row.
            if crate::codegen::collection::layout::list_element_is_fixed_width(&element_type)
                .is_none()
            {
                return Ok(false);
            }
            let index = self.lower_value(&target.args[1])?;
            if index.type_ != ParameterType::Integer {
                return Err(format!(
                    "native collection set list index must be Integer, got {}",
                    index.type_
                ));
            }
            let index = self.materialize_value(index)?;
            let index_slot = self.allocate_stack_object("inplace_recfield_set_index", 8);
            self.emit(abi::store_u64(
                &index.location,
                abi::stack_pointer(),
                index_slot,
            ));
            let item = self.lower_value(&target.args[2])?;
            // Observation boundary: an in-place replacement `Float` element must be
            // finite (plan-17).
            self.observe_float(&target.args[2], &item)?;
            if item.type_ != element_type {
                return Err(format!(
                    "native collection set list item must be {element_type}, got {}",
                    item.type_
                ));
            }
            let item = self.materialize_value(item)?;
            let item_slot = self.allocate_stack_object("inplace_recfield_set_item", 8);
            self.emit(abi::store_u64(
                &item.location,
                abi::stack_pointer(),
                item_slot,
            ));
            let buffer_slot = self.open_inplace_inlined_subblock(&dest)?;
            self.lower_list_set_in_place(
                buffer_slot,
                index_slot,
                item_slot,
                &collection_type,
                &element_type,
            )?;
            if let Some(local) = self.locals.get_mut(name) {
                local.constant = None;
            }
            return Ok(true);
        }

        // --- Map ----------------------------------------------------------
        let Some((key_type, value_type)) =
            crate::codegen::engine::types::typed_map_type_parts(&collection_type)
                .map(|(k, v)| (k.clone(), v.clone()))
        else {
            return Ok(false);
        };
        let key = self.lower_value(&target.args[1])?;
        // Observation boundary: an in-place `Float` map key must be finite (plan-17).
        self.observe_float(&target.args[1], &key)?;
        if key.type_ != key_type {
            return Err(format!(
                "native collection set map key must be {key_type}, got {}",
                key.type_
            ));
        }
        let key = self.materialize_value(key)?;
        let key_slot = self.allocate_stack_object("inplace_recfield_set_key", 8);
        self.emit(abi::store_u64(
            &key.location,
            abi::stack_pointer(),
            key_slot,
        ));
        let val = self.lower_value(&target.args[2])?;
        // Observation boundary: an in-place `Float` map value must be finite (plan-17).
        self.observe_float(&target.args[2], &val)?;
        if val.type_ != value_type {
            return Err(format!(
                "native collection set map value must be {value_type}, got {}",
                val.type_
            ));
        }
        let val = self.materialize_value(val)?;
        let value_slot = self.allocate_stack_object("inplace_recfield_set_value", 8);
        self.emit(abi::store_u64(
            &val.location,
            abi::stack_pointer(),
            value_slot,
        ));
        let field_off_slot = self.open_inplace_inlined_field_offset(&dest)?;
        let map_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_map_set_in_place(
            map_slot,
            key_slot,
            value_slot,
            &collection_type,
            &key_type,
            &value_type,
            Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                block_slot: stack_offset,
                field_off_slot,
            }),
        )?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    // ----------------------------------------------------------------------
    // plan-121-D Phase 2 — the `RES … STATE` container.
    //
    // Each of these is its record-field twin with the container swapped, and the
    // swap is genuinely the whole of it: the *lowerings* are shared verbatim,
    // because the reallocation split (plan-121-C Correction C2) is a property of
    // the operation, not of who owns the block. A non-growing op is handed the
    // inlined sub-block address; a growing one is handed an `InlineGrow` so its
    // own realloc sites grow the STATE block and repoint it.
    //
    // The ONE thing the record container did not need is `O4`. A record local's
    // block has no second holder, so nothing has to be told it moved. A `STATE`
    // block does — the resource record's `RESOURCE_OFFSET_STATE` slot, which every
    // alias of the handle reads through (§15) — so every arm below finishes with
    // `close_inplace_dest`, and it is a no-op for the other two containers by
    // construction.
    //
    // ORDERING. `open_inplace_state_dest` runs after every gate (`O-order-1`) and
    // *before* the operands are lowered (`O-order-4`), which is the opposite of
    // the record arms and deliberately so: it matches the shipped bug-430 `append`
    // arm, so all seven STATE operations snapshot the STATE pointer at the same
    // point and cannot disagree with each other.
    // ----------------------------------------------------------------------

    /// The `block_slot` of a destination that is known to be inlined.
    ///
    /// Every `open_inplace_state_dest` returns `InPlaceDest::Inlined`, so the
    /// other arm is unreachable rather than a decline — reaching it would mean an
    /// arm matched a container and then asked for a different one, which is a bug
    /// in the arm and not a program to fall back on.
    fn inplace_dest_block_slot(dest: &InPlaceDest) -> Result<usize, String> {
        match dest {
            InPlaceDest::Inlined { block_slot, .. } => Ok(*block_slot),
            _ => Err("native in-place STATE arm resolved a non-inlined destination".to_string()),
        }
    }

    /// plan-121-D: `s.state.field = removeKey(s.state.field, k)` on a STATE-held
    /// `Map` — the STATE twin of [`Self::try_inplace_record_field_remove_key_assign`].
    ///
    /// Non-growing, so it takes the sub-block route unchanged:
    /// `lower_map_remove_key_in_place` only ever *loads* the slot it is given, and
    /// compacts the entry table in place while clearing `BUCKETS_READY` so the next
    /// probe rebuilds the index. Nothing is reallocated, so the write-back at the
    /// end publishes the same pointer it read — correct, and cheap enough not to
    /// be worth a special case.
    ///
    /// `map (State-Dynamic) removeKey` is 326.0x in the element-type overhead
    /// table; the copy this removes is the whole of that.
    pub(crate) fn try_inplace_state_remove_key_assign(
        &mut self,
        resource: &str,
        value: &NirValue,
    ) -> Result<bool, String> {
        // Container: `G2`, `G13`, `G14`, `G16`, `G17`, `G10`, `G3`, `G4`.
        let Some(target) = self.resolve_inplace_state_field(resource, value, "removeKey", 2) else {
            return Ok(false);
        };
        let map_type = target.field_type.clone();
        // `G9` — `removeKey` mutates a Map. The container matcher admits any
        // collection kind, so each arm states its own.
        let Some((key_type, _value_type)) =
            crate::codegen::engine::types::typed_map_type_parts(&map_type)
                .map(|(k, v)| (k.clone(), v.clone()))
        else {
            return Ok(false);
        };
        // `G18` — deleting from exactly this same field.
        if !self.value_is_state_field(&target.args[0], resource, target.field) {
            return Ok(false);
        }
        // `G11` — the key must be the map's key type.
        match self.static_item_type(&target.args[1]) {
            Some(kt) if kt == key_type => {}
            _ => return Ok(false),
        }

        let dest = self.open_inplace_state_dest(resource, target.field_index)?;
        let key = self.lower_value(&target.args[1])?;
        let key = self.materialize_value(key)?;
        let key_slot = self.allocate_stack_object("inplace_state_remove_key", 8);
        self.store_value_at(&key, abi::stack_pointer(), key_slot);
        let map_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_map_remove_key_in_place(map_slot, key_slot, &map_type, &key_type)?;
        // `O4`.
        self.close_inplace_dest(&dest)?;
        Ok(true)
    }

    /// plan-121-D: `s.state.field = add(s.state.field, v)` on a STATE-held `Set`.
    ///
    /// **Growing**, so it takes the [`InlineGrow`] route rather than the sub-block
    /// one — handing `lower_map_set_in_place` a sub-block address would have it
    /// `free()` a pointer into the middle of the live STATE block.
    ///
    /// `set (State-Dynamic) add` is the worst element-type overhead row in the
    /// whole suite at **701.6x**, and it is two copies per call: the out-of-place
    /// `add` copies the set, then the `WITH` rebuilds the STATE record around it.
    pub(crate) fn try_inplace_state_set_add_assign(
        &mut self,
        resource: &str,
        value: &NirValue,
    ) -> Result<bool, String> {
        let Some(target) = self.resolve_inplace_state_field(resource, value, "add", 2) else {
            return Ok(false);
        };
        let set_type = target.field_type.clone();
        // `G9` — `add` mutates a Set.
        let Some(element_type) =
            crate::codegen::engine::types::typed_set_element_type(&set_type).cloned()
        else {
            return Ok(false);
        };
        // `G18` — adding to exactly this same field.
        if !self.value_is_state_field(&target.args[0], resource, target.field) {
            return Ok(false);
        }
        // `G11` — the added value must be the set's element type.
        match self.static_item_type(&target.args[1]) {
            Some(vt) if vt == element_type => {}
            _ => return Ok(false),
        }
        // `G12` — exclude the self-alias `add(field, field)`: the grow frees the
        // old block out from under the RHS copy.
        if self.value_is_state_field(&target.args[1], resource, target.field) {
            return Ok(false);
        }

        let dest = self.open_inplace_state_dest(resource, target.field_index)?;
        let block_slot = Self::inplace_dest_block_slot(&dest)?;
        let item = self.lower_value(&target.args[1])?;
        self.observe_float(&target.args[1], &item)?;
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object("inplace_state_add_item", 8);
        self.store_value_at(&item, abi::stack_pointer(), item_slot);
        // A Set is a Map to TRUE.
        let true_slot = self.allocate_stack_object("inplace_state_add_true", 8);
        let true_reg = self.allocate_register();
        self.emit(abi::move_immediate(&true_reg, "Boolean", "true"));
        self.emit(abi::store_u64(&true_reg, abi::stack_pointer(), true_slot));

        // The field's block-relative offset, read once and held across the grow:
        // the prefix is copied verbatim, so the offset survives a realloc where
        // the sub-block *address* does not.
        let field_off_slot = self.open_inplace_inlined_field_offset(&dest)?;
        let set_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_map_set_in_place(
            set_slot,
            item_slot,
            true_slot,
            &set_type,
            &element_type,
            &ParameterType::Boolean,
            Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                block_slot,
                field_off_slot,
            }),
        )?;
        // `O4` — the grow may have moved the STATE block; publish it.
        self.close_inplace_dest(&dest)?;
        Ok(true)
    }

    /// plan-121-D: `s.state.field = set(s.state.field, k, v)` — the STATE twin of
    /// [`Self::try_inplace_record_field_set_assign`], and it splits the same three
    /// ways, by **element width** rather than by collection kind:
    ///
    /// * **`Map`** — [`InlineGrow`]: a new key grows the map and so the STATE block.
    /// * **`List` of a fixed-width element** — the sub-block route. A fixed-width
    ///   payload is always replaced by one of exactly its own size, so
    ///   `lower_list_set_in_place`'s rebuild branch (the one that would install a
    ///   fresh block) is unreachable, which its own comment records.
    /// * **`List` of a variable-width element** — declines: the replacement can be
    ///   longer, making that branch reachable. `list (State-Dynamic) set` is the
    ///   worst row in the suite at 17742x, and it belongs to plan-121-F along with
    ///   the rest of the String-representation work — not to a wrong fast path here.
    pub(crate) fn try_inplace_state_set_assign(
        &mut self,
        resource: &str,
        value: &NirValue,
    ) -> Result<bool, String> {
        let Some(target) = self.resolve_inplace_state_field(resource, value, "set", 3) else {
            return Ok(false);
        };
        let collection_type = target.field_type.clone();
        // `G18` — setting into exactly this same field.
        if !self.value_is_state_field(&target.args[0], resource, target.field) {
            return Ok(false);
        }

        // --- List ---------------------------------------------------------
        if let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&collection_type).cloned()
        {
            // plan-121-F owns the variable-width row; see the doc comment.
            if crate::codegen::collection::layout::list_element_is_fixed_width(&element_type)
                .is_none()
            {
                return Ok(false);
            }
            let dest = self.open_inplace_state_dest(resource, target.field_index)?;
            let index = self.lower_value(&target.args[1])?;
            if index.type_ != ParameterType::Integer {
                return Err(format!(
                    "native collection set list index must be Integer, got {}",
                    index.type_
                ));
            }
            let index = self.materialize_value(index)?;
            let index_slot = self.allocate_stack_object("inplace_state_set_index", 8);
            self.emit(abi::store_u64(
                &index.location,
                abi::stack_pointer(),
                index_slot,
            ));
            let item = self.lower_value(&target.args[2])?;
            // Observation boundary: an in-place replacement `Float` element must
            // be finite (plan-17).
            self.observe_float(&target.args[2], &item)?;
            if item.type_ != element_type {
                return Err(format!(
                    "native collection set list item must be {element_type}, got {}",
                    item.type_
                ));
            }
            let item = self.materialize_value(item)?;
            let item_slot = self.allocate_stack_object("inplace_state_set_item", 8);
            self.emit(abi::store_u64(
                &item.location,
                abi::stack_pointer(),
                item_slot,
            ));
            let buffer_slot = self.open_inplace_inlined_subblock(&dest)?;
            self.lower_list_set_in_place(
                buffer_slot,
                index_slot,
                item_slot,
                &collection_type,
                &element_type,
            )?;
            // `O4`. Nothing moved on this route, so this republishes the pointer
            // it read -- kept rather than special-cased, so no STATE arm can be
            // read as "this one does not have to publish".
            self.close_inplace_dest(&dest)?;
            return Ok(true);
        }

        // --- Map ----------------------------------------------------------
        let Some((key_type, value_type)) =
            crate::codegen::engine::types::typed_map_type_parts(&collection_type)
                .map(|(k, v)| (k.clone(), v.clone()))
        else {
            return Ok(false);
        };
        let dest = self.open_inplace_state_dest(resource, target.field_index)?;
        let block_slot = Self::inplace_dest_block_slot(&dest)?;
        let key = self.lower_value(&target.args[1])?;
        // Observation boundary: an in-place `Float` map key must be finite (plan-17).
        self.observe_float(&target.args[1], &key)?;
        if key.type_ != key_type {
            return Err(format!(
                "native collection set map key must be {key_type}, got {}",
                key.type_
            ));
        }
        let key = self.materialize_value(key)?;
        let key_slot = self.allocate_stack_object("inplace_state_set_key", 8);
        self.emit(abi::store_u64(
            &key.location,
            abi::stack_pointer(),
            key_slot,
        ));
        let val = self.lower_value(&target.args[2])?;
        // Observation boundary: an in-place `Float` map value must be finite (plan-17).
        self.observe_float(&target.args[2], &val)?;
        if val.type_ != value_type {
            return Err(format!(
                "native collection set map value must be {value_type}, got {}",
                val.type_
            ));
        }
        let val = self.materialize_value(val)?;
        let value_slot = self.allocate_stack_object("inplace_state_set_value", 8);
        self.emit(abi::store_u64(
            &val.location,
            abi::stack_pointer(),
            value_slot,
        ));
        let field_off_slot = self.open_inplace_inlined_field_offset(&dest)?;
        let map_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_map_set_in_place(
            map_slot,
            key_slot,
            value_slot,
            &collection_type,
            &key_type,
            &value_type,
            Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                block_slot,
                field_off_slot,
            }),
        )?;
        // `O4`.
        self.close_inplace_dest(&dest)?;
        Ok(true)
    }

    /// plan-121-D Phase 3: `s.state.field = removeAt(s.state.field, i)` on a
    /// STATE-held `List` — the STATE twin of
    /// [`Self::try_inplace_record_field_remove_at_assign`].
    ///
    /// Non-growing (it shifts down and decrements `count`), so the sub-block
    /// route serves it unchanged.
    ///
    /// **`G24` applies here for the same reason it applies in the other two
    /// containers**, and this is the third place the rule is inherited rather
    /// than rediscovered: `removeAt` is the only operation in the family that
    /// *relocates existing payloads*, and a recursive element's `get` is not an
    /// independent deep copy. That is a property of the ELEMENT TYPE, so the
    /// container is irrelevant to it and the same predicate gates all three.
    pub(crate) fn try_inplace_state_remove_at_assign(
        &mut self,
        resource: &str,
        value: &NirValue,
    ) -> Result<bool, String> {
        let Some(target) = self.resolve_inplace_state_field(resource, value, "removeAt", 2) else {
            return Ok(false);
        };
        let list_type = target.field_type.clone();
        // `G9` — `removeAt` mutates a List.
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&list_type).cloned()
        else {
            return Ok(false);
        };
        // `G24` — inherited from plan-121-B B7 via plan-121-C.
        if crate::codegen::collection::layout::type_participates_in_cycle(
            &self.type_model,
            &element_type,
        ) {
            return Ok(false);
        }
        // `G18` — removing from exactly this same field.
        if !self.value_is_state_field(&target.args[0], resource, target.field) {
            return Ok(false);
        }

        let dest = self.open_inplace_state_dest(resource, target.field_index)?;
        let index = self.lower_value(&target.args[1])?;
        // `E1` — the index is Integer by construction.
        if index.type_ != ParameterType::Integer {
            return Err(format!(
                "native collection removeAt index must be Integer, got {}",
                index.type_
            ));
        }
        let index = self.materialize_value(index)?;
        let index_slot = self.allocate_stack_object("inplace_state_remove_at_index", 8);
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        let buffer_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_list_remove_at_in_place(buffer_slot, index_slot, &list_type, &element_type)?;
        // `O4`.
        self.close_inplace_dest(&dest)?;
        Ok(true)
    }

    /// plan-121-D: `s.state.field = remove(s.state.field, v)` on a STATE-held
    /// `Set`. A `Set` is a `Map` to `TRUE`, so this reuses
    /// `lower_map_remove_key_in_place`; non-growing, so the sub-block route.
    pub(crate) fn try_inplace_state_set_remove_assign(
        &mut self,
        resource: &str,
        value: &NirValue,
    ) -> Result<bool, String> {
        let Some(target) = self.resolve_inplace_state_field(resource, value, "remove", 2) else {
            return Ok(false);
        };
        let set_type = target.field_type.clone();
        // `G9` — `remove` mutates a Set.
        let Some(element_type) =
            crate::codegen::engine::types::typed_set_element_type(&set_type).cloned()
        else {
            return Ok(false);
        };
        // `G18` — removing from exactly this same field.
        if !self.value_is_state_field(&target.args[0], resource, target.field) {
            return Ok(false);
        }
        // `G11` — the removed value must be the set's element type.
        match self.static_item_type(&target.args[1]) {
            Some(vt) if vt == element_type => {}
            _ => return Ok(false),
        }

        let dest = self.open_inplace_state_dest(resource, target.field_index)?;
        let item = self.lower_value(&target.args[1])?;
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object("inplace_state_set_remove", 8);
        self.store_value_at(&item, abi::stack_pointer(), item_slot);
        let set_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_map_remove_key_in_place(set_slot, item_slot, &set_type, &element_type)?;
        // `O4`.
        self.close_inplace_dest(&dest)?;
        Ok(true)
    }

    /// plan-121-D: `s.state.field = insert(s.state.field, i, v)` and its
    /// `prepend` sibling on a STATE-held `List` — the last two operations, and
    /// both **growing**, so both take the [`InlineGrow`] route.
    ///
    /// One arm serves both spellings, because a `prepend` is `SpliceAt::Front`.
    /// The codegen tests still assert the two independently: a regression confined
    /// to the `prepend` wrapper would otherwise hide behind `insert`.
    ///
    /// Bounds are the lowering's own (`0 <= index <= count`, plan-121-B B6); the
    /// container does not change them.
    fn try_inplace_state_splice_assign(
        &mut self,
        resource: &str,
        value: &NirValue,
        builtin: &str,
        arity: usize,
    ) -> Result<bool, String> {
        let Some(target) = self.resolve_inplace_state_field(resource, value, builtin, arity) else {
            return Ok(false);
        };
        let list_type = target.field_type.clone();
        // `G9` — both operations mutate a List.
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&list_type).cloned()
        else {
            return Ok(false);
        };
        // `G18` — splicing into exactly this same field.
        if !self.value_is_state_field(&target.args[0], resource, target.field) {
            return Ok(false);
        }
        let rhs_index = arity - 1;
        // `G12` — exclude the self-alias: the grow frees the old block out from
        // under the right-hand side copy.
        if self.value_is_state_field(&target.args[rhs_index], resource, target.field) {
            return Ok(false);
        }
        // `G11` — the spliced-in value must be a single element.
        match self.static_item_type(&target.args[rhs_index]) {
            Some(vt) if vt == element_type => {}
            _ => return Ok(false),
        }

        let dest = self.open_inplace_state_dest(resource, target.field_index)?;
        let block_slot = Self::inplace_dest_block_slot(&dest)?;
        // `insert` carries an index; `prepend` is `SpliceAt::Front`.
        let at = if arity == 3 {
            let index = self.lower_value(&target.args[1])?;
            if index.type_ != ParameterType::Integer {
                return Err(format!(
                    "native collection insert index must be Integer, got {}",
                    index.type_
                ));
            }
            let index = self.materialize_value(index)?;
            let index_slot = self.allocate_stack_object("inplace_state_splice_index", 8);
            self.emit(abi::store_u64(
                &index.location,
                abi::stack_pointer(),
                index_slot,
            ));
            crate::codegen::collection::list::list_mutate::SpliceAt::At(index_slot)
        } else {
            crate::codegen::collection::list::list_mutate::SpliceAt::Front
        };
        let item = self.lower_value(&target.args[rhs_index])?;
        self.observe_float(&target.args[rhs_index], &item)?;
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object("inplace_state_splice_item", 8);
        self.store_value_at(&item, abi::stack_pointer(), item_slot);

        let field_off_slot = self.open_inplace_inlined_field_offset(&dest)?;
        let buffer_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_list_splice_in_place(
            buffer_slot,
            at,
            item_slot,
            &list_type,
            &element_type,
            Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                block_slot,
                field_off_slot,
            }),
        )?;
        // `O4` — the grow may have moved the STATE block; publish it.
        self.close_inplace_dest(&dest)?;
        Ok(true)
    }

    /// `s.state.field = insert(s.state.field, i, v)`.
    pub(crate) fn try_inplace_state_insert_assign(
        &mut self,
        resource: &str,
        value: &NirValue,
    ) -> Result<bool, String> {
        self.try_inplace_state_splice_assign(resource, value, "insert", 3)
    }

    /// `s.state.field = prepend(s.state.field, v)`.
    pub(crate) fn try_inplace_state_prepend_assign(
        &mut self,
        resource: &str,
        value: &NirValue,
    ) -> Result<bool, String> {
        self.try_inplace_state_splice_assign(resource, value, "prepend", 2)
    }

    /// plan-121-C: `rec = WITH rec { field := insert(rec.field, i, v) }` and its
    /// `prepend` sibling — the last two Phase 3 operations, and both **growing**,
    /// so both take the [`InlineGrow`] route rather than the sub-block one.
    ///
    /// `lower_list_splice_in_place` serves both (a `prepend` is
    /// `SpliceAt::Front`), so a single arm shape covers them with the spelling as
    /// the only difference. It reallocates when the buffer is full, which is
    /// exactly why the sub-block address the three non-growing arms use would be
    /// wrong here: `emit_free_pre_grow_buffer` would release a pointer into the
    /// middle of the record block.
    ///
    /// Bounds are the lowering's own (`0 <= index <= count`, plan-121-B B6); the
    /// container does not change them.
    fn try_inplace_record_field_splice_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
        builtin: &str,
        arity: usize,
    ) -> Result<bool, String> {
        let Some(target) = self.resolve_inplace_record_field(name, value, by_ref, builtin, arity)
        else {
            return Ok(false);
        };
        let list_type = target.field_type.clone();
        // `G9` — both operations mutate a List.
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&list_type).cloned()
        else {
            return Ok(false);
        };
        // `G18` — splicing into exactly this same field.
        if !self.value_is_record_field(&target.args[0], name, target.field) {
            return Ok(false);
        }
        // `G12` — exclude the self-alias: the grow frees the old block out from
        // under the right-hand side copy.
        let rhs_index = arity - 1;
        if self.value_is_record_field(&target.args[rhs_index], name, target.field) {
            return Ok(false);
        }
        // `G11` — the spliced-in value must be a single element.
        match self.static_item_type(&target.args[rhs_index]) {
            Some(vt) if vt == element_type => {}
            _ => return Ok(false),
        }

        let dest = InPlaceDest::Inlined {
            block_slot: stack_offset,
            field_index: target.field_index,
            write_back: None,
        };
        // `insert` carries an index; `prepend` is `SpliceAt::Front`.
        let at = if arity == 3 {
            let index = self.lower_value(&target.args[1])?;
            if index.type_ != ParameterType::Integer {
                return Err(format!(
                    "native collection insert index must be Integer, got {}",
                    index.type_
                ));
            }
            let index = self.materialize_value(index)?;
            let index_slot = self.allocate_stack_object("inplace_recfield_splice_index", 8);
            self.emit(abi::store_u64(
                &index.location,
                abi::stack_pointer(),
                index_slot,
            ));
            crate::codegen::collection::list::list_mutate::SpliceAt::At(index_slot)
        } else {
            crate::codegen::collection::list::list_mutate::SpliceAt::Front
        };
        let item = self.lower_value(&target.args[rhs_index])?;
        self.observe_float(&target.args[rhs_index], &item)?;
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object("inplace_recfield_splice_item", 8);
        self.store_value_at(&item, abi::stack_pointer(), item_slot);

        let field_off_slot = self.open_inplace_inlined_field_offset(&dest)?;
        let buffer_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_list_splice_in_place(
            buffer_slot,
            at,
            item_slot,
            &list_type,
            &element_type,
            Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                block_slot: stack_offset,
                field_off_slot,
            }),
        )?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// `rec = WITH rec { field := insert(rec.field, i, v) }`.
    pub(crate) fn try_inplace_record_field_insert_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        self.try_inplace_record_field_splice_assign(name, value, stack_offset, by_ref, "insert", 3)
    }

    /// `rec = WITH rec { field := prepend(rec.field, v) }`.
    pub(crate) fn try_inplace_record_field_prepend_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        self.try_inplace_record_field_splice_assign(name, value, stack_offset, by_ref, "prepend", 2)
    }

    pub(crate) fn try_inplace_bulk_append_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        if by_ref {
            return Ok(false);
        }
        let NirValue::Call { target, args, .. } = value else {
            return Ok(false);
        };
        if crate::codegen::builtins::native_builtin_target(target) != Some("append")
            || args.len() != 2
        {
            return Ok(false);
        }
        let NirValue::Local(arg0) = &args[0] else {
            return Ok(false);
        };
        if arg0 != name {
            return Ok(false);
        }
        // Exclude the self-alias `append(name, name)`: the grow path frees the old
        // buffer, so a RHS pointing at the same buffer would read freed memory. The
        // value path rebuilds correctly from both operands read up front.
        if let NirValue::Local(arg1) = &args[1] {
            if arg1 == name {
                return Ok(false);
            }
        }
        // Same live `FOR EACH` iterable hazard as the single-element append: the
        // grow path frees the snapshot buffer out from under the loop (bug-142).
        if self.for_each_iterable_locals.iter().any(|n| n == name) {
            return Ok(false);
        }
        let Some(local) = self.locals.get(name) else {
            return Ok(false);
        };
        let list_type = local.type_.clone();
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&list_type).cloned()
        else {
            return Ok(false);
        };
        if crate::codegen::engine::builder::CollectionTypeLayout::from_type(&list_type).is_none() {
            return Ok(false);
        }
        // Commit only for a statically-known RHS of the *list* type (not the
        // element type — that is the single-element fast path). A RHS whose static
        // type is unknown (a general call result) falls through to the value path.
        match self.static_item_type(&args[1]) {
            Some(item_type) if item_type == list_type => {}
            _ => return Ok(false),
        }
        let rhs = self.lower_value(&args[1])?;
        if rhs.type_ != list_type {
            return Err(format!(
                "native bulk append sublist must be {list_type}, got {}",
                rhs.type_
            ));
        }
        let rhs_slot = self.allocate_stack_object("inplace_bulk_append_rhs", 8);
        self.emit(abi::store_u64(
            &rhs.location,
            abi::stack_pointer(),
            rhs_slot,
        ));
        self.lower_list_bulk_append_in_place(stack_offset, rhs_slot, &list_type, &element_type)?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// Recognize `name = collections::set(name, index, item)` on a uniquely-owned
    /// `MUT` **list** local and lower it as an in-place overwrite (plan-02 §4.1).
    /// When the replacement payload fits the target slot (`newLen <= oldLen`, the
    /// fixed-width and same-size record cases always do) the value bytes are
    /// overwritten at the entry's `valueOffset` and `valueLength` patched — no
    /// allocation, no copy. Otherwise it falls back to the rebuild (remove+insert)
    /// path, which is always correct (D1). Returns `true` when handled.
    ///
    /// Soundness mirrors `try_inplace_append_assign`: value semantics + copy
    /// insertion guarantee the buffer is unaliased, and `by_ref` locals are
    /// excluded. Unlike append, an overwrite is observable to an enclosing
    /// `FOR EACH` over the same binding, so that case is excluded
    /// (`for_each_iterable_locals`). The map overload (Phase 3) is the same shape:
    /// scan for the key, overwrite the value in place when it fits, append a new
    /// entry into spare slot/data headroom otherwise (geometric grow when full).
    pub(crate) fn try_inplace_set_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        if by_ref {
            return Ok(false);
        }
        let NirValue::Call { target, args, .. } = value else {
            return Ok(false);
        };
        if crate::codegen::builtins::native_builtin_target(target) != Some("set") || args.len() != 3
        {
            return Ok(false);
        }
        let NirValue::Local(arg0) = &args[0] else {
            return Ok(false);
        };
        if arg0 != name {
            return Ok(false);
        }
        if self.for_each_iterable_locals.iter().any(|n| n == name) {
            return Ok(false);
        }
        let Some(local) = self.locals.get(name) else {
            return Ok(false);
        };
        let collection_type = local.type_.clone();
        if crate::codegen::engine::builder::CollectionTypeLayout::from_type(&collection_type)
            .is_none()
        {
            return Ok(false);
        }
        if let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&collection_type).cloned()
        {
            // The list `set` item is always a single element of type `T`
            // (source-checker-enforced), so — unlike append's bulk-vs-single gate — no
            // static element-type check is needed; the post-lowering `item.type_`
            // check catches any mismatch.
            let index = self.lower_value(&args[1])?;
            if index.type_ != ParameterType::Integer {
                return Err(format!(
                    "native collection set list index must be Integer, got {}",
                    index.type_
                ));
            }
            let index_slot = self.allocate_stack_object("inplace_set_index", 8);
            self.emit(abi::store_u64(
                &index.location,
                abi::stack_pointer(),
                index_slot,
            ));
            let item = self.lower_value(&args[2])?;
            // Observation boundary: an in-place replacement `Float` element must
            // be finite (plan-17).
            self.observe_float(&args[2], &item)?;
            if item.type_ != element_type {
                return Err(format!(
                    "native collection set list item must be {element_type}, got {}",
                    item.type_
                ));
            }
            let item = self.materialize_value(item)?;
            let item_slot = self.allocate_stack_object("inplace_set_item", 8);
            self.emit(abi::store_u64(
                &item.location,
                abi::stack_pointer(),
                item_slot,
            ));
            self.lower_list_set_in_place(
                stack_offset,
                index_slot,
                item_slot,
                &collection_type,
                &element_type,
            )?;
            if let Some(local) = self.locals.get_mut(name) {
                local.constant = None;
            }
            return Ok(true);
        }
        if let Some((key_type, value_type)) =
            crate::codegen::engine::types::typed_map_type_parts(&collection_type)
                .map(|(k, v)| (k.clone(), v.clone()))
        {
            let key = self.lower_value(&args[1])?;
            // Observation boundary: an in-place `Float` map key must be finite
            // (plan-17).
            self.observe_float(&args[1], &key)?;
            if key.type_ != key_type {
                return Err(format!(
                    "native collection set map key must be {key_type}, got {}",
                    key.type_
                ));
            }
            let key = self.materialize_value(key)?;
            let key_slot = self.allocate_stack_object("inplace_set_key", 8);
            self.emit(abi::store_u64(
                &key.location,
                abi::stack_pointer(),
                key_slot,
            ));
            let val = self.lower_value(&args[2])?;
            // Observation boundary: an in-place `Float` map value must be finite
            // (plan-17).
            self.observe_float(&args[2], &val)?;
            if val.type_ != value_type {
                return Err(format!(
                    "native collection set map value must be {value_type}, got {}",
                    val.type_
                ));
            }
            let val = self.materialize_value(val)?;
            let value_slot = self.allocate_stack_object("inplace_set_value", 8);
            self.emit(abi::store_u64(
                &val.location,
                abi::stack_pointer(),
                value_slot,
            ));
            self.lower_map_set_in_place(
                stack_offset,
                key_slot,
                value_slot,
                &collection_type,
                &key_type,
                &value_type,
                None,
            )?;
            if let Some(local) = self.locals.get_mut(name) {
                local.constant = None;
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Recognize `name = collections::prepend(name, item)` on a uniquely-owned
    /// `MUT` list local and lower it as an in-place prepend (plan-02 §3): shift the
    /// live lookup entries right by one and write the new entry at index 0, with the
    /// new element's payload appended to the spare data tail — no per-op allocation
    /// (geometric grow only when full). Still O(n) per op (the entry shift), but it
    /// drops the alloc + double-copy the value-semantic insert did each call. Like
    /// `set`, the entry shift is observable to an enclosing `FOR EACH` over the same
    /// binding, so that case is excluded. Returns `true` when handled.
    pub(crate) fn try_inplace_prepend_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        if by_ref {
            return Ok(false);
        }
        let NirValue::Call { target, args, .. } = value else {
            return Ok(false);
        };
        if crate::codegen::builtins::native_builtin_target(target) != Some("prepend")
            || args.len() != 2
        {
            return Ok(false);
        }
        let NirValue::Local(arg0) = &args[0] else {
            return Ok(false);
        };
        if arg0 != name {
            return Ok(false);
        }
        if self.for_each_iterable_locals.iter().any(|n| n == name) {
            return Ok(false);
        }
        let Some(local) = self.locals.get(name) else {
            return Ok(false);
        };
        let list_type = local.type_.clone();
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&list_type).cloned()
        else {
            return Ok(false);
        };
        if crate::codegen::engine::builder::CollectionTypeLayout::from_type(&list_type).is_none() {
            return Ok(false);
        }
        // `prepend` always takes a single element of the list element type
        // (a bulk form is rejected in `lower_collection_prepend`), so no static
        // gate is needed; the post-lowering check catches any mismatch.
        let item = self.lower_value(&args[1])?;
        // Observation boundary: an in-place prepended `Float` must be finite
        // (plan-17).
        self.observe_float(&args[1], &item)?;
        if item.type_ != element_type {
            return Err(format!(
                "native collection prepend item must be {element_type}, got {}",
                item.type_
            ));
        }
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object("inplace_prepend_item", 8);
        self.emit(abi::store_u64(
            &item.location,
            abi::stack_pointer(),
            item_slot,
        ));
        self.lower_list_prepend_in_place(stack_offset, item_slot, &list_type, &element_type)?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// Recognize `name = name & x` (and the left-associated chain
    /// `name = name & a & b …`) on a uniquely-owned `MUT` `String` local and lower
    /// it as an in-place self-append (plan-02 §4.1, the string sibling of
    /// `try_inplace_append_assign`). The grown buffer carries geometric capacity
    /// headroom tracked in a frame-local shadow slot, so each append writes the
    /// operand's bytes into the spare tail and bumps the length — amortized O(1) —
    /// instead of `lower_string_concat` allocating a fresh tight buffer every time.
    /// The shadow never escapes: any copy/return/transfer reads only `len` bytes,
    /// freezing the value to the canonical tight `[len][bytes][NUL]` form (D9). A
    /// `String` can never be a `FOR EACH` iterable, so this needs no iterator gate.
    /// Returns `true` when handled.
    pub(crate) fn try_inplace_concat_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        if by_ref {
            return Ok(false);
        }
        // Only fire for a name we pre-allocated a capacity shadow for (a self-append
        // target discovered by the prescan); the shadow is reset on every other
        // bind/assign so it always reflects the live buffer's spare bytes.
        let Some(&shadow_slot) = self.string_capacity_slots.get(name) else {
            return Ok(false);
        };
        let Some(operands) = string_self_append_operands(value, name) else {
            return Ok(false);
        };
        // If the target reappears in a later operand (`s = s & x & s`), lowering
        // operands in sequence would re-read the already-mutated buffer and append
        // the extended value (bug-143). Fall back to the out-of-place concat path,
        // which reads every operand from the original value.
        if operands
            .iter()
            .any(|operand| nir_value_reads_local(operand, name))
        {
            return Ok(false);
        }
        for operand in operands {
            self.lower_string_self_append_one(stack_offset, shadow_slot, operand)?;
        }
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// Append one `String` operand's bytes to the grown self-append buffer whose
    /// pointer lives at `name_slot`, using/maintaining the spare-capacity shadow at
    /// `shadow_slot`. Writes into the spare tail when `rlen <= spare`; otherwise
    /// allocates a geometric-headroom buffer, copies the current bytes + the
    /// operand, and repoints `name_slot`. Mirrors `lower_list_append_in_place`.
    fn lower_string_self_append_one(
        &mut self,
        name_slot: usize,
        shadow_slot: usize,
        operand: &NirValue,
    ) -> Result<(), String> {
        let right = self.lower_value(operand)?;
        if right.type_ != ParameterType::String {
            return Err(format!(
                "native string self-append operand must be String, got {}",
                right.type_
            ));
        }
        let right_slot = self.allocate_stack_object("concat_self_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        let newlen_slot = self.allocate_stack_object("concat_self_newlen", 8);
        let newcap_slot = self.allocate_stack_object("concat_self_newcap", 8);
        let newbuf_slot = self.allocate_stack_object("concat_self_newbuf", 8);
        // bug-77: the old arena-owned buffer must be freed on regrow; capture
        // its true alloc size (payload capacity + 9) before the alloc clobbers
        // the registers holding it.
        let oldsize_slot = self.allocate_stack_object("concat_self_oldsize", 8);

        let ptr = self.temporary_vreg();
        let len = self.temporary_vreg();
        let right_ptr = self.temporary_vreg();
        let rlen = self.temporary_vreg();
        let newlen = self.temporary_vreg();
        let spare = self.temporary_vreg();
        let newcap = self.temporary_vreg();
        let step_scratch = self.temporary_vreg();
        let zero = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let oldsize = self.temporary_vreg();

        let regrow = self.label("concat_self_regrow");
        let write = self.label("concat_self_write");
        let alloc_ok = self.label("concat_self_alloc_ok");
        let cap_keep = self.label("concat_self_cap_keep");
        let done = self.label("concat_self_done");

        // newlen = len + rlen; decide in-place vs regrow on rlen vs spare.
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64(&len, &ptr, 0)); // len
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(&rlen, &right_ptr, 0)); // rlen
        self.emit(abi::add_registers(&newlen, &len, &rlen));
        self.emit(abi::store_u64(&newlen, abi::stack_pointer(), newlen_slot));
        self.emit(abi::load_u64(&spare, abi::stack_pointer(), shadow_slot)); // spare
        self.emit(abi::compare_registers(&rlen, &spare));
        self.emit(abi::branch_hi(&regrow)); // rlen > spare → regrow
        self.emit(abi::branch(&write));

        // --- Regrow: alloc newcap_payload + 9; copy old + operand; install. ---
        self.emit(abi::label(&regrow));
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64(&len, &ptr, 0)); // len
        self.emit(abi::load_u64(&spare, abi::stack_pointer(), shadow_slot)); // spare
        self.emit(abi::add_registers(&right_ptr, &len, &spare)); // current payload capacity
                                                                 // bug-77: oldsize = payload_capacity + 9 ([len:8][bytes][NUL]). The
                                                                 // headroom is tracked only in the shadow slot, so a tight len+9 free
                                                                 // would under-free; capture the real size now before it is clobbered.
        self.emit(abi::add_immediate(&oldsize, &right_ptr, 9));
        self.emit(abi::store_u64(&oldsize, abi::stack_pointer(), oldsize_slot));
        self.emit_geometric_step(
            &right_ptr,
            &newcap,
            &step_scratch,
            COLLECTION_GROW_DATA_INIT,
            COLLECTION_GROW_DATA_TAPER,
            "concat_self_step",
        );
        // newcap_payload = max(step, newlen).
        self.emit(abi::load_u64(&newlen, abi::stack_pointer(), newlen_slot));
        self.emit(abi::compare_registers(&newcap, &newlen));
        self.emit(abi::branch_hi(&cap_keep));
        self.emit(abi::branch_eq(&cap_keep));
        self.emit(abi::move_register(&newcap, &newlen));
        self.emit(abi::label(&cap_keep));
        self.emit(abi::store_u64(&newcap, abi::stack_pointer(), newcap_slot));
        // alloc size = 8 (len word) + newcap_payload + 1 (NUL).
        // plan-71-C Family-1a: alloc size is arg 0 → `%arg0`, not return_register().
        self.emit(abi::add_immediate(abi::c_arg(0), &newcap, 9));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            newbuf_slot,
        ));
        // newbuf[0] = newlen.
        self.emit(abi::load_u64(&newlen, abi::stack_pointer(), newlen_slot));
        self.emit(abi::store_u64(&newlen, abi::mfb_return(1), 0));
        // Copy the current bytes (len) to newbuf+8.
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64(&len, &ptr, 0)); // len
        self.emit(abi::add_immediate(&ptr, &ptr, 8)); // old data
        self.emit(abi::add_immediate(&dst, abi::mfb_return(1), 8)); // new data
        self.emit_copy_bytes(&dst, &ptr, &len, "concat_self_old");
        // Copy the operand bytes (rlen) to newbuf+8+len. dst now points at +8+len.
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(&rlen, &right_ptr, 0)); // rlen
        self.emit(abi::add_immediate(&right_ptr, &right_ptr, 8)); // operand data
        self.emit_copy_bytes(&dst, &right_ptr, &rlen, "concat_self_new");
        // NUL terminator at newbuf+8+newlen.
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::store_u8(&zero, &dst, 0));
        // bug-77: free the old buffer before installing the new pointer. The
        // old buffer pointer is still live at name_slot (overwritten just
        // below) and its size is in oldsize_slot; the new buffer is already
        // spilled in newbuf_slot, so it survives this call. arena_free clobbers
        // all caller-saved registers. This free runs exactly once per regrow.
        // plan-71-C Family-1a: ptr is arg 0 of arena-free → `%arg0`.
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            name_slot,
        ));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            oldsize_slot,
        ));
        self.emit_arena_free_call();
        // Install new buffer; spare = newcap_payload - newlen.
        self.emit(abi::load_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            newbuf_slot,
        ));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            name_slot,
        ));
        self.emit(abi::load_u64(&newcap, abi::stack_pointer(), newcap_slot));
        self.emit(abi::load_u64(&newlen, abi::stack_pointer(), newlen_slot));
        self.emit(abi::subtract_registers(&newcap, &newcap, &newlen));
        self.emit(abi::store_u64(&newcap, abi::stack_pointer(), shadow_slot));
        self.emit(abi::branch(&done));

        // --- In place: write operand bytes into the spare tail. ---
        self.emit(abi::label(&write));
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64(&len, &ptr, 0)); // len
        self.emit(abi::add_immediate(&dst, &ptr, 8));
        self.emit(abi::add_registers(&dst, &dst, &len)); // dst = ptr+8+len
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(&rlen, &right_ptr, 0)); // rlen
        self.emit(abi::add_immediate(&right_ptr, &right_ptr, 8)); // operand data
        self.emit_copy_bytes(&dst, &right_ptr, &rlen, "concat_self_inplace");
        // NUL after the new end; ptr[0] = newlen; spare -= rlen.
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::store_u8(&zero, &dst, 0));
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64(&newlen, abi::stack_pointer(), newlen_slot));
        self.emit(abi::store_u64(&newlen, &ptr, 0));
        self.emit(abi::load_u64(&spare, abi::stack_pointer(), shadow_slot));
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(&rlen, &right_ptr, 0)); // rlen
        self.emit(abi::subtract_registers(&spare, &spare, &rlen));
        self.emit(abi::store_u64(&spare, abi::stack_pointer(), shadow_slot));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// plan-121-B: recognize `name = collections::removeAt(name, i)` on a
    /// uniquely-owned `MUT` list local and close the hole in the live buffer
    /// instead of allocating a fresh tight block and copying every survivor into
    /// it. The shift stays O(n) — that is `removeAt`'s defined cost and C pays it
    /// too — but the per-call allocate + copy + free disappears, which spike 3
    /// measured at 36x the data movement.
    ///
    /// **This arm's `FOR EACH` gate is stricter than `append`'s, and the
    /// difference is the whole reason the gate inventory exists.** `append`
    /// writes only *beyond* the count a live loop snapshotted at entry, so it may
    /// proceed until it reallocs. `removeAt` shifts survivors *down*, rewriting
    /// entries below that snapshot — which the loop can observe as a skipped or
    /// repeated element even though no buffer was freed. So it declines on any
    /// live `FOR EACH` over this binding, unconditionally
    /// (`planning/plan-121-gate-inventory.md`, "the `removeAt` asymmetry").
    pub(crate) fn try_inplace_remove_at_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        // Container (plan-121-A's seam). G7 — the live-`FOR EACH` decline — is
        // enforced here for the shift reason above, not merely the realloc one.
        let Some(target) =
            self.resolve_inplace_plain_local(name, value, stack_offset, by_ref, "removeAt", 2)
        else {
            return Ok(false);
        };
        let list_type = target.collection_type.clone();
        // G9 — `removeAt` mutates a List.
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&list_type).cloned()
        else {
            return Ok(false);
        };
        // G24 — decline for a RECURSIVE element type. Unlike every other arm in
        // this family, `removeAt` compacts the data region: it moves surviving
        // payloads *down* inside the live buffer. That is safe only while nothing
        // else refers into those payloads.
        //
        // `type_participates_in_cycle` is exactly the class where something does.
        // Its own doc records why: a recursive value is a **pointer-linked graph**
        // that inline copy codegen cannot reproduce, so it needs a per-type runtime
        // copy function — which means an ordinary `collections::get` of such an
        // element does not produce an independent deep copy the way a `String`,
        // record or nested-list element does. Relocating the payload under a value
        // already read out of the list leaves that value reading moved bytes.
        //
        // Measured, on `List OF Node` where `ElementNode.children` is `List OF Node`
        // (the shape `tests/rt_recursive_thread_transfer.rs` builds):
        // `get(xs, 0)` then `xs = removeAt(xs, 0)` then `MATCH` on the value read
        // fell to `CASE ELSE` for every element whose removal actually moved bytes,
        // and was correct only for the last one — where `count == 1` makes the
        // shift length zero. Dropping `children` from the record (making the union
        // non-recursive) makes the same program pass, which is what isolates the
        // predicate.
        //
        // The copying path is unaffected because it never disturbs the original
        // buffer, so declining restores exactly the previous behavior. `insert` and
        // `prepend` need no such gate: they place the new payload at the data tail
        // and shift only the 40-byte lookup entries, so no existing payload moves.
        if crate::codegen::collection::layout::type_participates_in_cycle(
            &self.type_model,
            &element_type,
        ) {
            return Ok(false);
        }
        let index = self.lower_value(&target.args[1])?;
        // E1 — the index is Integer by construction; a mismatch is a codegen
        // invariant violation, not a program to decline.
        if index.type_ != ParameterType::Integer {
            return Err(format!(
                "native collection removeAt index must be Integer, got {}",
                index.type_
            ));
        }
        let index = self.materialize_value(index)?;
        let index_slot = self.allocate_stack_object("inplace_remove_at_index", 8);
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        self.lower_list_remove_at_in_place(
            target.dest.block_slot(),
            index_slot,
            &list_type,
            &element_type,
        )?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// plan-121-B: recognize `name = collections::remove(name, v)` on a
    /// uniquely-owned `MUT` `Set` local and delete the entry in place.
    ///
    /// A `Set` is a `Map` to `TRUE` and `collections::remove` already reuses
    /// `lower_map_remove_key` out of place, so the in-place form is the same
    /// reuse of [`Self::lower_map_remove_key_in_place`] — entry-table compaction
    /// plus `BUCKETS_READY = 0` so the next probe rebuilds the index. That the
    /// arm did not exist is the entire gap: `set (Fixed) remove` measured **677x
    /// c -O0** while the Map sibling with the identical lowering was 32x.
    ///
    /// Gates match the Map `removeKey` arm exactly, including the live-`FOR EACH`
    /// decline: the compaction shift moves entries below a live iterator's
    /// snapshot, which it can observe (bug-142's non-freeing twin).
    pub(crate) fn try_inplace_set_remove_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        let Some(target) =
            self.resolve_inplace_plain_local(name, value, stack_offset, by_ref, "remove", 2)
        else {
            return Ok(false);
        };
        let set_type = target.collection_type.clone();
        // G9 — `remove` mutates a Set. (`removeKey` handles the Map spelling.)
        let Some(element_type) =
            crate::codegen::engine::types::typed_set_element_type(&set_type).cloned()
        else {
            return Ok(false);
        };
        // G11 — the removed value must be statically the set's element type.
        match self.static_item_type(&target.args[1]) {
            Some(t) if t == element_type => {}
            _ => return Ok(false),
        }
        let item = self.lower_value(&target.args[1])?;
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object("inplace_set_remove_item", 8);
        self.store_value_at(&item, abi::stack_pointer(), item_slot);
        self.lower_map_remove_key_in_place(
            target.dest.block_slot(),
            item_slot,
            &set_type,
            &element_type,
        )?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// plan-121-B: recognize `name = collections::insert(name, i, v)` on a
    /// uniquely-owned `MUT` list local and splice into the live buffer instead of
    /// allocating a fresh block and copying the whole list per call. The shift
    /// stays O(N) — that is `insert`'s defined cost and C pays it too — but the
    /// per-call allocate + copy + free disappears, which spike 3 measured at 36x
    /// the data movement.
    ///
    /// **Declines under a live `FOR EACH`, where `append` would not.** The shift
    /// moves entries up from an index *inside* the range the loop snapshotted at
    /// entry, so the loop can observe it; `append` writes only beyond that count.
    /// See `planning/plan-121-gate-inventory.md`, "the `removeAt` asymmetry",
    /// which covers `insert` for the same reason.
    pub(crate) fn try_inplace_insert_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        // Container (plan-121-A's seam). G7 — the live-`FOR EACH` decline — is
        // enforced here for the shift reason above, not merely the realloc one.
        let Some(target) =
            self.resolve_inplace_plain_local(name, value, stack_offset, by_ref, "insert", 3)
        else {
            return Ok(false);
        };
        let list_type = target.collection_type.clone();
        // G9 — `insert` mutates a List.
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&list_type).cloned()
        else {
            return Ok(false);
        };
        // Source order, matching the out-of-place lowering: index then item.
        let index = self.lower_value(&target.args[1])?;
        // E1 — the index is Integer by construction; a mismatch is a codegen
        // invariant violation, not a program to decline.
        if index.type_ != ParameterType::Integer {
            return Err(format!(
                "native collection insert index must be Integer, got {}",
                index.type_
            ));
        }
        let index = self.materialize_value(index)?;
        let index_slot = self.allocate_stack_object("inplace_insert_index", 8);
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        let item = self.lower_value(&target.args[2])?;
        // Observation boundary: an in-place spliced `Float` must be finite
        // (plan-17).
        self.observe_float(&target.args[2], &item)?;
        // E2 — like `set`/`prepend`, the source checker enforces the item type and
        // this post-lowering check catches any mismatch as a hard error, so there
        // is no static G11 gate (`insert` has no bulk form to distinguish).
        if item.type_ != element_type {
            return Err(format!(
                "native collection insert item must be {element_type}, got {}",
                item.type_
            ));
        }
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object("inplace_insert_item", 8);
        self.emit(abi::store_u64(
            &item.location,
            abi::stack_pointer(),
            item_slot,
        ));
        self.lower_list_insert_in_place(
            target.dest.block_slot(),
            index_slot,
            item_slot,
            &list_type,
            &element_type,
        )?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }
}
