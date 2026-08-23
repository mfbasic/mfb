//! The get-result owning copy. Moved out of
//! `the retired flat collection-query helpers`. Shared beyond the
//! collections package (`builder_control` materializes bound elements too), so it
//! lives in the `codegen/memory` data tier. Reads the `borrow_get_result` flag
//! through its accessor and copies via the shared `copy_flat_block` — both stay
//! in `src/target` (the accepted `codegen -> target` edge).
// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
impl CodeBuilder<'_> {
    /// `collections::get`/`getOr` extract an element as an alias into the
    /// container's data region for inline composite / nested-collection payloads
    /// (`emit_load_collection_payload`). By value semantics `get` returns an
    /// **owned** value the caller may bind, store, and free, so copy such a
    /// alias into a standalone arena block (scalars are by-value and `String`
    /// is already materialized fresh, so they pass through). plan-02 Phase 8.
    pub(crate) fn materialize_owned_element(
        &mut self,
        result: ValueResult,
    ) -> Result<ValueResult, String> {
        // plan-86 E: the enclosing `LET e = get(L, i)` binding is consumed read-only
        // (only a MATCH scrutinee) over an immutable container, so `e` may alias the
        // container's inline element — skip the owning copy. The Bind arm gates this
        // to the freeable-flat-non-String element type and suppresses the scope-drop
        // free on the SAME condition, so the alias is never freed (a freed borrow is
        // a double-free into the container).
        if self.borrow_get_result() {
            return Ok(result);
        }
        if self.is_freeable_flat_value(&result.type_) && result.type_ != "String" {
            let copied = self.copy_flat_block(&result.type_, &result.location)?;
            return Ok(ValueResult {
                origin: None,
                type_: result.type_,
                location: Operand::from(copied.render()),
                text: result.text,
            });
        }
        Ok(result)
    }
}
