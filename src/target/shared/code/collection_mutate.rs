use super::*;

impl CodeBuilder<'_> {
    /// Shared body of `append`/`prepend`: insert a single item at one end of a
    /// list. The two differ only in the insertion index (`count` for append, `0`
    /// for prepend), prepend's reject-a-list-argument guard, and the slot/error
    /// names — all keyed off `op`/`at_start`, so each variant emits exactly what
    /// its former standalone function did (`op` reproduces the original stack-slot
    /// names, keeping the dumps byte-identical).
    pub(crate) fn lower_collection_end_insert(
        &mut self,
        args: &[NirValue],
        op: &str,
        at_start: bool,
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection {op} does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object(&format!("{op}_list"), 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let item = self.lower_value(&args[1])?;
        // Observation boundary: a `Float` inserted element must be finite (plan-17).
        self.observe_float(&args[1], &item)?;
        if at_start && item.type_ == list.type_ {
            return Err("native collection prepend expects a single item, not a list".to_string());
        }
        // A `d`-native float item is materialized into a GPR before being
        // spilled into the collection payload (plan-01 float-dnative).
        let item = self.materialize_value(item)?;
        let (insert_slot, materialized) =
            self.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
        let index_slot = self.allocate_stack_object(&format!("{op}_index"), 8);
        if at_start {
            self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
        } else {
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), list_slot));
            self.emit(abi::load_u64(&scratch8, &scratch8, COLLECTION_OFFSET_COUNT));
        }
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
}
