// --- codegen tier imports (migration) ---
use crate::codegen::collection::assign::inplace_dest::*;
use crate::codegen::collection::assign::self_update::SelfUpdateSite;
use crate::codegen::collection::assign::string_self_update::StringRegrow;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::control::string_self_append_operands_of;
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
    /// has no live alias. A `FOR EACH` over the local whose body writes it walks
    /// a copy made at loop entry (plan-142-F), and one that borrows it declines
    /// (`G7`), because a grow frees the block the loop holds. A reference
    /// (`by_ref`) local is reached through `InPlaceDest::Ref` (plan-142-G), and
    /// bulk `append(list, otherList)` is the next arm (the item must be a single
    /// element).
    pub(crate) fn try_inplace_append_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let name = site.name;
        // Container (plan-121-A's shared seam): `name = append(name, …)` on a
        // uniquely-owned binding. Discharges G1 `by_ref`, G2 the call shape,
        // G3/G4 target and arity, G5/G6 the self-update, G7 the live `FOR EACH`
        // hazard (the grow frees the buffer the loop snapshotted — bug-142),
        // and G10 the collection layout.
        let Some(target) = self.resolve_self_update(site, value, "append", 2) else {
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
        // plan-145-B: a record or `STATE` field grows its owner's block.
        if target.dest.is_field() {
            return self.lower_field_append(site, &target, false, &list_type, &element_type);
        }
        let item = self.lower_value_stored(&target.args[1])?;
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
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let name = site.name;
        // G1–G7, G10 (the shared container gates).
        let Some(target) = self.resolve_self_update(site, value, "add", 2) else {
            return Ok(false);
        };
        let args = target.args;
        let set_type = target.collection_type.clone();
        // G9 — `add` mutates a Set.
        let Some(element_type) =
            crate::codegen::engine::types::typed_set_element_type(&set_type).cloned()
        else {
            return Ok(false);
        };
        match self.static_item_type(&args[1]) {
            Some(item_type) if item_type == element_type => {}
            _ => return Ok(false),
        }
        // plan-145-B: a field's `Set` grows its owner's block (`InlineGrow`).
        if target.dest.is_field() {
            return self.lower_field_set_add(site, &target, &set_type, &element_type);
        }
        let stack_offset = target.dest.block_slot();
        let item = self.lower_value_stored(&args[1])?;
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
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let name = site.name;
        // G1–G7, G10 (the shared container gates).
        let Some(target) = self.resolve_self_update(site, value, "removeKey", 2) else {
            return Ok(false);
        };
        let args = target.args;
        let map_type = target.collection_type.clone();
        // G9 — `removeKey` mutates a Map.
        let Some((key_type, _value_type)) =
            crate::codegen::engine::types::typed_map_type_parts(&map_type)
                .map(|(k, v)| (k.clone(), v.clone()))
        else {
            return Ok(false);
        };
        match self.static_item_type(&args[1]) {
            Some(kt) if kt == key_type => {}
            _ => return Ok(false),
        }
        // plan-145-B: a field's `Map` compacts where it lies (the sub-block route).
        if target.dest.is_field() {
            return self.lower_field_remove_key(site, &target, &map_type, &key_type);
        }
        let stack_offset = target.dest.block_slot();
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

    pub(crate) fn try_inplace_bulk_append_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let name = site.name;
        // G1–G7, G10 (the shared container gates). G7 is the same live `FOR EACH`
        // iterable hazard as the single-element append: the grow path frees the
        // snapshot buffer out from under the loop (bug-142).
        let Some(target) = self.resolve_self_update(site, value, "append", 2) else {
            return Ok(false);
        };
        let args = target.args;
        // Exclude the self-alias `append(name, name)`: the grow path frees the old
        // buffer, so a RHS pointing at the same buffer would read freed memory. The
        // value path rebuilds correctly from both operands read up front.
        if site.is_self(&args[1]) {
            return Ok(false);
        }
        let list_type = target.collection_type.clone();
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&list_type).cloned()
        else {
            return Ok(false);
        };
        // Commit only for a statically-known RHS of the *list* type (not the
        // element type — that is the single-element fast path). A RHS whose static
        // type is unknown (a general call result) falls through to the value path.
        match self.static_item_type(&args[1]) {
            Some(item_type) if item_type == list_type => {}
            _ => return Ok(false),
        }
        // plan-145-B: a record or `STATE` field grows its owner's block.
        if target.dest.is_field() {
            return self.lower_field_append(site, &target, true, &list_type, &element_type);
        }
        let stack_offset = target.dest.block_slot();
        let rhs = self.lower_value_stored(&args[1])?;
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
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let name = site.name;
        // G1–G7, G10 (the shared container gates).
        let Some(target) = self.resolve_self_update(site, value, "set", 3) else {
            return Ok(false);
        };
        let collection_type = target.collection_type.clone();
        // plan-145-B: a field's `List`/`Map` (split by element width).
        if target.dest.is_field() {
            return self.lower_field_set(site, &target, &collection_type);
        }
        let args = target.args;
        let stack_offset = target.dest.block_slot();
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
            let item = self.lower_value_stored(&args[2])?;
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
            let val = self.lower_value_stored(&args[2])?;
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
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let name = site.name;
        // G1–G7, G10 (the shared container gates).
        let Some(target) = self.resolve_self_update(site, value, "prepend", 2) else {
            return Ok(false);
        };
        let list_type = target.collection_type.clone();
        // G9 — `prepend` mutates a List.
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&list_type).cloned()
        else {
            return Ok(false);
        };
        // plan-145-B: a field's `List` splices in, growing its owner's block.
        if target.dest.is_field() {
            return self.lower_field_splice(site, &target, 2, &list_type, &element_type);
        }
        let args = target.args;
        let stack_offset = target.dest.block_slot();
        // `prepend` always takes a single element of the list element type
        // (a bulk form is rejected in `lower_collection_prepend`), so no static
        // gate is needed; the post-lowering check catches any mismatch.
        let item = self.lower_value_stored(&args[1])?;
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
    /// A global's shadow is a hidden global (plan-142-H), worked on through a
    /// frame slot. Returns `true` when handled.
    pub(crate) fn try_inplace_concat_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let name = site.name;
        // G1 — a by-ref slot holds the parent's slot address, not the buffer.
        if site.by_ref {
            return Ok(false);
        }
        let stack_offset = site.dest.block_slot();
        // Only fire for a name we pre-allocated a capacity shadow for (a self-append
        // target discovered by the prescan — for a global, the hidden global
        // `add_global_string_capacities` declared); the shadow is reset on every
        // other bind/assign so it always reflects the live buffer's spare bytes.
        if !self.string_shadow_exists(site) {
            return Ok(false);
        }
        let Some(operands) = string_self_append_operands_of(value, &|root| site.is_self(root))
        else {
            return Ok(false);
        };
        // If the target reappears in a later operand (`s = s & x & s`), lowering
        // operands in sequence would re-read the already-mutated buffer and append
        // the extended value (bug-143). Fall back to the out-of-place concat path,
        // which reads every operand from the original value.
        if operands.iter().any(|operand| site.read_by(operand)) {
            return Ok(false);
        }
        // `G-global-operand` — an operand that stores to the global would replace
        // the buffer under the append (and leave the shadow describing the old one).
        if let InPlaceDest::Global { name, .. } = &site.dest {
            let leaf = crate::codegen::engine::value::store_reach::StoreLeaf::Global(name);
            if operands
                .iter()
                .any(|operand| self.values_reach_store(std::slice::from_ref(*operand), leaf))
            {
                return Ok(false);
            }
        }
        let shadow = self
            .string_shadow_slot(site)?
            .ok_or("native self-append lost its capacity shadow")?;
        for operand in operands {
            self.lower_string_self_append_one(stack_offset, shadow.slot, operand)?;
        }
        self.publish_string_shadow(&shadow)?;
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
        self.emit_string_regrow(
            &StringRegrow {
                name_slot,
                shadow_slot,
                need_slot: newlen_slot,
                len_after_slot: newlen_slot,
                newcap_slot,
                newbuf_slot,
                oldsize_slot,
                ptr: &ptr,
                len: &len,
                cap: &right_ptr,
                spare: &spare,
                newcap: &newcap,
                step_scratch: &step_scratch,
                newlen: &newlen,
                dst: &dst,
                oldsize: &oldsize,
                alloc_ok: &alloc_ok,
                cap_keep: &cap_keep,
                step_prefix: "concat_self_step",
                copy_prefix: "concat_self_old",
            },
            &mut |b, dst| {
                // Copy the operand bytes (rlen) to newbuf+8+len.
                b.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
                b.emit(abi::load_u64(&rlen, &right_ptr, 0)); // rlen
                b.emit(abi::add_immediate(&right_ptr, &right_ptr, 8)); // operand data
                b.emit_copy_bytes(dst, &right_ptr, &rlen, "concat_self_new");
                // NUL terminator at newbuf+8+newlen.
                b.emit(abi::move_immediate(&zero, "Integer", "0"));
                b.emit(abi::store_u8(&zero, dst, 0));
                Ok(())
            },
        )?;
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
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let name = site.name;
        // Container (plan-121-A's seam). G7 — the live-`FOR EACH` decline — is
        // enforced here for the shift reason above, not merely the realloc one.
        let Some(target) = self.resolve_self_update(site, value, "removeAt", 2) else {
            return Ok(false);
        };
        let list_type = target.collection_type.clone();
        // G9 — `removeAt` mutates a List.
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&list_type).cloned()
        else {
            return Ok(false);
        };
        // plan-145-B: a field's `List` shifts down where it lies.
        if target.dest.is_field() {
            return self.lower_field_remove_at(site, &target, &list_type, &element_type);
        }
        // G24 — LIFTED by plan-134-H. `removeAt` compacts the data region: it moves
        // surviving payloads *down* inside the live buffer, which is safe only while
        // nothing else refers into those payloads. It used to decline a recursive
        // element type (`type_participates_in_cycle`) because `collections::get` of
        // one handed back an ALIAS into the data region: `get(xs, 0)` then
        // `xs = removeAt(xs, 0)` then `MATCH` on the value read fell to `CASE ELSE` for
        // every element whose removal moved bytes (plan-121-B B7).
        //
        // Neither half holds any more. `get` of a recursive element returns an owned
        // deep copy (bug-538, `materialize_owned_element`'s `needs_graph_copy` branch,
        // plan-134-D), so nothing a program can hold points into the buffer. And the
        // compaction now frees the removed element's graph before its entry is shifted
        // away (`lower_list_remove_at_in_place`), so the in-place arm leaks nothing
        // the rebuild would have freed. Pinned by
        // `a_fetched_recursive_element_survives_an_in_place_remove_and_a_growing_append`
        // (`tests/runtime/rt_recursive_value_collection_drops.rs`). `insert` and
        // `prepend` never needed the gate: they place the new payload at the data tail
        // and shift only the 40-byte lookup entries.
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
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let name = site.name;
        let Some(target) = self.resolve_self_update(site, value, "remove", 2) else {
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
        // plan-145-B: a field's `Set` compacts where it lies.
        if target.dest.is_field() {
            return self.lower_field_set_remove(site, &target, &set_type, &element_type);
        }
        let item = self.lower_value_stored(&target.args[1])?;
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
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let name = site.name;
        // Container (plan-121-A's seam). G7 — the live-`FOR EACH` decline — is
        // enforced here for the shift reason above, not merely the realloc one.
        let Some(target) = self.resolve_self_update(site, value, "insert", 3) else {
            return Ok(false);
        };
        let list_type = target.collection_type.clone();
        // G9 — `insert` mutates a List.
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(&list_type).cloned()
        else {
            return Ok(false);
        };
        // plan-145-B: a field's `List` splices in, growing its owner's block.
        if target.dest.is_field() {
            return self.lower_field_splice(site, &target, 3, &list_type, &element_type);
        }
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
        let item = self.lower_value_stored(&target.args[2])?;
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

// ---------------------------------------------------------------------------
// plan-145-B: the field routes of the seam arms.
//
// These are the bodies of the nine record-field arms (plan-121-C) and eight
// `RES … STATE` arms (plan-121-D, bug-430) that used to be dispatched outside the
// seam, moved here unchanged. Each serves the seam arm of the same operation when
// its site is a field (`SelfUpdateTarget::dest.is_field()`), after the seam's
// resolver has run the field container's gates (`G13`/`G14` in the site builder;
// `G15`/`G16`/`G17`/`G10`/`G25`/`G18` in `resolve_self_update`).
//
// The two containers differed in three places, and those are the only branches:
//   * the `STATE` destination is opened (the STATE pointer loaded,
//     `open_inplace_dest`) after the gates and BEFORE the operands are lowered
//     (`O-order-4`) — opening a record destination emits nothing, so calling it at
//     the same point keeps the record arms' bytes;
//   * the slot names, which the `.ncode` records (`inplace_recfield_*` /
//     `inplace_state_*`, and bug-430's `inline_state_rhs`);
//   * `close_inplace_dest` publishes a `STATE` block back through the resource
//     (`O4`) and is a no-op for a record.
// ---------------------------------------------------------------------------

/// The slot name a field route allocates: the record arm's or the `STATE` arm's.
fn field_slot_name(dest: &InPlaceDest, record: &'static str, state: &'static str) -> &'static str {
    if matches!(dest, InPlaceDest::StateField { .. }) {
        state
    } else {
        record
    }
}

impl CodeBuilder<'_> {
    /// A record field's local no longer holds a folded constant after an in-place
    /// update (the record arms cleared it; a `STATE` handle never has one).
    fn forget_field_owner_constant(&mut self, site: &SelfUpdateSite<'_>, dest: &InPlaceDest) {
        if !matches!(dest, InPlaceDest::StateField { .. }) {
            if let Some(local) = self.locals.get_mut(site.name) {
                local.constant = None;
            }
        }
    }

    /// `append` at a field — one element (`bulk = false`) or a whole list — into
    /// the last-inlined `List`, growing the owner's block (bug-430).
    fn lower_field_append(
        &mut self,
        site: &SelfUpdateSite<'_>,
        target: &SelfUpdateTarget<'_>,
        bulk: bool,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<bool, String> {
        // `G12` — exclude the self-alias `append(field, field)`: the grow frees the
        // old block out from under the RHS copy.
        if site.is_self(&target.args[1]) {
            return Ok(false);
        }
        // `G17` — a grow must not shift a later sibling.
        if !self.field_is_last_inlined(site) {
            return Ok(false);
        }
        let dest = self.open_inplace_dest(&target.dest)?;
        let rhs = self.lower_value_stored(&target.args[1])?;
        self.observe_float(&target.args[1], &rhs)?;
        let rhs = self.materialize_value(rhs)?;
        let rhs_slot = self.allocate_stack_object(
            field_slot_name(&target.dest, "inplace_recfield_rhs", "inline_state_rhs"),
            8,
        );
        self.emit(abi::store_u64(
            &rhs.location,
            abi::stack_pointer(),
            rhs_slot,
        ));
        self.lower_inplace_inlined_list_grow(&dest, bulk, list_type, element_type, rhs_slot)?;
        self.close_inplace_dest(&dest)?;
        self.forget_field_owner_constant(site, &target.dest);
        Ok(true)
    }

    /// `add` at a field: the growing `Set` insert, through `InlineGrow`.
    fn lower_field_set_add(
        &mut self,
        site: &SelfUpdateSite<'_>,
        target: &SelfUpdateTarget<'_>,
        set_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<bool, String> {
        // `G12` — exclude the self-alias `add(field, field)`.
        if site.is_self(&target.args[1]) {
            return Ok(false);
        }
        // `G17` — a grow must not shift a later sibling.
        if !self.field_is_last_inlined(site) {
            return Ok(false);
        }
        let dest = self.open_inplace_dest(&target.dest)?;
        let block_slot = dest.block_slot();
        let item = self.lower_value_stored(&target.args[1])?;
        self.observe_float(&target.args[1], &item)?;
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_add_item",
                "inplace_state_add_item",
            ),
            8,
        );
        self.store_value_at(&item, abi::stack_pointer(), item_slot);
        // A Set is a Map to TRUE.
        let true_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_add_true",
                "inplace_state_add_true",
            ),
            8,
        );
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
            set_type,
            element_type,
            &ParameterType::Boolean,
            Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                block_slot,
                field_off_slot,
            }),
        )?;
        // `O4` — the grow may have moved a STATE block; publish it.
        self.close_inplace_dest(&dest)?;
        self.forget_field_owner_constant(site, &target.dest);
        Ok(true)
    }

    /// `set` at a field. It splits by element width, not collection kind: a
    /// fixed-width `List` element is replaced where it lies (the sub-block route —
    /// `lower_list_set_in_place`'s rebuild branch is unreachable for it), a
    /// variable-width one declines (a longer replacement makes that branch
    /// reachable, and it installs a fresh block no sub-block can receive), and a
    /// `Map` takes `InlineGrow`, because a new key grows the map.
    fn lower_field_set(
        &mut self,
        site: &SelfUpdateSite<'_>,
        target: &SelfUpdateTarget<'_>,
        collection_type: &ParameterType,
    ) -> Result<bool, String> {
        if let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(collection_type).cloned()
        {
            if crate::codegen::collection::layout::list_element_is_fixed_width(&element_type)
                .is_none()
            {
                return Ok(false);
            }
            let dest = self.open_inplace_dest(&target.dest)?;
            let index = self.lower_value(&target.args[1])?;
            if index.type_ != ParameterType::Integer {
                return Err(format!(
                    "native collection set list index must be Integer, got {}",
                    index.type_
                ));
            }
            let index = self.materialize_value(index)?;
            let index_slot = self.allocate_stack_object(
                field_slot_name(
                    &target.dest,
                    "inplace_recfield_set_index",
                    "inplace_state_set_index",
                ),
                8,
            );
            self.emit(abi::store_u64(
                &index.location,
                abi::stack_pointer(),
                index_slot,
            ));
            let item = self.lower_value_stored(&target.args[2])?;
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
            let item_slot = self.allocate_stack_object(
                field_slot_name(
                    &target.dest,
                    "inplace_recfield_set_item",
                    "inplace_state_set_item",
                ),
                8,
            );
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
                collection_type,
                &element_type,
            )?;
            // `O4`. Nothing moved on this route, so a STATE write-back republishes
            // the pointer it read.
            self.close_inplace_dest(&dest)?;
            self.forget_field_owner_constant(site, &target.dest);
            return Ok(true);
        }
        let Some((key_type, value_type)) =
            crate::codegen::engine::types::typed_map_type_parts(collection_type)
                .map(|(k, v)| (k.clone(), v.clone()))
        else {
            return Ok(false);
        };
        // `G17` — a new key grows the map, which must not shift a later sibling.
        if !self.field_is_last_inlined(site) {
            return Ok(false);
        }
        let dest = self.open_inplace_dest(&target.dest)?;
        let block_slot = dest.block_slot();
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
        let key_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_set_key",
                "inplace_state_set_key",
            ),
            8,
        );
        self.emit(abi::store_u64(
            &key.location,
            abi::stack_pointer(),
            key_slot,
        ));
        let val = self.lower_value_stored(&target.args[2])?;
        // Observation boundary: an in-place `Float` map value must be finite (plan-17).
        self.observe_float(&target.args[2], &val)?;
        if val.type_ != value_type {
            return Err(format!(
                "native collection set map value must be {value_type}, got {}",
                val.type_
            ));
        }
        let val = self.materialize_value(val)?;
        let value_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_set_value",
                "inplace_state_set_value",
            ),
            8,
        );
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
            collection_type,
            &key_type,
            &value_type,
            Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                block_slot,
                field_off_slot,
            }),
        )?;
        // `O4`.
        self.close_inplace_dest(&dest)?;
        self.forget_field_owner_constant(site, &target.dest);
        Ok(true)
    }

    /// `removeKey` at a field: non-growing, so the sub-block route —
    /// `lower_map_remove_key_in_place` only loads the slot it is given.
    fn lower_field_remove_key(
        &mut self,
        site: &SelfUpdateSite<'_>,
        target: &SelfUpdateTarget<'_>,
        map_type: &ParameterType,
        key_type: &ParameterType,
    ) -> Result<bool, String> {
        let dest = self.open_inplace_dest(&target.dest)?;
        let key = self.lower_value(&target.args[1])?;
        let key = self.materialize_value(key)?;
        let key_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_remove_key",
                "inplace_state_remove_key",
            ),
            8,
        );
        self.store_value_at(&key, abi::stack_pointer(), key_slot);
        // `O-order-4`: the sub-block address is taken AFTER the operand is lowered —
        // lowering the key can itself call `_mfb_arena_alloc`, and the address is
        // never held across an allocation.
        let map_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_map_remove_key_in_place(map_slot, key_slot, map_type, key_type)?;
        // `O4`.
        self.close_inplace_dest(&dest)?;
        self.forget_field_owner_constant(site, &target.dest);
        Ok(true)
    }

    /// `removeAt` at a field: it shifts down and decrements `count`, never
    /// reallocating, so the sub-block route serves it.
    fn lower_field_remove_at(
        &mut self,
        site: &SelfUpdateSite<'_>,
        target: &SelfUpdateTarget<'_>,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<bool, String> {
        let dest = self.open_inplace_dest(&target.dest)?;
        let index = self.lower_value(&target.args[1])?;
        // `E1` — the index is Integer by construction.
        if index.type_ != ParameterType::Integer {
            return Err(format!(
                "native collection removeAt index must be Integer, got {}",
                index.type_
            ));
        }
        let index = self.materialize_value(index)?;
        let index_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_remove_at_index",
                "inplace_state_remove_at_index",
            ),
            8,
        );
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        let buffer_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_list_remove_at_in_place(buffer_slot, index_slot, list_type, element_type)?;
        // `O4`.
        self.close_inplace_dest(&dest)?;
        self.forget_field_owner_constant(site, &target.dest);
        Ok(true)
    }

    /// Set `remove` at a field: a Set is a Map to TRUE, so this is the
    /// `removeKey` sub-block route.
    fn lower_field_set_remove(
        &mut self,
        site: &SelfUpdateSite<'_>,
        target: &SelfUpdateTarget<'_>,
        set_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<bool, String> {
        let dest = self.open_inplace_dest(&target.dest)?;
        let item = self.lower_value_stored(&target.args[1])?;
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_set_remove",
                "inplace_state_set_remove",
            ),
            8,
        );
        self.store_value_at(&item, abi::stack_pointer(), item_slot);
        let set_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_map_remove_key_in_place(set_slot, item_slot, set_type, element_type)?;
        // `O4`.
        self.close_inplace_dest(&dest)?;
        self.forget_field_owner_constant(site, &target.dest);
        Ok(true)
    }

    /// `insert` (`arity` 3) and `prepend` (`arity` 2) at a field: both grow, so
    /// both take `InlineGrow` (`lower_list_splice_in_place`; a `prepend` is
    /// `SpliceAt::Front`). Bounds are the lowering's own.
    fn lower_field_splice(
        &mut self,
        site: &SelfUpdateSite<'_>,
        target: &SelfUpdateTarget<'_>,
        arity: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<bool, String> {
        let rhs_index = arity - 1;
        // `G12` — exclude the self-alias: the grow frees the old block out from
        // under the right-hand side copy.
        if site.is_self(&target.args[rhs_index]) {
            return Ok(false);
        }
        // `G11` — the spliced-in value must be a single element.
        match self.static_item_type(&target.args[rhs_index]) {
            Some(vt) if vt == *element_type => {}
            _ => return Ok(false),
        }
        // `G17` — a grow must not shift a later sibling.
        if !self.field_is_last_inlined(site) {
            return Ok(false);
        }
        let dest = self.open_inplace_dest(&target.dest)?;
        let block_slot = dest.block_slot();
        let at = if arity == 3 {
            let index = self.lower_value(&target.args[1])?;
            if index.type_ != ParameterType::Integer {
                return Err(format!(
                    "native collection insert index must be Integer, got {}",
                    index.type_
                ));
            }
            let index = self.materialize_value(index)?;
            let index_slot = self.allocate_stack_object(
                field_slot_name(
                    &target.dest,
                    "inplace_recfield_splice_index",
                    "inplace_state_splice_index",
                ),
                8,
            );
            self.emit(abi::store_u64(
                &index.location,
                abi::stack_pointer(),
                index_slot,
            ));
            crate::codegen::collection::list::list_mutate::SpliceAt::At(index_slot)
        } else {
            crate::codegen::collection::list::list_mutate::SpliceAt::Front
        };
        let item = self.lower_value_stored(&target.args[rhs_index])?;
        self.observe_float(&target.args[rhs_index], &item)?;
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object(
            field_slot_name(
                &target.dest,
                "inplace_recfield_splice_item",
                "inplace_state_splice_item",
            ),
            8,
        );
        self.store_value_at(&item, abi::stack_pointer(), item_slot);
        let field_off_slot = self.open_inplace_inlined_field_offset(&dest)?;
        let buffer_slot = self.open_inplace_inlined_subblock(&dest)?;
        self.lower_list_splice_in_place(
            buffer_slot,
            at,
            item_slot,
            list_type,
            element_type,
            Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                block_slot,
                field_off_slot,
            }),
        )?;
        // `O4` — the grow may have moved a STATE block; publish it.
        self.close_inplace_dest(&dest)?;
        self.forget_field_owner_constant(site, &target.dest);
        Ok(true)
    }
}
