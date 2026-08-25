//! Target-generic set/map membership primitive shared within the `collections`
//! package (plan-96 follow-up: A1 code motion out of `src/target`).
//!
//! `emit_key_membership` was a `CodeBuilder` method in
//! `the retired flat collection-query helpers`. The census in the git
//! history of `planning/` classified it **A1** — its only callers are collection
//! lowerings (`func_has_key`, `func_contains`, and the Set/Map membership paths
//! in `builder_collection_queries.rs`). Membership is a Set's defining operation,
//! so it lives here; `collections::hasKey` on a Map reuses the same byte-compare
//! because a Set's element *is* its entry key.
//!
//! It stays an `impl CodeBuilder` method (call sites unchanged); only the
//! defining module moved. It delegates to the map probe/scan helpers now in
//! [`super::gen_map`] and the shared payload-compare branch in `src/target`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    /// The shared Map/Set membership test: probe the FNV-1a bucket index for a
    /// probe-eligible key type, else linear-scan the entry keys, yielding a
    /// `Boolean`. Both `collections::hasKey` (Map) and the Set overload of
    /// `collections::contains` (plan-63-B) lower through here — a Set's element is
    /// its entry key, so the byte-compare is identical. `collection_slot` holds the
    /// collection pointer and `key_slot` the needle, both already spilled.
    pub(crate) fn emit_key_membership(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        key_type: &str,
        label_prefix: &str,
        collection_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();

        if Self::map_key_probe_eligible(key_type) {
            self.reset_temporary_registers();
            let not_found = self.label(&format!("{label_prefix}_not_found"));
            let done = self.label(&format!("{label_prefix}_done"));
            let _ = self.emit_map_probe(collection_slot, key_slot, key_type, &not_found)?;
            let result = self.allocate_register()?;
            self.emit(abi::move_immediate(&result, "Boolean", "true"));
            self.emit(abi::branch(&done));
            self.emit(abi::label(&not_found));
            self.emit(abi::move_immediate(&result, "Boolean", "false"));
            self.emit(abi::label(&done));
            return Ok(ValueResult {
                origin: None,
                type_: ParameterType::Boolean,
                location: Operand::from(result.render()),
                text: format!("{label_prefix}({collection_type}) [hash]"),
            });
        }

        self.reset_temporary_registers();
        let result = self.allocate_register()?;
        let loop_label = self.label(&format!("{label_prefix}_loop"));
        let found = self.label(&format!("{label_prefix}_found"));
        let next = self.label(&format!("{label_prefix}_next"));
        let not_found = self.label(&format!("{label_prefix}_not_found"));
        let done = self.label(&format!("{label_prefix}_done"));

        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), key_slot));
        self.emit_entry_scan_setup(
            &scratch8,
            &scratch10,
            &scratch11,
            &scratch12,
            &scratch13,
            &scratch14,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            &loop_label,
            &not_found,
        );
        self.emit_collection_payload_matches_value_branch(
            key_type, "", &scratch8, &scratch13, &scratch14, &scratch9, &found, &next,
        )?;
        self.emit(abi::label(&found));
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::branch(&done));
        self.emit_entry_scan_advance(&scratch12, &scratch11, &next, &loop_label);
        self.emit(abi::label(&not_found));
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::label(&done));

        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Boolean,
            location: Operand::from(result.render()),
            text: format!("{label_prefix}({collection_type}, {key_type})"),
        })
    }
}
