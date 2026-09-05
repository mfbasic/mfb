//! The get-result owning copy. Moved out of
//! `the retired flat collection-query helpers`. Shared beyond the
//! collections package (`builder_control` materializes bound elements too), so it
//! lives in the `codegen/memory` data tier. Reads the `borrow_get_result` flag
//! through its accessor and copies via the shared `copy_flat_block` — both stay
//! in `src/target` (the accepted `codegen -> target` edge).
// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::types::ParameterType;
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
        // bug-538: an element whose type reaches a TYPE CYCLE is a pointer-linked
        // graph — the payload in the container's data region holds a pointer to a
        // separately-allocated sub-block. `is_freeable_flat_value` is false for the
        // whole class (it requires `type_is_memcpy_copyable`, and a cycle is never
        // memcpy-copyable), so the copy below never fired and `get` handed back an
        // ALIAS into the container. `.ai/collections.md` records the sibling case:
        // plan-121's gate G24 declines an in-place `removeAt` for this class because
        // the compaction relocates the payload under a fetched value. `append`'s
        // GROW path is a relocation too — it reallocs the data region and frees the
        // old block — so the fetched value dangled and the next read of its
        // recursive field was a use-after-free (SIGSEGV).
        //
        // The fix is the one this function already exists to provide: give the
        // caller an OWNED, independent value. Inline copy codegen cannot reproduce
        // a cyclic graph without unbounded compile-time recursion, so route it
        // through the per-type runtime deep copy `copy_value_to_current_arena`
        // already selects for exactly these types (bug-391's `thread_copy_symbol`,
        // emitted for every recursive type in the module regardless of threads).
        //
        // Excluded, deliberately: a value with a resource anywhere inside it. A
        // resource handle is move-only — copying it would duplicate an OS object
        // with its own close op — so an alias is both the existing and the correct
        // behaviour there.
        if !self.is_freeable_flat_value(&result.type_)
            && crate::codegen::collection::layout::type_reaches_cycle(
                &self.type_model,
                &result.type_,
            )
            && !crate::codegen::collection::layout::type_contains_resource(
                &self.type_model,
                &result.type_,
            )
        {
            let copied = self.copy_value_to_current_arena(&result.type_, &result.location)?;
            return Ok(ValueResult {
                origin: None,
                type_: result.type_,
                location: Operand::from(copied.render()),
                text: result.text,
            });
        }
        if self.is_freeable_flat_value(&result.type_) && result.type_ != ParameterType::String {
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
