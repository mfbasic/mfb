//! Target-generic list-retrieval primitives shared within the `collections`
//! package (plan-96 follow-up: A1 code motion out of `src/target`).
//!
//! These were `CodeBuilder` methods in `the retired flat collection-query helpers`.
//! The census in the git history of `planning/` classified them **A1** — only
//! collection-domain callers (`func_get`/`func_get_or` and sibling collection
//! lowerings), never anything outside the package — so they belong here beside
//! the `func_*.rs` entries that use them, rather than in the shared `src/target`
//! codegen. They stay `impl CodeBuilder` methods (call sites are unchanged); only
//! their defining module moved. Emit-only through `abi::`, so byte-identical to
//! the copies they replaced.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    pub(crate) fn lower_list_get(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        collection_type: &ParameterType,
        element_type: &ParameterType,
        unchecked: bool,
    ) -> Result<ValueResult, String> {
        self.lower_list_get_common(
            collection_slot,
            key_slot,
            None,
            collection_type,
            element_type,
            unchecked,
        )
    }

    /// Shared body of list `get`/`getOr`: bounds-check the index and load the
    /// element. `default_slot` is the `Miss` selector — `None` traps
    /// (index-out-of-range), `Some(slot)` returns the default. Each variant mints
    /// its own label prefix and result text, so output is byte-identical to the two
    /// former standalone functions.
    fn lower_list_get_common(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        default_slot: Option<usize>,
        collection_type: &ParameterType,
        element_type: &ParameterType,
        unchecked: bool,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let collection = self.allocate_register();
        let index = self.allocate_register();
        let count = self.allocate_register();
        let entry_offset = self.allocate_register();
        let entry = self.allocate_register();
        let value_offset = self.allocate_register();
        let value_length = self.allocate_register();
        let (miss, done) = match default_slot {
            None => (self.label("list_get_invalid"), self.label("list_get_done")),
            Some(_) => (
                self.label("list_get_or_default"),
                self.label("list_get_or_done"),
            ),
        };

        self.emit(abi::load_u64(
            &collection,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), key_slot));
        // plan-86 G1: skip the `0 <= index < count` bounds check when the caller has
        // PROVEN the index in range (induction var over `len(L)-k`, L unmodified).
        // `count` is still loaded below by `emit_element_value_offset` as needed.
        if !unchecked {
            self.emit(abi::compare_immediate(&index, "0"));
            self.emit(abi::branch_lt(&miss));
            self.emit(abi::load_u64(&count, &collection, COLLECTION_OFFSET_COUNT));
            self.emit(abi::compare_registers(&index, &count));
            self.emit(abi::branch_ge(&miss));
        }
        self.emit_element_value_offset(
            value_offset,
            value_length,
            &collection,
            &index,
            &entry_offset,
            &entry,
            element_type,
        );
        let result = self.emit_load_collection_payload(
            element_type,
            &collection,
            value_offset,
            value_length,
        )?;
        self.emit(abi::branch(&done));
        self.emit(abi::label(&miss));
        let text = match default_slot {
            None => {
                self.raise_error("collections.get", "ErrIndexOutOfRange")?;
                format!("get({collection_type}, Integer)")
            }
            Some(default_slot) => {
                if *element_type == ParameterType::String {
                    // See `lower_map_get_or`: the found path materializes a fresh
                    // owned string, so the default must be copied too — returning
                    // the alias double-frees it and corrupts the arena.
                    let default_ptr = self.allocate_register();
                    self.emit(abi::load_u64(
                        &default_ptr,
                        abi::stack_pointer(),
                        default_slot,
                    ));
                    let copied = self.emit_copy_owned_string(&default_ptr)?;
                    self.emit(abi::move_register(&result, &copied));
                } else {
                    self.emit(abi::load_u64(&result, abi::stack_pointer(), default_slot));
                }
                format!("getOr({collection_type}, Integer, {element_type})")
            }
        };
        self.emit(abi::label(&done));

        Ok(ValueResult {
            origin: None,
            type_: element_type.clone(),
            location: Operand::from(result.render()),
            text,
        })
    }

    pub(crate) fn lower_list_get_or(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        default_slot: usize,
        collection_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        self.lower_list_get_common(
            collection_slot,
            key_slot,
            Some(default_slot),
            collection_type,
            element_type,
            // getOr returns a default on OOB (no trap), so the elision does not apply.
            false,
        )
    }
}
