//! `collections::mapValues` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::target::shared::abi;
use crate::target::shared::code::*;
use crate::target::shared::nir::NirValue;

/// Native fast path for `#collections_mapValues$K$V$U` with V == U and 8-byte
/// fixed-width (rewrites value payloads in place over a tight copy). Other
/// instantiations decline (`Ok(None)`). Free fn.
pub(super) fn map_values_fast_path(
    builder: &mut CodeBuilder,
    target: &str,
    args: &[NirValue],
) -> Result<Option<ValueResult>, String> {
    let Some(params) = target.strip_prefix("#collections_mapValues$") else {
        return Ok(None);
    };
    if args.len() != 2 {
        return Ok(None);
    }
    let parts: Vec<&str> = params.split('$').collect();
    let ok = parts.len() == 3
        && parts[1] == parts[2]
        && matches!(parts[1], "Integer" | "Float" | "Fixed" | "Money");
    if ok {
        return builder.lower_collection_map_values_call(args).map(Some);
    }
    Ok(None)
}

impl CodeBuilder<'_> {
    /// plan-64 C2: native `collections::mapValues` for a same-type 8-byte
    /// fixed-width value (V == U in Integer/Float/Fixed/Money), gated by parsing
    /// the monomorphized target `#collections_mapValues$K$V$U`. The `.mfb` version
    /// rebuilds the whole map entry-by-entry (`set(result, e.key, f(e.value))`,
    /// N inserts, leaving `ready=0`); this copies the map's key/bucket structure
    /// once and rewrites each value payload in place (keys unchanged → the copied
    /// index stays valid). Every other instantiation falls through to the `.mfb`.
    pub(super) fn lower_collection_map_values_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let map = self.lower_value(&args[0])?;
        let map_type = map.type_.clone();
        let map_slot = self.allocate_stack_object("mapvalues_map", 8);
        self.emit(abi::store_u64(
            &map.location,
            abi::stack_pointer(),
            map_slot,
        ));
        let action = self.lower_value(&args[1])?;
        self.require_direct_callable("mapValues", &action)?;
        let action_slot = self.allocate_stack_object("mapvalues_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));

        // result = tight copy of the map (keys + bucket structure preserved).
        let srcreg = self.temporary_vreg();
        self.emit(abi::load_u64(&srcreg, abi::stack_pointer(), map_slot));
        let result_copy = self.copy_collection_tight(&map_type, &srcreg)?;
        let result_slot = self.allocate_stack_object("mapvalues_result", 8);
        self.emit(abi::store_u64(
            &result_copy,
            abi::stack_pointer(),
            result_slot,
        ));

        let n_slot = self.allocate_stack_object("mapvalues_n", 8);
        let i_slot = self.allocate_stack_object("mapvalues_i", 8);
        let r = self.temporary_vreg();
        let r2 = self.temporary_vreg();
        self.emit(abi::load_u64(&r, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&r2, &r, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&r2, abi::stack_pointer(), n_slot));

        let loop_l = self.label("mapvalues_loop");
        let done_l = self.label("mapvalues_done");
        let ok_l = self.label("mapvalues_ok");
        let entry = self.temporary_vreg();
        let off = self.temporary_vreg();
        let idxoff = self.temporary_vreg();
        let valoff = self.temporary_vreg();
        let base = self.temporary_vreg();
        let resreg = self.temporary_vreg();
        let valaddr = self.temporary_vreg();
        let val = self.temporary_vreg();
        let act = self.temporary_vreg();

        self.emit(abi::move_immediate(&r, "Integer", "0"));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), i_slot));
        self.emit(abi::label(&loop_l));
        self.emit(abi::load_u64(&r, abi::stack_pointer(), i_slot));
        self.emit(abi::load_u64(&r2, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&r, &r2));
        self.emit(abi::branch_ge(&done_l));
        // valAddr = dataBase(result) + entry[i].valueOffset
        self.emit(abi::load_u64(&entry, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(
            &off,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&idxoff, &r, &off));
        self.emit(abi::add_registers(&entry, &entry, &idxoff));
        self.emit(abi::load_u64(
            &valoff,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(&resreg, abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer_for(&base, &resreg, "");
        self.emit(abi::add_registers(&valaddr, &base, &valoff));
        self.emit(abi::load_u64(&val, &valaddr, 0));
        // f(value)
        self.emit(abi::move_register(&abi::argument_register(0)?, &val));
        self.emit(abi::load_u64(&act, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&act);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_l));
        self.emit_callback_failure_exit(Some((result_slot, map_type.clone())))?;
        self.emit(abi::label(&ok_l));
        // Recompute valAddr (the call clobbered caller-saved regs) and store f's result.
        self.emit(abi::load_u64(&r, abi::stack_pointer(), i_slot));
        self.emit(abi::load_u64(&entry, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(
            &off,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&idxoff, &r, &off));
        self.emit(abi::add_registers(&entry, &entry, &idxoff));
        self.emit(abi::load_u64(
            &valoff,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(&resreg, abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer_for(&base, &resreg, "");
        self.emit(abi::add_registers(&valaddr, &base, &valoff));
        self.emit(abi::store_u64(RESULT_VALUE_REGISTER, &valaddr, 0));
        self.emit(abi::add_immediate(&r, &r, 1));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), i_slot));
        self.emit(abi::branch(&loop_l));
        self.emit(abi::label(&done_l));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: map_type.clone(),
            location: Operand::from(result.render()),
            text: format!("mapValues({map_type})"),
        })
    }
}
