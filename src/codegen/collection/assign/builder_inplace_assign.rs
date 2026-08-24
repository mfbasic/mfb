// --- codegen tier imports (migration) ---
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
        // A live `FOR EACH` iterable snapshots the buffer pointer/count at loop
        // entry; the grow path frees the old buffer once the append outgrows
        // capacity, so an in-place append to the list being iterated is a
        // use-after-free (bug-142). Force the out-of-place (copying) path, matching
        // the set/prepend guards.
        if self.for_each_iterable_locals.iter().any(|n| n == name) {
            return Ok(false);
        }
        let Some(local) = self.locals.get(name) else {
            return Ok(false);
        };
        let list_type = local.type_.clone();
        let Some(element_type) =
            crate::codegen::engine::types::list_element_type(&list_type.name())
        else {
            return Ok(false);
        };
        if crate::codegen::engine::builder::CollectionTypeLayout::from_type(&list_type.name())
            .is_none()
        {
            return Ok(false);
        }
        // Commit only for a statically-known single element of the list's element
        // type. A bulk `append(list, otherList)` has item type == list_type and
        // falls through to the general (concatenating) path.
        match self.static_type_name(&args[1]) {
            Some(item_type) if item_type == element_type => {}
            _ => return Ok(false),
        }
        let item = self.lower_value(&args[1])?;
        // Observation boundary: an in-place appended `Float` must be finite
        // (plan-17).
        self.observe_float(&args[1], &item)?;
        // Materialize a `d`-native float before the payload spill (plan-01).
        let item = self.materialize_value(item)?;
        let item_slot = self.allocate_stack_object("inplace_append_item", 8);
        self.emit(abi::store_u64(
            &item.location,
            abi::stack_pointer(),
            item_slot,
        ));
        self.lower_list_append_in_place(stack_offset, item_slot, &list_type.name(), &element_type)?;
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
        if by_ref {
            return Ok(false);
        }
        let NirValue::WithUpdate {
            type_,
            target,
            updates,
        } = value
        else {
            return Ok(false);
        };
        // The update must rebuild THIS same local (self-update), not install some
        // other record as the new value.
        if !matches!(target.as_ref(), NirValue::Local(n) if n == name) {
            return Ok(false);
        }
        if updates.len() != 1 {
            return Ok(false);
        }
        let update = &updates[0];
        // A live `FOR EACH` over this record's field aliases the buffer the grow
        // would free — take the non-freeing rebuild instead.
        if self
            .for_each_iterable_record_fields
            .iter()
            .any(|(base, field)| base == name && field == &update.field)
        {
            return Ok(false);
        }
        let Some((field_index, field_type)) =
            self.record_collection_last_inlined(&type_.name(), &update.field)
        else {
            return Ok(false);
        };
        let Some(element_type) = crate::codegen::engine::types::list_element_type(&field_type)
        else {
            return Ok(false);
        };
        if crate::codegen::engine::builder::CollectionTypeLayout::from_type(&field_type).is_none() {
            return Ok(false);
        }
        let NirValue::Call {
            target: call_target,
            args,
            ..
        } = &update.value
        else {
            return Ok(false);
        };
        if crate::codegen::builtins::native_builtin_target(call_target) != Some("append")
            || args.len() != 2
        {
            return Ok(false);
        }
        // The appended-to source must be exactly this same field (self-append).
        if !self.value_is_record_field(&args[0], name, &update.field) {
            return Ok(false);
        }
        // Single element (item type == element type) vs bulk concatenation.
        let bulk = match self.static_type_name(&args[1]) {
            Some(t) if t == element_type => false,
            Some(t) if t == field_type => true,
            _ => return Ok(false),
        };
        // Exclude the self-alias `append(field, field)`.
        if self.value_is_record_field(&args[1], name, &update.field) {
            return Ok(false);
        }

        // Evaluate the appended value and spill it for the grow helper.
        let rhs = self.lower_value(&args[1])?;
        self.observe_float(&args[1], &rhs)?;
        let rhs = self.materialize_value(rhs)?;
        let rhs_slot = self.allocate_stack_object("inplace_recfield_rhs", 8);
        self.emit(abi::store_u64(
            &rhs.location,
            abi::stack_pointer(),
            rhs_slot,
        ));

        // The local slot holds the record pointer; the grow helper repoints it on
        // a realloc, so the reassignment `rec = …` needs no further store.
        if bulk {
            self.lower_inline_list_bulk_append_in_place(
                stack_offset,
                field_index,
                &field_type,
                &element_type,
                rhs_slot,
            )?;
        } else {
            self.lower_inline_list_append_in_place(
                stack_offset,
                field_index,
                &field_type,
                &element_type,
                rhs_slot,
            )?;
        }
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
        let Some(element_type) = crate::codegen::engine::types::set_element_type(&set_type.name())
        else {
            return Ok(false);
        };
        if crate::codegen::engine::builder::CollectionTypeLayout::from_type(&set_type.name())
            .is_none()
        {
            return Ok(false);
        }
        match self.static_type_name(&args[1]) {
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
        let true_reg = self.allocate_register()?;
        self.emit(abi::move_immediate(&true_reg, "Boolean", "true"));
        self.emit(abi::store_u64(&true_reg, abi::stack_pointer(), true_slot));
        self.lower_map_set_in_place(
            stack_offset,
            item_slot,
            true_slot,
            &set_type.name(),
            &element_type,
            "Boolean",
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
            crate::codegen::engine::types::map_type_parts(&map_type.name())
        else {
            return Ok(false);
        };
        if crate::codegen::engine::builder::CollectionTypeLayout::from_type(&map_type.name())
            .is_none()
        {
            return Ok(false);
        }
        match self.static_type_name(&args[1]) {
            Some(kt) if kt == key_type => {}
            _ => return Ok(false),
        }
        let key = self.lower_value(&args[1])?;
        let key = self.materialize_value(key)?;
        let key_slot = self.allocate_stack_object("inplace_remove_key", 8);
        self.store_value_at(&key, abi::stack_pointer(), key_slot);
        self.lower_map_remove_key_in_place(stack_offset, key_slot, &map_type.name(), &key_type)?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
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
            crate::codegen::engine::types::list_element_type(&list_type.name())
        else {
            return Ok(false);
        };
        if crate::codegen::engine::builder::CollectionTypeLayout::from_type(&list_type.name())
            .is_none()
        {
            return Ok(false);
        }
        // Commit only for a statically-known RHS of the *list* type (not the
        // element type — that is the single-element fast path). A RHS whose static
        // type is unknown (a general call result) falls through to the value path.
        match self.static_type_name(&args[1]) {
            Some(item_type) if item_type == list_type.name().as_ref() => {}
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
        self.lower_list_bulk_append_in_place(
            stack_offset,
            rhs_slot,
            &list_type.name(),
            &element_type,
        )?;
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
        if crate::codegen::engine::builder::CollectionTypeLayout::from_type(&collection_type.name())
            .is_none()
        {
            return Ok(false);
        }
        if let Some(element_type) =
            crate::codegen::engine::types::list_element_type(&collection_type.name())
        {
            // The list `set` item is always a single element of type `T`
            // (syntaxcheck-enforced), so — unlike append's bulk-vs-single gate — no
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
            if item.type_.name() != element_type.as_str() {
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
                &collection_type.name(),
                &element_type,
            )?;
            if let Some(local) = self.locals.get_mut(name) {
                local.constant = None;
            }
            return Ok(true);
        }
        if let Some((key_type, value_type)) =
            crate::codegen::engine::types::map_type_parts(&collection_type.name())
        {
            let key = self.lower_value(&args[1])?;
            // Observation boundary: an in-place `Float` map key must be finite
            // (plan-17).
            self.observe_float(&args[1], &key)?;
            if key.type_.name() != key_type.as_str() {
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
            if val.type_.name() != value_type.as_str() {
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
                &collection_type.name(),
                &key_type,
                &value_type,
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
            crate::codegen::engine::types::list_element_type(&list_type.name())
        else {
            return Ok(false);
        };
        if crate::codegen::engine::builder::CollectionTypeLayout::from_type(&list_type.name())
            .is_none()
        {
            return Ok(false);
        }
        // `prepend` always takes a single element of the list element type
        // (a bulk form is rejected in `lower_collection_prepend`), so no static
        // gate is needed; the post-lowering check catches any mismatch.
        let item = self.lower_value(&args[1])?;
        // Observation boundary: an in-place prepended `Float` must be finite
        // (plan-17).
        self.observe_float(&args[1], &item)?;
        if item.type_.name() != element_type.as_str() {
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
        self.lower_list_prepend_in_place(
            stack_offset,
            item_slot,
            &list_type.name(),
            &element_type,
        )?;
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
}
