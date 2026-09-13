//! plan-134-F: the non-recursive drop of a recursive value — the inverse of
//! `graph_copy.rs`.
//!
//! `mfb spec language memory-semantics` §14.5: "dropping a recursive value recursively
//! drops its owned children … Implementations may use iterative drop internally to avoid
//! stack overflow." `_mfb_rt_graph_drop(kind, pointer)` frees every block of the graph a
//! value of a recursive type owns, one block at a time, with its pending edges on an arena
//! work stack rather than the native stack.
//!
//! * **kinds and work stack** are the copy walker's: the same `recursive_transfer_types`
//!   indices, the same `{count, capacity, entries}` block, the same
//!   `_mfb_rt_graph_stack_grow`. An entry's destination word is unused and pushed as `0`.
//! * **one block** — take an entry; walk the block's pointer edges in the copy's own order,
//!   pushing an edge whose type takes part in a cycle and dropping any other edge inline;
//!   then size the block from its own words and `arena_free` it. Every edge is read before
//!   its parent is freed, and a pushed child is taken only after that, so no freed block is
//!   ever read.
//! * **edges** — the per-block enumerations are the copy's own (`record_pointer_edges`,
//!   `union_variants_by_tag`, `collection_payload_edges` and `payload_edge_shape` in
//!   `builder_arena_transfer.rs`), and the push decision is the copy's
//!   `graph_copy_edge_kind`. The copy allocates one block per block it copies; the drop
//!   frees exactly those. `graph_drop_edges_match_the_copy_edges` pins the pushed edges kind
//!   by kind; `tests/runtime/rt_recursive_value_drop_symmetry.rs` pins the bytes.
//!
//! A kind whose type holds a resource has no arm: a resource is move-only and closed by its
//! own op, `needs_graph_copy` keeps every such value out of the class, and a resource-free
//! type reaches no resource-bearing one, so no resource-free block pushes such a kind.
//!
//! **Test hook.** Until plan-134-G registers drops, no program calls the walker. With
//! `MFB_TEST_GRAPH_DROP` set at build time to a comma-separated list of local names, every
//! `LET` or assignment of such a local whose type is a resource-free cycle member is
//! followed by a deep copy of the value and a `_mfb_rt_graph_drop` of that copy, so a
//! `--debug` build's final `live_bytes` and the program's output must equal the unhooked
//! build's. Unset, the hook emits nothing.

use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::arena::builder_arena_transfer::PayloadEdgeShape;
use crate::codegen::memory::arena::graph_copy::{
    finish_helper, GraphCopyWalker, STACK_ENTRY_OFFSET_KIND, STACK_ENTRY_OFFSET_SOURCE,
    STACK_ENTRY_SIZE, STACK_HEADER_SIZE, STACK_INITIAL_CAPACITY, STACK_OFFSET_CAPACITY,
    STACK_OFFSET_COUNT,
};
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The walker: kind index in argument 0, the pointer to drop in argument 1; returns
/// nothing.
pub(crate) const GRAPH_DROP_SYMBOL: &str = "_mfb_rt_graph_drop";
/// plan-134-H: the same walker for a ROOT that is not a block of its own — a record or union
/// element inlined in a collection's data region. It frees everything the root's edges own
/// and leaves the root's bytes, which belong to the collection's block.
pub(crate) const GRAPH_DROP_EDGES_SYMBOL: &str = "_mfb_rt_graph_drop_edges";

/// The test hook's environment variable (see the module comment).
const TEST_GRAPH_DROP_ENV: &str = "MFB_TEST_GRAPH_DROP";

/// plan-134-G: how a construction store writes the value it is given
/// (`CodeBuilder::lower_value_stored` and its siblings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoreShape {
    /// A collection payload: a pointer word for a collection/`Result`/`Error` payload, the
    /// block's bytes for a record or union payload (`payload_edge_shape`).
    Payload,
    /// The value's pointer is kept: a record field (a recursive field is never inlined) or a
    /// resource `STATE`.
    Pointer,
    /// The value's block is byte-copied: a variant record wrapped into a union.
    Inline,
}

/// The local names `MFB_TEST_GRAPH_DROP` lists, read once per compiler process.
fn test_graph_drop_locals() -> &'static [String] {
    static LOCALS: OnceLock<Vec<String>> = OnceLock::new();
    LOCALS.get_or_init(|| {
        std::env::var(TEST_GRAPH_DROP_ENV)
            .map(|names| {
                names
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    })
}

impl CodeBuilder<'_> {
    /// The walker kind a value of `type_` is dropped with: its index in
    /// `TypeModel::graph_drop_kinds` — every type of `recursive_transfer_types` (in that order,
    /// so a kind is the copy walker's index too), then the resource-free types that only reach
    /// a cycle (plan-134-H). `None` for any other type.
    pub(crate) fn graph_drop_kind(&self, type_: &ParameterType) -> Option<usize> {
        // A resource-bearing value is move-only and closed by its own op: never a drop kind.
        if type_contains_resource(&self.type_model, type_) {
            return None;
        }
        let rendered = type_.name();
        self.type_model
            .graph_drop_kinds
            .iter()
            .position(|kind| kind.as_str() == rendered.as_ref())
    }

    /// plan-134-H: before an in-place arm discards list element `index_slot` of the list in
    /// `buffer_slot`, free the graph that element owns (nothing for a flat element type).
    pub(crate) fn emit_drop_list_element(
        &mut self,
        buffer_slot: usize,
        index_slot: usize,
        element_type: &ParameterType,
    ) -> Result<(), String> {
        if !self.owns_graph(element_type) {
            return Ok(());
        }
        let entry_slot = self.allocate_stack_object("drop_element_entry", 8);
        let block = self.temporary_vreg();
        let index = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::move_immediate(
            &scratch,
            "Integer",
            &list_entry_stride(element_type).to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch, &index, &scratch));
        self.emit(abi::add_immediate(
            &scratch,
            &scratch,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch, &block, &scratch));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), entry_slot));
        self.emit_drop_entry_value(buffer_slot, entry_slot, element_type)
    }

    /// plan-134-H: free the graph owned by the value payload of the entry whose address is in
    /// `entry_slot`, in the collection whose block is in `collection_slot` — through
    /// `_mfb_rt_graph_drop` for a pointer payload, `_mfb_rt_graph_drop_edges` for a record or
    /// union inlined in the data region (its bytes stay with the collection). Nothing for a flat
    /// value type.
    pub(crate) fn emit_drop_entry_value(
        &mut self,
        collection_slot: usize,
        entry_slot: usize,
        value_type: &ParameterType,
    ) -> Result<(), String> {
        if !self.owns_graph(value_type) {
            return Ok(());
        }
        let kind = self
            .graph_drop_kind(value_type)
            .ok_or_else(|| format!("the graph drop has no kind for '{value_type}'"))?;
        let shape = self.payload_edge_shape(value_type);
        let symbol = match shape {
            Some(PayloadEdgeShape::Pointer) => GRAPH_DROP_SYMBOL,
            Some(PayloadEdgeShape::InlineRecord | PayloadEdgeShape::InlineUnion) => {
                GRAPH_DROP_EDGES_SYMBOL
            }
            None => return Ok(()),
        };
        let payload_slot = self.allocate_stack_object("drop_element_payload", 8);
        let block = self.temporary_vreg();
        let data = self.temporary_vreg();
        let offset = self.temporary_vreg();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), collection_slot));
        // Kind-0 stride: a value that owns a graph is never fixed-width, so its collection
        // has an entry table (as `fix_collection_transfer_payload` assumes).
        self.emit_collection_data_pointer_for(&data, &block, &ParameterType::named(""));
        self.emit(abi::load_u64(&offset, abi::stack_pointer(), entry_slot));
        self.emit(abi::load_u64(
            &offset,
            &offset,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::add_registers(&data, &data, &offset));
        if shape == Some(PayloadEdgeShape::Pointer) {
            self.emit(abi::load_u64(&data, &data, 0));
        }
        self.emit(abi::store_u64(&data, abi::stack_pointer(), payload_slot));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            payload_slot,
        ));
        self.emit(abi::move_immediate(
            abi::c_arg(0),
            "Integer",
            &kind.to_string(),
        ));
        self.emit_symbol_call(symbol);
        Ok(())
    }

    /// plan-134-G: drop the recursive value whose pointer is in `slot` through the walker, then
    /// zero the slot — null-guarded and free-and-null, like the flat drop (bug-440).
    pub(crate) fn emit_graph_value_drop(
        &mut self,
        type_: &ParameterType,
        slot: usize,
    ) -> Result<(), String> {
        let kind = self
            .graph_drop_kind(type_)
            .ok_or_else(|| format!("the graph drop has no kind for '{type_}'"))?;
        let skip = self.label("graph_value_drop_skip");
        self.emit(abi::load_u64(abi::c_arg(1), abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(abi::c_arg(1), "0"));
        self.emit(abi::branch_eq(&skip));
        self.emit(abi::move_immediate(
            abi::c_arg(0),
            "Integer",
            &kind.to_string(),
        ));
        self.emit_symbol_call(GRAPH_DROP_SYMBOL);
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
        self.emit(abi::label(&skip));
        Ok(())
    }

    /// plan-134-G: free only the block whose pointer is in `slot` — a recursive record or union
    /// whose bytes a store copied, so the graph it points into now belongs to that store's
    /// owner. Null-guarded and free-and-null.
    pub(crate) fn emit_shallow_block_free(
        &mut self,
        type_: &ParameterType,
        slot: usize,
    ) -> Result<(), String> {
        let skip = self.label("shallow_block_free_skip");
        let pointer = self.temporary_vreg();
        self.emit(abi::load_u64(&pointer, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&pointer, "0"));
        self.emit(abi::branch_eq(&skip));
        let size_slot = self.allocate_stack_object("shallow_block_size", 8);
        self.emit_inlined_block_size_from_ptr_slot(type_, slot, size_slot)?;
        self.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), slot));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit_arena_free_call();
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
        self.emit(abi::label(&skip));
        Ok(())
    }

    /// The test hook, after a `LET` or assignment of local `name` (see the module comment).
    pub(crate) fn emit_test_graph_drop_hook(&mut self, name: &str) -> Result<(), String> {
        if !test_graph_drop_locals().iter().any(|local| local == name) {
            return Ok(());
        }
        let Some(local) = self.locals.get(name) else {
            return Ok(());
        };
        if local.by_ref {
            return Ok(());
        }
        let type_ = local.type_.clone();
        let Some(kind) = self.graph_drop_kind(&type_) else {
            return Ok(());
        };
        let value = self.lower_value(&NirValue::Local(name.to_string()))?;
        let slot = self.allocate_stack_object("graph_drop_hook_copy", 8);
        let absent = self.label("graph_drop_hook_absent");
        self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
        let source = self.temporary_vreg();
        self.emit(abi::load_u64(&source, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&source, "0"));
        self.emit(abi::branch_eq(&absent));
        let copied = self.copy_value_to_current_arena(&type_, &source)?;
        self.emit(abi::store_u64(&copied, abi::stack_pointer(), slot));
        self.emit(abi::load_u64(abi::c_arg(1), abi::stack_pointer(), slot));
        self.emit(abi::move_immediate(
            abi::c_arg(0),
            "Integer",
            &kind.to_string(),
        ));
        self.emit_symbol_call(GRAPH_DROP_SYMBOL);
        self.emit(abi::label(&absent));
        Ok(())
    }

    /// Drop the one block of `type_` whose non-null pointer is in `pointer_slot`: its
    /// pointer edges, then the block itself. The arms follow `emit_thread_copy_real`'s, so
    /// every block the copy allocates for a shape is the block freed for it here.
    fn emit_graph_drop_block(
        &mut self,
        type_: &ParameterType,
        pointer_slot: usize,
    ) -> Result<(), String> {
        if self.emit_graph_drop_edges(type_, pointer_slot)? {
            self.emit_graph_free_block(type_, pointer_slot)?;
        }
        Ok(())
    }

    /// The pointer edges of the one block of `type_` whose non-null pointer is in
    /// `pointer_slot`, without freeing the block itself. `Ok(true)` when `type_` is a block (so
    /// its caller frees it), `Ok(false)` for a scalar word.
    fn emit_graph_drop_edges(
        &mut self,
        type_: &ParameterType,
        pointer_slot: usize,
    ) -> Result<bool, String> {
        if matches!(
            type_,
            ParameterType::Nothing
                | ParameterType::Boolean
                | ParameterType::Byte
                | ParameterType::Integer
                | ParameterType::Float
                | ParameterType::Fixed
                | ParameterType::Money
        ) || type_.is_named("Scalar")
        {
            // Not a block: the copy moves the word.
            return Ok(false);
        }
        if type_contains_resource(&self.type_model, type_) {
            return Err(format!(
                "the graph drop cannot drop '{type_}': it holds a resource"
            ));
        }
        // A pointer-free block (copied by `copy_flat_block`).
        if self.type_is_memcpy_copyable(type_) {
            return Ok(true);
        }
        if typed_is_collection_type(type_) {
            for (payload_type, key_payload) in self.collection_payload_edges(type_)? {
                self.emit_graph_drop_collection_payload(pointer_slot, &payload_type, key_payload)?;
            }
            return Ok(true);
        }
        if self
            .type_model
            .union_names
            .contains(&ParameterType::declared(&type_.without_state().name()))
        {
            if !self.union_is_data(type_) {
                return Err(format!(
                    "the graph drop cannot drop '{type_}': a resource union"
                ));
            }
            let base = self.temporary_vreg();
            self.emit(abi::load_u64(&base, abi::stack_pointer(), pointer_slot));
            self.emit_graph_drop_union_edges(type_, &base)?;
            return Ok(true);
        }
        if self.type_model.record_fields.contains_key(type_) {
            let base = self.temporary_vreg();
            self.emit(abi::load_u64(&base, abi::stack_pointer(), pointer_slot));
            self.emit_graph_drop_record_edges(type_, &base)?;
            return Ok(true);
        }
        Err(format!(
            "the graph drop cannot drop a value of type '{type_}'"
        ))
    }

    /// One pointer edge of type `type_`: pushed when its type takes part in a cycle, else
    /// dropped here (null skipped).
    fn emit_graph_drop_edge(
        &mut self,
        type_: &ParameterType,
        edge: impl Into<Operand>,
    ) -> Result<(), String> {
        if self.graph_copy_walker.is_none() {
            return Err("graph drop edge emitted outside the walker".to_string());
        }
        if let Some(kind) = self.graph_copy_edge_kind(type_)? {
            return self.emit_graph_copy_push(kind, edge, None);
        }
        let slot = self.allocate_stack_object("graph_drop_edge", 8);
        let absent = self.label("graph_drop_edge_absent");
        let pointer = self.temporary_vreg();
        self.emit(abi::store_u64(edge, abi::stack_pointer(), slot));
        self.emit(abi::load_u64(&pointer, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&pointer, "0"));
        self.emit(abi::branch_eq(&absent));
        self.emit_graph_drop_block(type_, slot)?;
        self.emit(abi::label(&absent));
        Ok(())
    }

    /// The pointer edges of a record whose base is `base` (a block of its own, or one
    /// inlined in a union or a collection's data region).
    fn emit_graph_drop_record_edges(
        &mut self,
        type_: &ParameterType,
        base: impl Into<Operand>,
    ) -> Result<(), String> {
        let edges = self.record_pointer_edges(type_)?;
        let base_slot = self.allocate_stack_object("graph_drop_record_base", 8);
        self.emit(abi::store_u64(base, abi::stack_pointer(), base_slot));
        for (offset, field_type) in &edges {
            let edge = self.temporary_vreg();
            self.emit(abi::load_u64(&edge, abi::stack_pointer(), base_slot));
            self.emit(abi::load_u64(&edge, &edge, *offset));
            self.emit_graph_drop_edge(field_type, &edge)?;
        }
        Ok(())
    }

    /// The pointer edges of a data union whose base is `base`: the active variant's record,
    /// inlined at +16.
    fn emit_graph_drop_union_edges(
        &mut self,
        type_: &ParameterType,
        base: impl Into<Operand>,
    ) -> Result<(), String> {
        let variants = self.union_variants_by_tag(&type_.without_state())?;
        let base_slot = self.allocate_stack_object("graph_drop_union_base", 8);
        let done = self.label("graph_drop_union_done");
        let labels: Vec<String> = variants
            .iter()
            .map(|_| self.label("graph_drop_union_variant"))
            .collect();
        let tag = self.temporary_vreg();
        self.emit(abi::store_u64(base, abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64(&tag, abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64(&tag, &tag, 0));
        for ((_, variant_tag, _), label) in variants.iter().zip(&labels) {
            self.emit(abi::compare_immediate(&tag, &variant_tag.to_string()));
            self.emit(abi::branch_eq(label));
        }
        self.emit(abi::branch(&done));
        for ((variant, _, _), label) in variants.iter().zip(&labels) {
            self.emit(abi::label(label));
            let inner = self.temporary_vreg();
            self.emit(abi::load_u64(&inner, abi::stack_pointer(), base_slot));
            self.emit(abi::add_immediate(&inner, &inner, 16));
            self.emit_graph_drop_record_edges(variant, &inner)?;
            self.emit(abi::branch(&done));
        }
        self.emit(abi::label(&done));
        Ok(())
    }

    /// The edges of one payload (the key or the value) of every live entry of the
    /// collection whose pointer is in `block_slot` — `fix_collection_transfer_payload`'s
    /// walk.
    fn emit_graph_drop_collection_payload(
        &mut self,
        block_slot: usize,
        payload_type: &ParameterType,
        key_payload: bool,
    ) -> Result<(), String> {
        let sp = abi::stack_pointer();
        let index_slot = self.allocate_stack_object("graph_drop_collection_index", 8);
        let payload_slot = self.allocate_stack_object("graph_drop_collection_payload", 8);
        let loop_label = self.label("graph_drop_collection_loop");
        let done_label = self.label("graph_drop_collection_done");
        let entry_offset = if key_payload {
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET
        } else {
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET
        };
        let zero = self.temporary_vreg();
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::store_u64(&zero, sp, index_slot));
        self.emit(abi::label(&loop_label));
        // The live entries `[0..count)` only (bug-146).
        let index = self.temporary_vreg();
        let block = self.temporary_vreg();
        let count = self.temporary_vreg();
        self.emit(abi::load_u64(&index, sp, index_slot));
        self.emit(abi::load_u64(&block, sp, block_slot));
        self.emit(abi::load_u64(&count, &block, COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&done_label));
        let entry = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        self.emit(abi::move_immediate(
            &scratch,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&entry, &index, &scratch));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&entry, &block, &entry));
        // Kind-0 stride, as the copy's walk: only a block with an entry table gets here.
        let data = self.temporary_vreg();
        self.emit_collection_data_pointer_for(&data, &block, &ParameterType::named(""));
        self.emit(abi::load_u64(&scratch, &entry, entry_offset));
        self.emit(abi::add_registers(&data, &data, &scratch));
        self.emit(abi::store_u64(&data, sp, payload_slot));
        match self.payload_edge_shape(payload_type) {
            Some(PayloadEdgeShape::Pointer) => {
                let edge = self.temporary_vreg();
                self.emit(abi::load_u64(&edge, sp, payload_slot));
                self.emit(abi::load_u64(&edge, &edge, 0));
                self.emit_graph_drop_edge(payload_type, &edge)?;
            }
            Some(PayloadEdgeShape::InlineRecord) => {
                let base = self.temporary_vreg();
                self.emit(abi::load_u64(&base, sp, payload_slot));
                self.emit_graph_drop_record_edges(payload_type, &base)?;
            }
            Some(PayloadEdgeShape::InlineUnion) => {
                let base = self.temporary_vreg();
                self.emit(abi::load_u64(&base, sp, payload_slot));
                self.emit_graph_drop_union_edges(payload_type, &base)?;
            }
            None => {}
        }
        let index = self.temporary_vreg();
        self.emit(abi::load_u64(&index, sp, index_slot));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::store_u64(&index, sp, index_slot));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }

    /// `arena_free` the block of `type_` whose pointer is in `pointer_slot`, sized from its
    /// own words by the same sizer the copy allocated it with.
    fn emit_graph_free_block(
        &mut self,
        type_: &ParameterType,
        pointer_slot: usize,
    ) -> Result<(), String> {
        let size_slot = self.allocate_stack_object("graph_drop_size", 8);
        self.emit_inlined_block_size_from_ptr_slot(type_, pointer_slot, size_slot)?;
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            pointer_slot,
        ));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit_arena_free_call();
        Ok(())
    }
}

/// `_mfb_rt_graph_drop(kind, pointer)` (`free_root`) or `_mfb_rt_graph_drop_edges(kind,
/// pointer)` (not `free_root`, plan-134-H): the module's drops for values of recursive types.
/// `kinds` is `TypeModel::graph_drop_kinds`. The edges variant frees everything the root's
/// edges own but not the root's own bytes, which belong to the collection block it is inlined
/// in: its first entry taken skips the block free, every pushed entry frees as usual.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_graph_drop_walker(
    symbol: &str,
    free_root: bool,
    kinds: &[String],
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &crate::target::shared::nir::NirFunction>,
    package_return_types: &HashMap<String, ParameterType>,
    platform_imports: &HashMap<String, String>,
    platform: &dyn crate::codegen::engine::types::CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &HashMap<String, GlobalValue>,
    string_symbols: &HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let mut builder = CodeBuilder::for_synthetic_function(
        symbol,
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals,
        string_symbols,
        type_model,
    );
    let sp = abi::stack_pointer();
    let kind_slot = builder.allocate_stack_object("graph_drop_kind", 8);
    let pointer_slot = builder.allocate_stack_object("graph_drop_pointer", 8);
    let stack_slot = builder.allocate_stack_object("graph_drop_stack", 8);
    // The edges variant: 1 until the root entry has been taken.
    let root_slot = (!free_root).then(|| builder.allocate_stack_object("graph_drop_root", 8));
    let alloc_ok = builder.label("graph_drop_stack_alloc_ok");
    let take = builder.label("graph_drop_take");
    let pop = builder.label("graph_drop_pop");
    let finish = builder.label("graph_drop_finish");
    let droppable: Vec<(usize, ParameterType)> = kinds
        .iter()
        .enumerate()
        .map(|(index, name)| (index, ParameterType::declared(name)))
        .filter(|(_, type_)| !type_contains_resource(&builder.type_model, type_))
        .collect();
    let arm_labels: Vec<String> = droppable
        .iter()
        .map(|_| builder.label("graph_drop_kind"))
        .collect();

    let kind_in = builder.allocate_register();
    let pointer_in = builder.allocate_register();
    builder.emit(abi::move_register(&kind_in, abi::c_arg(0)));
    builder.emit(abi::move_register(&pointer_in, abi::c_arg(1)));
    builder.emit(abi::store_u64(&kind_in, sp, kind_slot));
    builder.emit(abi::store_u64(&pointer_in, sp, pointer_slot));
    if let Some(root_slot) = root_slot {
        let one = builder.temporary_vreg();
        builder.emit(abi::move_immediate(&one, "Integer", "1"));
        builder.emit(abi::store_u64(&one, sp, root_slot));
    }

    // The work stack: header + 64 entries, as the copy walker's.
    builder.emit(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        &(STACK_HEADER_SIZE + STACK_INITIAL_CAPACITY * STACK_ENTRY_SIZE).to_string(),
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&alloc_ok));
    builder.emit(abi::store_u64(abi::mfb_return(1), sp, stack_slot));
    let block = builder.temporary_vreg();
    let scratch = builder.temporary_vreg();
    builder.emit(abi::load_u64(&block, sp, stack_slot));
    builder.emit(abi::move_immediate(&scratch, "Integer", "0"));
    builder.emit(abi::store_u64(&scratch, &block, STACK_OFFSET_COUNT));
    builder.emit(abi::move_immediate(
        &scratch,
        "Integer",
        &STACK_INITIAL_CAPACITY.to_string(),
    ));
    builder.emit(abi::store_u64(&scratch, &block, STACK_OFFSET_CAPACITY));

    builder.graph_copy_walker = Some(GraphCopyWalker::new(kinds, stack_slot));

    // Take one entry: a null edge owns nothing; otherwise dispatch on the kind.
    builder.emit(abi::label(&take));
    let pointer = builder.temporary_vreg();
    builder.emit(abi::load_u64(&pointer, sp, pointer_slot));
    builder.emit(abi::compare_immediate(&pointer, "0"));
    builder.emit(abi::branch_eq(&pop));
    let kind = builder.temporary_vreg();
    builder.emit(abi::load_u64(&kind, sp, kind_slot));
    for ((index, _), label) in droppable.iter().zip(&arm_labels) {
        builder.emit(abi::compare_immediate(&kind, &index.to_string()));
        builder.emit(abi::branch_eq(label));
    }
    // Only a resource-bearing kind has no arm, and no droppable block pushes one.
    builder.emit(abi::branch(&pop));
    for ((_, type_), label) in droppable.iter().zip(&arm_labels) {
        builder.emit(abi::label(label));
        if builder.emit_graph_drop_edges(type_, pointer_slot)? {
            match root_slot {
                None => builder.emit_graph_free_block(type_, pointer_slot)?,
                Some(root_slot) => {
                    // The root's bytes are the collection's: only a pushed entry is freed.
                    let root = builder.label("graph_drop_edges_root");
                    let flag = builder.temporary_vreg();
                    builder.emit(abi::load_u64(&flag, sp, root_slot));
                    builder.emit(abi::compare_immediate(&flag, "0"));
                    builder.emit(abi::branch_ne(&root));
                    builder.emit_graph_free_block(type_, pointer_slot)?;
                    builder.emit(abi::label(&root));
                }
            }
        }
        builder.emit(abi::branch(&pop));
    }

    // Pop the next entry into the frame slots, or finish. Every entry after the first is a
    // pushed edge, never the root.
    builder.emit(abi::label(&pop));
    if let Some(root_slot) = root_slot {
        builder.emit(abi::store_u64(abi::ZERO, sp, root_slot));
    }
    let block = builder.temporary_vreg();
    let count = builder.temporary_vreg();
    let entry = builder.temporary_vreg();
    let scratch = builder.temporary_vreg();
    builder.emit(abi::load_u64(&block, sp, stack_slot));
    builder.emit(abi::load_u64(&count, &block, STACK_OFFSET_COUNT));
    builder.emit(abi::compare_immediate(&count, "0"));
    builder.emit(abi::branch_eq(&finish));
    builder.emit(abi::subtract_immediate(&count, &count, 1));
    builder.emit(abi::store_u64(&count, &block, STACK_OFFSET_COUNT));
    builder.emit(abi::move_immediate(
        &scratch,
        "Integer",
        &STACK_ENTRY_SIZE.to_string(),
    ));
    builder.emit(abi::multiply_registers(&entry, &count, &scratch));
    builder.emit(abi::add_immediate(&entry, &entry, STACK_HEADER_SIZE));
    builder.emit(abi::add_registers(&entry, &block, &entry));
    builder.emit(abi::load_u64(&scratch, &entry, STACK_ENTRY_OFFSET_KIND));
    builder.emit(abi::store_u64(&scratch, sp, kind_slot));
    builder.emit(abi::load_u64(&scratch, &entry, STACK_ENTRY_OFFSET_SOURCE));
    builder.emit(abi::store_u64(&scratch, sp, pointer_slot));
    builder.emit(abi::branch(&take));

    // Free the work stack (header + capacity * entry).
    builder.emit(abi::label(&finish));
    builder.graph_copy_walker = None;
    let block = builder.temporary_vreg();
    let size = builder.temporary_vreg();
    let scratch = builder.temporary_vreg();
    builder.emit(abi::load_u64(&block, sp, stack_slot));
    builder.emit(abi::load_u64(&size, &block, STACK_OFFSET_CAPACITY));
    builder.emit(abi::move_immediate(
        &scratch,
        "Integer",
        &STACK_ENTRY_SIZE.to_string(),
    ));
    builder.emit(abi::multiply_registers(&size, &size, &scratch));
    builder.emit(abi::add_immediate(&size, &size, STACK_HEADER_SIZE));
    builder.emit(abi::move_register(abi::c_arg(0), &block));
    builder.emit(abi::move_register(abi::c_arg(1), &size));
    builder.emit_arena_free_call();
    builder.emit(abi::return_());

    let name = if free_root {
        "runtime.graphDrop"
    } else {
        "runtime.graphDropEdges"
    };
    finish_helper(builder, name, symbol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::engine::tests::test_support::TestPlatform;
    use crate::codegen::memory::arena::graph_copy::tests::{
        calls_and_pushes, field, harness, json_like_types, model, pushed_kinds, record, union,
    };

    /// For kind `name`: the kinds the drop walker's body pushes, sorted. Also asserts the
    /// body calls neither a per-type copy nor either walker — no native recursion.
    fn drop_pushes(model: &TypeModel, name: &str) -> Vec<String> {
        let kinds: Vec<String> = recursive_transfer_types(model).into_iter().collect();
        let platform = TestPlatform;
        let harness = harness(model);
        let mut walker = harness.builder("drop", &platform);
        let stack_slot = walker.allocate_stack_object("stack", 8);
        walker.graph_copy_walker = Some(GraphCopyWalker::new(&kinds, stack_slot));
        let pointer_slot = walker.allocate_stack_object("pointer", 8);
        walker
            .emit_graph_drop_block(&ParameterType::declared(name), pointer_slot)
            .unwrap_or_else(|error| panic!("{name}: the drop body emits: {error}"));
        let per_type: Vec<String> = kinds
            .iter()
            .map(|kind| thread_copy_symbol(&ParameterType::declared(kind)))
            .collect();
        let calls: Vec<&String> = walker
            .relocations
            .iter()
            .map(|relocation| &relocation.to)
            .filter(|to| {
                per_type.contains(to)
                    || to.as_str() == GRAPH_DROP_SYMBOL
                    || to.as_str() == crate::codegen::memory::arena::graph_copy::GRAPH_COPY_SYMBOL
            })
            .collect();
        assert!(
            calls.is_empty(),
            "{name}: the drop body calls back into a copy or a walker: {calls:?}"
        );
        pushed_kinds(&walker, &kinds)
    }

    /// Every resource-free kind of `model`: its drop pushes exactly the edges its copy
    /// pushes, and only resource-free kinds. Returns the number of edges seen.
    fn assert_drop_edges_match(label: &str, model: &TypeModel) -> usize {
        let kinds = recursive_transfer_types(model);
        assert!(
            !kinds.is_empty(),
            "{label}: the model has no recursive type"
        );
        let mut edges = 0;
        for kind in &kinds {
            if type_contains_resource(model, &ParameterType::declared(kind)) {
                continue;
            }
            let (_, copy_pushes) = calls_and_pushes(model, kind);
            let drop_pushes = drop_pushes(model, kind);
            assert_eq!(
                drop_pushes, copy_pushes,
                "{label}: kind {kind}: the drop must push exactly the edges the copy pushes"
            );
            for pushed in &drop_pushes {
                assert!(
                    !type_contains_resource(model, &ParameterType::declared(pushed)),
                    "{label}: kind {kind} pushes resource-bearing kind {pushed}, which has no arm"
                );
            }
            edges += drop_pushes.len();
        }
        edges
    }

    /// The drop walker visits exactly the edges the copy walker visits, per kind, over the
    /// hand-built models of `graph_copy_edges_match_the_copy_calls` and a recursive user
    /// union.
    #[test]
    fn graph_drop_edges_match_the_copy_edges() {
        let node = model(vec![record(
            "Node",
            vec![field("kids", "List OF Node"), field("tag", "Integer")],
        )]);
        let json = model(json_like_types());
        let mut bridged_types = json_like_types();
        bridged_types.push(record(
            "Node2",
            vec![
                field("kids", "List OF Node2"),
                field("extra", "Map OF Integer TO J"),
            ],
        ));
        let bridged = model(bridged_types);
        let leaf = || vec![field("v", "Integer")];
        let pair = || vec![field("left", "Tree"), field("right", "Tree")];
        let tree = model(vec![
            record("Leaf", leaf()),
            record("Pair", pair()),
            union("Tree", vec![("Leaf", leaf()), ("Pair", pair())]),
        ]);
        for (label, model) in [
            ("TYPE Node (kids AS List OF Node)", &node),
            ("a Json-shaped recursive union", &json),
            ("a cycle member reaching a second cycle", &bridged),
            ("UNION Tree (Pair holds two Tree fields)", &tree),
        ] {
            let edges = assert_drop_edges_match(label, model);
            assert!(
                edges > 0,
                "{label}: no edges at all — not looking at a cycle"
            );
        }
    }

    /// The same table over the builtin recursive types, from the models of the
    /// recursive-value bench programs that use them: `json::Json`, and the regex engine's
    /// `__regex_Node`, `__regex_Cont` and `__regex_Choices`.
    #[test]
    fn graph_drop_edges_match_the_copy_edges_for_the_builtin_types() {
        use crate::testutil::{nir_for_src, CodeTarget};
        for (label, source, expected) in [
            (
                "json_repeat",
                include_str!(
                    "../../../../tools/recursive-value-bench/programs/json_repeat/src/main.mfb"
                ),
                &["Json"][..],
            ),
            (
                "regex_repeat",
                include_str!(
                    "../../../../tools/recursive-value-bench/programs/regex_repeat/src/main.mfb"
                ),
                // The package's private types render with its internal `#` prefix.
                &["#regex_Node", "#regex_Cont", "#regex_Choices"][..],
            ),
        ] {
            let module = nir_for_src(
                source,
                CodeTarget::MacosAarch64,
                crate::target::NativeBuildMode::Console,
            )
            .unwrap_or_else(|error| panic!("{label}: the program lowers to NIR: {error}"));
            let model = TypeModel::from_module(&module).expect("the program's type model builds");
            let kinds = recursive_transfer_types(&model);
            for name in expected {
                assert!(
                    kinds.iter().any(|kind| kind.ends_with(name)),
                    "{label}: `{name}` is not a recursive kind: {kinds:?}"
                );
            }
            let edges = assert_drop_edges_match(label, &model);
            assert!(
                edges > 0,
                "{label}: no edges at all — not looking at a cycle"
            );
        }
    }
}
