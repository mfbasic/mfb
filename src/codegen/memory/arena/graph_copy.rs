//! plan-134-B: the non-recursive deep copy of a recursive value.
//!
//! A value of a type that takes part in a type cycle (`TYPE Node / kids AS List OF
//! Node`, `json::Json`, the regex engine's `__regex_Choices`) is a graph of separately
//! allocated blocks whose depth is data, not type. Its deep copy used to be one function
//! per type that called itself once per edge on the native stack, so a chain about
//! 60 000 levels deep overflowed the 8 MiB stack (plan-134-A §2.1) — and the regex
//! backtracker alone builds `__regex_Choices` chains up to 500 000 deep.
//!
//! This module emits one module-level walker, `_mfb_rt_graph_copy(kind, source)`, that
//! keeps its pending edges on a work stack in the arena instead:
//!
//! * **kinds** — every member of `recursive_transfer_types`, indexed in that set's
//!   (sorted, deterministic) order. The module emitter hands the same order to the
//!   per-type shims and to the walker, so the number a shim passes is the number the
//!   walker dispatches on.
//! * **work stack** — one arena block `{count @0, capacity @8, entries @16}`, each entry
//!   `{kind @0, source @8, destination address @16}` (24 bytes). It starts at 64 entries
//!   and `_mfb_rt_graph_stack_grow` doubles it.
//! * **loop** — take an entry, copy that ONE block with `emit_thread_copy_real` (the same
//!   per-shape copy as before), write the new pointer to the entry's destination, and
//!   repeat until the stack is empty. The root's destination is `0`, which stands for the
//!   walker's own result slot.
//!
//! **Where the edges come from.** While the walker body is emitted, the builder carries a
//! [`GraphCopyWalker`]. The three places the per-shape copy deep-copies a pointer edge — a
//! record's pointer field (`copy_record_fields_into_existing`), a union variant field
//! (`copy_union_fields_into_existing`) and a collection's pointer payload
//! (`fix_collection_transfer_payload`) — ask [`CodeBuilder::graph_copy_edge_kind`] first,
//! and for an edge whose type takes part in a cycle they push `{kind, child, address of the
//! word in the new block}` where they used to call. The walker's edge list is therefore
//! not a second enumeration that can drift from the copy's: it is the copy's own, with the
//! call replaced by a push at the same site. `graph_copy_edges_match_the_copy_calls` pins
//! that per kind.
//!
//! Every entry writes only its own new block and its own destination word, and no block
//! moves once allocated, so the order entries are taken in cannot change the graph
//! produced.
//!
//! The per-type symbols (`thread_copy_symbol`) stay as the entry point every caller
//! already uses; each is now a shim that passes its kind to the walker. The one copy in
//! the per-shape code that is not an edge site — a resource's `STATE` record
//! (`copy_resource_to_current_arena`) — still calls that shim, which runs a walker of its
//! own.

use crate::codegen::collection::layout::type_participates_in_cycle;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::target::shared::abi;
use crate::types::ParameterType;
use std::collections::HashMap;

/// The walker: kind index in argument 0, source pointer in argument 1; returns the copy
/// in the return register.
pub(crate) const GRAPH_COPY_SYMBOL: &str = "_mfb_rt_graph_copy";
/// Grows a full work stack: the stack in argument 0; returns a block of twice the
/// capacity holding the same entries (the old block freed), or `0` when the arena is
/// exhausted (the old block kept, so the caller's raise leaves nothing dangling).
pub(crate) const GRAPH_STACK_GROW_SYMBOL: &str = "_mfb_rt_graph_stack_grow";

// The work stack's layout, shared with the drop walker (`graph_drop.rs`).
pub(super) const STACK_OFFSET_COUNT: usize = 0;
pub(super) const STACK_OFFSET_CAPACITY: usize = 8;
pub(super) const STACK_HEADER_SIZE: usize = 16;
pub(super) const STACK_ENTRY_SIZE: usize = 24;
pub(super) const STACK_ENTRY_OFFSET_KIND: usize = 0;
pub(super) const STACK_ENTRY_OFFSET_SOURCE: usize = 8;
const STACK_ENTRY_OFFSET_DESTINATION: usize = 16;
pub(super) const STACK_INITIAL_CAPACITY: usize = 64;

/// The label a push site defines, followed by the pushed kind's index. A label emits no
/// bytes, so the edge-table tests can read a walker's edges out of its instructions.
pub(super) const PUSH_LABEL_PREFIX: &str = "graph_copy_push_k";

/// The walker's state while its body is being emitted (`CodeBuilder::graph_copy_walker`).
#[derive(Clone, Debug)]
pub(crate) struct GraphCopyWalker {
    /// Rendered type name -> kind index. Keyed by the rendered name because that is what
    /// `thread_copy_symbol` keys the per-type functions by: an edge that has a kind here
    /// is exactly an edge that had a per-type function to call.
    kinds: HashMap<String, usize>,
    /// The frame slot holding the work stack's block pointer (it changes when it grows).
    stack_slot: usize,
}

impl GraphCopyWalker {
    /// `kinds` in `recursive_transfer_types` order; `stack_slot` holds the work stack.
    pub(crate) fn new(kinds: &[String], stack_slot: usize) -> Self {
        GraphCopyWalker {
            kinds: kinds
                .iter()
                .enumerate()
                .map(|(index, name)| (name.clone(), index))
                .collect(),
            stack_slot,
        }
    }
}

impl CodeBuilder<'_> {
    /// Inside the walker body, the kind of an edge of `type_` that must be pushed rather
    /// than copied in place; `None` everywhere else, and for an edge whose type takes no
    /// part in a cycle (copied inline exactly as before).
    pub(crate) fn graph_copy_edge_kind(
        &self,
        type_: &ParameterType,
    ) -> Result<Option<usize>, String> {
        let Some(walker) = self.graph_copy_walker.as_ref() else {
            return Ok(None);
        };
        // The same gate `copy_value_to_current_arena` routes to a per-type call with.
        if !type_participates_in_cycle(&self.type_model, type_) {
            return Ok(None);
        }
        walker
            .kinds
            .get(type_.name().as_ref())
            .copied()
            .map(Some)
            .ok_or_else(|| format!("graph copy walker has no kind for recursive type '{type_}'"))
    }

    /// Push `{kind, child, *destination_base_slot + offset}` onto the walker's work stack,
    /// growing it first when it is full. `child` is the source edge's pointer; the
    /// destination is the word of the already-allocated new block the copy belongs in.
    /// The drop walker (plan-134-F) pushes with no destination, recorded as `0`.
    pub(crate) fn emit_graph_copy_push(
        &mut self,
        kind: usize,
        child: impl Into<Operand>,
        destination: Option<(usize, usize)>,
    ) -> Result<(), String> {
        let stack_slot = self
            .graph_copy_walker
            .as_ref()
            .map(|walker| walker.stack_slot)
            .ok_or_else(|| "graph copy push emitted outside the walker".to_string())?;
        let child_slot = self.allocate_stack_object("graph_copy_push_child", 8);
        let have_room = self.label(&format!("{PUSH_LABEL_PREFIX}{kind}"));
        let grown = self.label("graph_copy_push_grown");
        let block = self.temporary_vreg();
        let count = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        let entry = self.temporary_vreg();
        self.emit(abi::store_u64(child, abi::stack_pointer(), child_slot));
        self.emit(abi::load_u64(&block, abi::stack_pointer(), stack_slot));
        self.emit(abi::load_u64(&count, &block, STACK_OFFSET_COUNT));
        self.emit(abi::load_u64(&scratch, &block, STACK_OFFSET_CAPACITY));
        self.emit(abi::compare_registers(&count, &scratch));
        self.emit(abi::branch_lt(&have_room));
        self.emit(abi::move_register(abi::c_arg(0), &block));
        self.emit_symbol_call(GRAPH_STACK_GROW_SYMBOL);
        self.emit(abi::compare_immediate(abi::return_register(), "0"));
        self.emit(abi::branch_ne(&grown));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&grown));
        self.emit(abi::store_u64(
            abi::return_register(),
            abi::stack_pointer(),
            stack_slot,
        ));
        self.emit(abi::label(&have_room));
        // Everything is reloaded: the grow call clobbered the registers on that path.
        self.emit(abi::load_u64(&block, abi::stack_pointer(), stack_slot));
        self.emit(abi::load_u64(&count, &block, STACK_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            &scratch,
            "Integer",
            &STACK_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&entry, &count, &scratch));
        self.emit(abi::add_immediate(&entry, &entry, STACK_HEADER_SIZE));
        self.emit(abi::add_registers(&entry, &block, &entry));
        self.emit(abi::move_immediate(&scratch, "Integer", &kind.to_string()));
        self.emit(abi::store_u64(&scratch, &entry, STACK_ENTRY_OFFSET_KIND));
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), child_slot));
        self.emit(abi::store_u64(&scratch, &entry, STACK_ENTRY_OFFSET_SOURCE));
        match destination {
            Some((destination_base_slot, offset)) => {
                self.emit(abi::load_u64(
                    &scratch,
                    abi::stack_pointer(),
                    destination_base_slot,
                ));
                self.emit(abi::add_immediate(&scratch, &scratch, offset));
            }
            None => self.emit(abi::move_immediate(&scratch, "Integer", "0")),
        }
        self.emit(abi::store_u64(
            &scratch,
            &entry,
            STACK_ENTRY_OFFSET_DESTINATION,
        ));
        self.emit(abi::add_immediate(&count, &count, 1));
        self.emit(abi::store_u64(&count, &block, STACK_OFFSET_COUNT));
        Ok(())
    }
}

/// `_mfb_rt_graph_copy(kind, source) -> copy`: the module's one deep copy for values of
/// recursive types. `kinds` is `recursive_transfer_types` in its own order.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_graph_copy_walker(
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
        GRAPH_COPY_SYMBOL,
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
    let kind_slot = builder.allocate_stack_object("graph_copy_kind", 8);
    let source_slot = builder.allocate_stack_object("graph_copy_source", 8);
    let destination_slot = builder.allocate_stack_object("graph_copy_destination", 8);
    let copied_slot = builder.allocate_stack_object("graph_copy_copied", 8);
    let result_slot = builder.allocate_stack_object("graph_copy_result", 8);
    let stack_slot = builder.allocate_stack_object("graph_copy_stack", 8);
    let alloc_ok = builder.label("graph_copy_stack_alloc_ok");
    let take = builder.label("graph_copy_take");
    let null_edge = builder.label("graph_copy_null_edge");
    let store = builder.label("graph_copy_store");
    let store_edge = builder.label("graph_copy_store_edge");
    let pop = builder.label("graph_copy_pop");
    let finish = builder.label("graph_copy_finish");
    let kind_labels: Vec<String> = kinds
        .iter()
        .map(|_| builder.label("graph_copy_kind"))
        .collect();

    // The root is the first entry to take; destination 0 stands for the result slot,
    // which lives in this frame rather than in any block.
    let kind_in = builder.allocate_register();
    let source_in = builder.allocate_register();
    builder.emit(abi::move_register(&kind_in, abi::c_arg(0)));
    builder.emit(abi::move_register(&source_in, abi::c_arg(1)));
    builder.emit(abi::store_u64(&kind_in, sp, kind_slot));
    builder.emit(abi::store_u64(&source_in, sp, source_slot));
    let zero = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&zero, "Integer", "0"));
    builder.emit(abi::store_u64(&zero, sp, destination_slot));
    builder.emit(abi::store_u64(&zero, sp, result_slot));

    // The work stack: header + 64 entries.
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

    // Take one entry: a null edge copies to null; otherwise dispatch on the kind.
    builder.emit(abi::label(&take));
    let source = builder.temporary_vreg();
    builder.emit(abi::load_u64(&source, sp, source_slot));
    builder.emit(abi::compare_immediate(&source, "0"));
    builder.emit(abi::branch_eq(&null_edge));
    let kind = builder.temporary_vreg();
    builder.emit(abi::load_u64(&kind, sp, kind_slot));
    for (index, label) in kind_labels.iter().enumerate() {
        builder.emit(abi::compare_immediate(&kind, &index.to_string()));
        builder.emit(abi::branch_eq(label));
    }
    // Every pushed kind comes from the same table the chain was built from, and a shim
    // passes only its own index, so no other value reaches here.
    builder.emit(abi::branch(&null_edge));
    for (name, label) in kinds.iter().zip(&kind_labels) {
        builder.emit(abi::label(label));
        let type_ = ParameterType::declared(name);
        let source = builder.allocate_register();
        builder.emit(abi::load_u64(&source, sp, source_slot));
        // One block, one level: its cycle-typed edges are pushed by the edge sites.
        let copied = builder.emit_thread_copy_real(&type_, &source)?;
        builder.emit(abi::store_u64(&copied, sp, copied_slot));
        builder.emit(abi::branch(&store));
    }
    builder.emit(abi::label(&null_edge));
    let zero = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&zero, "Integer", "0"));
    builder.emit(abi::store_u64(&zero, sp, copied_slot));

    // Write the new pointer where the entry said.
    builder.emit(abi::label(&store));
    let copied = builder.temporary_vreg();
    let destination = builder.temporary_vreg();
    builder.emit(abi::load_u64(&copied, sp, copied_slot));
    builder.emit(abi::load_u64(&destination, sp, destination_slot));
    builder.emit(abi::compare_immediate(&destination, "0"));
    builder.emit(abi::branch_ne(&store_edge));
    builder.emit(abi::store_u64(&copied, sp, result_slot));
    builder.emit(abi::branch(&pop));
    builder.emit(abi::label(&store_edge));
    builder.emit(abi::store_u64(&copied, &destination, 0));

    // Pop the next entry into the frame slots, or finish.
    builder.emit(abi::label(&pop));
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
    builder.emit(abi::store_u64(&scratch, sp, source_slot));
    builder.emit(abi::load_u64(
        &scratch,
        &entry,
        STACK_ENTRY_OFFSET_DESTINATION,
    ));
    builder.emit(abi::store_u64(&scratch, sp, destination_slot));
    builder.emit(abi::branch(&take));

    // Free the work stack (header + capacity * entry) and return the root's copy.
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
    let result = builder.allocate_register();
    builder.emit(abi::load_u64(&result, sp, result_slot));
    builder.emit(abi::move_register(abi::return_register(), &result));
    builder.emit(abi::return_());

    finish_helper(builder, "runtime.graphCopy", GRAPH_COPY_SYMBOL)
}

/// `_mfb_rt_graph_stack_grow(stack) -> grown stack, or 0`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_graph_stack_grow(
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
        GRAPH_STACK_GROW_SYMBOL,
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
    let old_slot = builder.allocate_stack_object("graph_stack_grow_old", 8);
    let old_size_slot = builder.allocate_stack_object("graph_stack_grow_old_size", 8);
    let new_slot = builder.allocate_stack_object("graph_stack_grow_new", 8);
    let failed = builder.label("graph_stack_grow_failed");
    let done = builder.label("graph_stack_grow_done");

    let old = builder.allocate_register();
    builder.emit(abi::move_register(&old, abi::c_arg(0)));
    builder.emit(abi::store_u64(&old, sp, old_slot));
    // old size = header + capacity * entry; new size = old size + capacity * entry.
    let capacity = builder.temporary_vreg();
    let bytes = builder.temporary_vreg();
    let scratch = builder.temporary_vreg();
    builder.emit(abi::load_u64(&capacity, &old, STACK_OFFSET_CAPACITY));
    builder.emit(abi::move_immediate(
        &scratch,
        "Integer",
        &STACK_ENTRY_SIZE.to_string(),
    ));
    builder.emit(abi::multiply_registers(&bytes, &capacity, &scratch));
    builder.emit(abi::add_immediate(&scratch, &bytes, STACK_HEADER_SIZE));
    builder.emit(abi::store_u64(&scratch, sp, old_size_slot));
    builder.emit(abi::add_registers(abi::c_arg(0), &scratch, &bytes));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_ne(&failed));
    builder.emit(abi::store_u64(abi::mfb_return(1), sp, new_slot));
    let new = builder.temporary_vreg();
    let old = builder.temporary_vreg();
    let size = builder.temporary_vreg();
    builder.emit(abi::load_u64(&new, sp, new_slot));
    builder.emit(abi::load_u64(&old, sp, old_slot));
    builder.emit(abi::load_u64(&size, sp, old_size_slot));
    builder.emit_copy_bytes(&new, &old, &size, "graph_stack_grow_entries");
    // The copied header carried the old capacity; double it.
    let new = builder.temporary_vreg();
    let capacity = builder.temporary_vreg();
    builder.emit(abi::load_u64(&new, sp, new_slot));
    builder.emit(abi::load_u64(&capacity, &new, STACK_OFFSET_CAPACITY));
    builder.emit(abi::shift_left_immediate(&capacity, &capacity, 1));
    builder.emit(abi::store_u64(&capacity, &new, STACK_OFFSET_CAPACITY));
    let old = builder.temporary_vreg();
    let size = builder.temporary_vreg();
    builder.emit(abi::load_u64(&old, sp, old_slot));
    builder.emit(abi::load_u64(&size, sp, old_size_slot));
    builder.emit(abi::move_register(abi::c_arg(0), &old));
    builder.emit(abi::move_register(abi::c_arg(1), &size));
    builder.emit_arena_free_call();
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&failed));
    let zero = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&zero, "Integer", "0"));
    builder.emit(abi::store_u64(&zero, sp, new_slot));
    builder.emit(abi::label(&done));
    let result = builder.allocate_register();
    builder.emit(abi::load_u64(&result, sp, new_slot));
    builder.emit(abi::move_register(abi::return_register(), &result));
    builder.emit(abi::return_());

    finish_helper(builder, "runtime.graphStackGrow", GRAPH_STACK_GROW_SYMBOL)
}

/// Register allocation, the two peephole passes and the frame — the tail every
/// synthesized runtime helper runs (`lower_drop_owned_collection_helper`).
pub(super) fn finish_helper(
    mut builder: CodeBuilder<'_>,
    name: &str,
    symbol: &str,
) -> Result<CodeFunction, String> {
    builder.run_register_allocation()?;
    let mut instructions = builder.instructions;
    let is_x86 = crate::codegen::engine::mir::active_backend()
        .register_model()
        .arena_base()
        == crate::arch::x86_64::regmodel::ARENA_BASE_REGISTER;
    crate::optimizer::opt2::peephole::forward_stores_to_loads(&mut instructions, is_x86);
    crate::optimizer::opt2::peephole::remove_fp_shuttles(
        &mut instructions,
        crate::codegen::engine::mir::active_backend().register_model(),
    );
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );
    Ok(CodeFunction {
        name: name.to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: String::new(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::codegen::collection::layout::{recursive_transfer_types, thread_copy_symbol};
    use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
    use crate::target::shared::nir::{NirField, NirModule, NirType, NirVariant};

    pub(crate) fn field(name: &str, type_: &str) -> NirField {
        NirField {
            visibility: None,
            name: name.to_string(),
            type_: ParameterType::parse(type_),
        }
    }

    pub(crate) fn record(name: &str, fields: Vec<NirField>) -> NirType {
        NirType {
            kind: "record".to_string(),
            visibility: "private".to_string(),
            name: name.to_string(),
            fields,
            includes: Vec::new(),
            variants: Vec::new(),
            members: Vec::new(),
        }
    }

    pub(crate) fn union(name: &str, variants: Vec<(&str, Vec<NirField>)>) -> NirType {
        NirType {
            kind: "union".to_string(),
            visibility: "private".to_string(),
            name: name.to_string(),
            fields: Vec::new(),
            includes: Vec::new(),
            variants: variants
                .into_iter()
                .map(|(variant, fields)| NirVariant {
                    name: variant.to_string(),
                    fields,
                })
                .collect(),
            members: Vec::new(),
        }
    }

    pub(crate) fn model(types: Vec<NirType>) -> TypeModel {
        let module = NirModule {
            target: "test".to_string(),
            build_mode: crate::target::NativeBuildMode::Console,
            stdin_log_cap: crate::codegen::error::constants::STDIN_LOG_CAP_DEFAULT,
            debug: crate::codegen::debug::DebugOptions::OFF,
            project: "test".to_string(),
            entry: None,
            globals: Vec::new(),
            types,
            imports: Vec::new(),
            runtime_helpers: Vec::new(),
            functions: Vec::new(),
            link_functions: Vec::new(),
            link_cstructs: Vec::new(),
            native_resources: Vec::new(),
            native_libraries: Default::default(),
            max_buffer_bytes: crate::manifest::DEFAULT_MAX_BUFFER_MIB * 1024 * 1024,
        };
        TypeModel::from_module(&module).expect("the test model builds")
    }

    /// `json::Json`'s shape: a data union whose array and object variants hold
    /// collections of the union itself.
    pub(crate) fn json_like_types() -> Vec<NirType> {
        let arr = || vec![field("items", "List OF J")];
        let obj = || vec![field("fields", "Map OF String TO J")];
        let text = || vec![field("s", "String")];
        vec![
            record("JArr", arr()),
            record("JObj", obj()),
            record("JStr", text()),
            union(
                "J",
                vec![("JArr", arr()), ("JObj", obj()), ("JStr", text())],
            ),
        ]
    }

    /// For kind `name`, as rendered type names: the per-type copy functions the ordinary
    /// copy CALLS, and the kinds the walker body PUSHES. Both sorted.
    /// A harness over `model` whose builders can emit a shape copy or drop. Every shape
    /// raises `ErrOutOfMemory` when an `arena_alloc` fails, and the raise loads its message
    /// as a string literal.
    pub(crate) fn harness(model: &TypeModel) -> BuilderHarness<'_> {
        let (_, out_of_memory) = crate::codegen::registry::runtime_error("ErrOutOfMemory")
            .expect("ErrOutOfMemory is a runtime error");
        BuilderHarness {
            type_model: model.clone(),
            string_symbols: HashMap::from([(
                out_of_memory.to_string(),
                "_mfb_str_out_of_memory".to_string(),
            )]),
            ..BuilderHarness::default()
        }
    }

    /// The kinds a walker body pushed, as rendered type names, sorted — read from the
    /// push sites' labels.
    pub(crate) fn pushed_kinds(builder: &CodeBuilder<'_>, kinds: &[String]) -> Vec<String> {
        let mut pushes: Vec<String> = builder
            .instructions
            .iter()
            .filter(|instruction| instruction.op == CodeOp::Label)
            .filter_map(|instruction| instruction.get("name"))
            .filter_map(|label| {
                let rest = label.strip_prefix(PUSH_LABEL_PREFIX)?;
                rest.split('_').next()?.parse::<usize>().ok()
            })
            .map(|index| kinds[index].clone())
            .collect();
        pushes.sort();
        pushes
    }

    /// For kind `name`, as rendered type names: the per-type copy functions the ordinary
    /// copy CALLS, and the kinds the walker body PUSHES. Both sorted.
    pub(crate) fn calls_and_pushes(model: &TypeModel, name: &str) -> (Vec<String>, Vec<String>) {
        let kinds: Vec<String> = recursive_transfer_types(model).into_iter().collect();
        let by_symbol: HashMap<String, String> = kinds
            .iter()
            .map(|kind| {
                (
                    thread_copy_symbol(&ParameterType::declared(kind)),
                    kind.clone(),
                )
            })
            .collect();
        let type_ = ParameterType::declared(name);
        let platform = TestPlatform;
        let harness = harness(model);

        let mut plain = harness.builder("plain", &platform);
        let source = plain.allocate_register();
        plain
            .emit_thread_copy_real(&type_, &source)
            .unwrap_or_else(|error| panic!("{name}: the ordinary copy emits: {error}"));
        let mut calls: Vec<String> = plain
            .relocations
            .iter()
            .filter_map(|relocation| by_symbol.get(&relocation.to).cloned())
            .collect();

        let mut walker = harness.builder("walker", &platform);
        let stack_slot = walker.allocate_stack_object("stack", 8);
        walker.graph_copy_walker = Some(GraphCopyWalker::new(&kinds, stack_slot));
        let source = walker.allocate_register();
        walker
            .emit_thread_copy_real(&type_, &source)
            .unwrap_or_else(|error| panic!("{name}: the walker body emits: {error}"));
        let still_calling: Vec<&String> = walker
            .relocations
            .iter()
            .filter(|relocation| by_symbol.contains_key(&relocation.to))
            .map(|relocation| &relocation.to)
            .collect();
        assert!(
            still_calling.is_empty(),
            "{name}: the walker body still calls per-type copies (native recursion): \
             {still_calling:?}"
        );
        let pushes = pushed_kinds(&walker, &kinds);
        calls.sort();
        (calls, pushes)
    }

    /// The walker pushes exactly the edges the ordinary copy calls a per-type function
    /// for, kind by kind — so a missed edge (left shared) or an extra one (a non-pointer
    /// word copied as a pointer) cannot hide. Three models: a self-recursive record, a
    /// Json-shaped recursive union, and a cycle member whose non-cycle field
    /// (`Map OF Integer TO J`) reaches a SECOND cycle, whose edges the walker must push
    /// from inside the inline copy of that field.
    #[test]
    fn graph_copy_edges_match_the_copy_calls() {
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

        for (label, model) in [
            ("TYPE Node (kids AS List OF Node)", &node),
            ("a Json-shaped recursive union", &json),
            ("a cycle member reaching a second cycle", &bridged),
        ] {
            let kinds = recursive_transfer_types(model);
            assert!(
                !kinds.is_empty(),
                "{label}: the model has no recursive type"
            );
            let mut edges = 0;
            for kind in &kinds {
                let (calls, pushes) = calls_and_pushes(model, kind);
                assert_eq!(
                    pushes, calls,
                    "{label}: kind {kind}: the walker must push exactly the edges the copy \
                     calls through"
                );
                edges += pushes.len();
            }
            assert!(
                edges > 0,
                "{label}: no edges at all — not looking at a cycle"
            );
        }

        // A `J` payload of `Map OF Integer TO J` is an inline data union, so the map's
        // inline copy walks into it and pushes ITS cycle-typed edges — the second
        // cycle's collections — from inside Node2's body.
        let (_, node2_pushes) = calls_and_pushes(&bridged, "Node2");
        for expected in ["List OF J", "Map OF String TO J", "List OF Node2"] {
            assert!(
                node2_pushes.iter().any(|kind| kind == expected),
                "Node2's body must push `{expected}`: {node2_pushes:?}"
            );
        }
    }
}
