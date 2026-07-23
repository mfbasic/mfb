use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_collection_append(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection append does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("append_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let item = self.lower_value(&args[1])?;
        // Observation boundary: a `Float` appended element must be finite
        // (plan-17).
        self.observe_float(&args[1], &item)?;
        // A `d`-native float item is materialized into a GPR before being
        // spilled into the collection payload (plan-01 float-dnative).
        let item = self.materialize_value(item)?;
        let (insert_slot, materialized) =
            self.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
        let index_slot = self.allocate_stack_object("append_index", 8);
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), list_slot));
        self.emit(abi::load_u64(&scratch8, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), index_slot));
        let result = self.lower_list_insert_collection(
            list_slot,
            index_slot,
            insert_slot,
            &list.type_,
            &element_type,
        )?;
        if materialized {
            return self.free_intermediate_collection(insert_slot, &list.type_, result);
        }
        Ok(result)
    }

    pub(super) fn lower_collection_prepend(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection prepend does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("prepend_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let item = self.lower_value(&args[1])?;
        // Observation boundary: a `Float` prepended element must be finite
        // (plan-17).
        self.observe_float(&args[1], &item)?;
        if item.type_ == list.type_ {
            return Err("native collection prepend expects a single item, not a list".to_string());
        }
        // Materialize a `d`-native float before the payload spill (plan-01).
        let item = self.materialize_value(item)?;
        let (insert_slot, materialized) =
            self.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
        let index_slot = self.allocate_stack_object("prepend_index", 8);
        self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), index_slot));
        let result = self.lower_list_insert_collection(
            list_slot,
            index_slot,
            insert_slot,
            &list.type_,
            &element_type,
        )?;
        if materialized {
            return self.free_intermediate_collection(insert_slot, &list.type_, result);
        }
        Ok(result)
    }

    pub(super) fn lower_collection_insert(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection insert does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("insert_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let index = self.lower_value(&args[1])?;
        if index.type_ != "Integer" {
            return Err(format!(
                "native collection insert index must be Integer, got {}",
                index.type_
            ));
        }
        let index_slot = self.allocate_stack_object("insert_index", 8);
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        let item = self.lower_value(&args[2])?;
        // Observation boundary: a `Float` inserted element must be finite
        // (plan-17).
        self.observe_float(&args[2], &item)?;
        if item.type_ == list.type_ {
            return Err("native collection insert expects a single item, not a list".to_string());
        }
        // Materialize a `d`-native float before the payload spill (plan-01).
        let item = self.materialize_value(item)?;
        let (insert_slot, materialized) =
            self.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
        let result = self.lower_list_insert_collection(
            list_slot,
            index_slot,
            insert_slot,
            &list.type_,
            &element_type,
        )?;
        if materialized {
            return self.free_intermediate_collection(insert_slot, &list.type_, result);
        }
        Ok(result)
    }

    pub(super) fn lower_collection_remove_at(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection removeAt does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("remove_at_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let index = self.lower_value(&args[1])?;
        if index.type_ != "Integer" {
            return Err(format!(
                "native collection removeAt index must be Integer, got {}",
                index.type_
            ));
        }
        let index_slot = self.allocate_stack_object("remove_at_index", 8);
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        self.lower_list_remove_at(list_slot, index_slot, &list.type_, &element_type)
    }

    pub(super) fn lower_collection_set(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        if let Some(element_type) = list_element_type(&collection.type_) {
            let list_slot = self.allocate_stack_object("set_list", 8);
            self.emit(abi::store_u64(
                &collection.location,
                abi::stack_pointer(),
                list_slot,
            ));
            let index = self.lower_value(&args[1])?;
            if index.type_ != "Integer" {
                return Err(format!(
                    "native collection set list index must be Integer, got {}",
                    index.type_
                ));
            }
            let index_slot = self.allocate_stack_object("set_index", 8);
            self.emit(abi::store_u64(
                &index.location,
                abi::stack_pointer(),
                index_slot,
            ));
            let item = self.lower_value(&args[2])?;
            // Observation boundary: a `Float` replacement element must be finite
            // (plan-17).
            self.observe_float(&args[2], &item)?;
            if item.type_ != element_type {
                return Err(format!(
                    "native collection set list item must be {}, got {}",
                    element_type, item.type_
                ));
            }
            // Materialize a `d`-native float before the payload spill (plan-01).
            let item = self.materialize_value(item)?;
            // bug-365: for a fixed-width element the replacement payload is the
            // same size as the one it replaces, by definition. So copy the block
            // and overwrite the payload in place rather than degrading to
            // `removeAt` + `insert` — that pair appended the new payload to the
            // data tail and spliced the lookup table, leaving the data region
            // permuted relative to index order for any `i < count-1`, which every
            // linear data-region reader (the `math::` SIMD kernels, the `fs` byte
            // writers) then read back in the wrong order.
            //
            // `copy_collection_tight` copies entries and data verbatim, so an
            // ordered source stays ordered; `lower_list_set_in_place` then writes
            // through the stored `valueOffset`. Its rebuild branch is unreachable
            // here — it fires only on a size change, and these payloads cannot
            // change size — so the write is always the in-place overwrite. It also
            // range-checks the index itself, so the bounds behavior below is
            // preserved. Cheaper too: one allocation and one block copy replace
            // two of each.
            if list_element_is_fixed_width(&element_type).is_some() {
                let item_slot = self.allocate_stack_object("set_value_item", 8);
                self.emit(abi::store_u64(
                    &item.location,
                    abi::stack_pointer(),
                    item_slot,
                ));
                let source = self.allocate_register()?;
                self.emit(abi::load_u64(&source, abi::stack_pointer(), list_slot));
                let copy = self.copy_collection_tight(&collection.type_, &source)?;
                let copy_slot = self.allocate_stack_object("set_value_copy", 8);
                self.emit(abi::store_u64(&copy, abi::stack_pointer(), copy_slot));
                return self.lower_list_set_in_place(
                    copy_slot,
                    index_slot,
                    item_slot,
                    &collection.type_,
                    &element_type,
                );
            }
            // Do the fallible `removeAt` (which range-checks the index) BEFORE
            // materializing the singleton, so an out-of-range index — the failure
            // an inline `TRAP`'d or auto-propagating `set` hits — routes to the
            // handler with nothing yet allocated, and cannot leak the singleton
            // (bug-147.5). `removeAt` allocates its product only after the bounds
            // pass, so the OOB route allocates nothing at all. Both intermediates
            // are freed on the success path once the insert has copied out of them;
            // the sole remaining leak window is a mid-operation OOM (arena already
            // exhausted), which was equally present before this reorder.
            let removed =
                self.lower_list_remove_at(list_slot, index_slot, &collection.type_, &element_type)?;
            let removed_slot = self.allocate_stack_object("set_removed_list", 8);
            self.emit(abi::store_u64(
                &removed.location,
                abi::stack_pointer(),
                removed_slot,
            ));
            let (singleton_slot, materialized) =
                self.collection_argument_as_list_slot(&collection.type_, &element_type, item)?;
            let mut result = self.lower_list_insert_collection(
                removed_slot,
                index_slot,
                singleton_slot,
                &collection.type_,
                &element_type,
            )?;
            // Both intermediates were fully copied into the result: the
            // materialized singleton and the removeAt product.
            if materialized {
                result =
                    self.free_intermediate_collection(singleton_slot, &collection.type_, result)?;
            }
            return self.free_intermediate_collection(removed_slot, &collection.type_, result);
        }

        if let Some((key_type, value_type)) = map_type_parts(&collection.type_) {
            let map_slot = self.allocate_stack_object("set_map", 8);
            self.emit(abi::store_u64(
                &collection.location,
                abi::stack_pointer(),
                map_slot,
            ));
            let key = self.lower_value(&args[1])?;
            // Observation boundary: a `Float` map key must be finite (plan-17).
            self.observe_float(&args[1], &key)?;
            if key.type_ != key_type {
                return Err(format!(
                    "native collection set map key must be {}, got {}",
                    key_type, key.type_
                ));
            }
            let key = self.materialize_value(key)?;
            let key_slot = self.allocate_stack_object("set_map_key", 8);
            self.emit(abi::store_u64(
                &key.location,
                abi::stack_pointer(),
                key_slot,
            ));
            let value = self.lower_value(&args[2])?;
            // Observation boundary: a `Float` map value must be finite (plan-17).
            self.observe_float(&args[2], &value)?;
            if value.type_ != value_type {
                return Err(format!(
                    "native collection set map value must be {}, got {}",
                    value_type, value.type_
                ));
            }
            let value = self.materialize_value(value)?;
            let value_slot = self.allocate_stack_object("set_map_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            let without =
                self.lower_map_remove_key(map_slot, key_slot, &collection.type_, &key_type)?;
            let without_slot = self.allocate_stack_object("set_map_without", 8);
            self.emit(abi::store_u64(
                &without.location,
                abi::stack_pointer(),
                without_slot,
            ));
            let singleton = self.lower_collection_values(
                &collection.type_,
                vec![CollectionValueSlot {
                    key: Some(PayloadSlot {
                        slot: key_slot,
                        type_: key_type.clone(),
                    }),
                    value: PayloadSlot {
                        slot: value_slot,
                        type_: value_type,
                    },
                }],
                "singleton map",
            )?;
            let singleton_slot = self.allocate_stack_object("set_map_singleton", 8);
            self.emit(abi::store_u64(
                &singleton.location,
                abi::stack_pointer(),
                singleton_slot,
            ));
            // The concat copies both intermediates into the result; free the
            // `without` whole-map copy and the `singleton` map afterward, mirroring
            // the list branch's frees. Without this every non-in-place map `set`
            // leaked one whole-map-sized block plus a singleton per call (bug-145).
            let result = self.lower_map_concat(without_slot, singleton_slot, &collection.type_)?;
            let result =
                self.free_intermediate_collection(without_slot, &collection.type_, result)?;
            return self.free_intermediate_collection(singleton_slot, &collection.type_, result);
        }

        Err(format!(
            "native collection set does not accept {} yet",
            collection.type_
        ))
    }

    pub(super) fn lower_collection_remove_key(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let map = self.lower_value(&args[0])?;
        let Some((key_type, _)) = map_type_parts(&map.type_) else {
            return Err(format!(
                "native collection removeKey does not accept {}",
                map.type_
            ));
        };
        let map_slot = self.allocate_stack_object("remove_key_map", 8);
        self.emit(abi::store_u64(
            &map.location,
            abi::stack_pointer(),
            map_slot,
        ));
        let key = self.lower_value(&args[1])?;
        if key.type_ != key_type {
            return Err(format!(
                "native collection removeKey key must be {}, got {}",
                key_type, key.type_
            ));
        }
        let key_slot = self.allocate_stack_object("remove_key_key", 8);
        // `d`-native float key stores via `str d` (plan-01 float-dnative).
        self.store_value_at(&key, abi::stack_pointer(), key_slot);
        self.lower_map_remove_key(map_slot, key_slot, &map.type_, &key_type)
    }
}
