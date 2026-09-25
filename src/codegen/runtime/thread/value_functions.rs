//! plan-156-A: the per-type functions the thread runtime calls to move a value into
//! and out of a control block's block arena.
//!
//! The thread helpers are type-agnostic — one `_mfb_rt_thread_thread_send` serves
//! every message type, and the trampoline serves every worker — but a deep copy is
//! not. So every call site that hands a value across (a `thread::start` seed and the
//! entry's result type, a `send`/`transfer` message, a `receive`/`accept` read) passes
//! the ADDRESS of a per-type function, and the helper calls it at the right moment
//! under the block lock:
//!
//! - **copy** `(source) -> copy` deep-copies a value into the CURRENT arena (whatever
//!   the pinned arena register names when it runs) and returns the tagged result. A
//!   helper points the register at the block arena to copy a value in, and leaves it
//!   on the reader's own arena to copy one out.
//! - **drop** `(value)` frees a value that lives in the current arena — the helper
//!   runs it on the block arena once a reader has its own copy of a message.
//!
//! A scalar needs neither: its word is the value, so the call site passes 0 and the
//! helper moves the word as is.
//!
//! The functions are requested while functions are lowered and emitted once per
//! module afterwards ([`take_requested`]). Lowering is sequential per module (the
//! builder is per-function and the module's functions are lowered in one loop on
//! one thread), so a thread-local set is the module's set; [`reset`] clears it at the
//! start of each module.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::push_symbol_address;
use crate::target::shared::abi;
use crate::target::shared::nir::NirFunction;
use crate::types::ParameterType;

/// Which per-type function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThreadValueFn {
    Copy,
    Drop,
}

thread_local! {
    static REQUESTED: RefCell<BTreeMap<String, (ThreadValueFn, ParameterType)>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// The symbol of `kind`'s function for `type_`. The type renders here — the type ->
/// symbol boundary — sanitized, with an FNV-1a hash of the full spelling so two types
/// that sanitize alike never share a symbol.
pub(crate) fn thread_value_fn_symbol(kind: ThreadValueFn, type_: &ParameterType) -> String {
    let rendered = type_.name();
    let sanitized: String = rendered
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    let mut hash: u64 = 1469598103934665603;
    for byte in rendered.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1099511628211);
    }
    let kind = match kind {
        ThreadValueFn::Copy => "copy",
        ThreadValueFn::Drop => "drop",
    };
    format!("_mfb_rt_thread_{kind}_value_{sanitized}_{hash:016x}")
}

/// Request `kind`'s function for `type_` in the module being lowered, returning its
/// symbol.
pub(crate) fn request(kind: ThreadValueFn, type_: &ParameterType) -> String {
    let symbol = thread_value_fn_symbol(kind, type_);
    REQUESTED.with(|requested| {
        requested
            .borrow_mut()
            .entry(symbol.clone())
            .or_insert_with(|| (kind, type_.clone()));
    });
    symbol
}

/// Every function requested since the last [`reset`], in symbol order (so the emitted
/// order is deterministic), leaving the set empty.
pub(crate) fn take_requested() -> Vec<(String, ThreadValueFn, ParameterType)> {
    REQUESTED.with(|requested| {
        std::mem::take(&mut *requested.borrow_mut())
            .into_iter()
            .map(|(symbol, (kind, type_))| (symbol, kind, type_))
            .collect()
    })
}

/// Forget every request — called when a module's lowering starts.
pub(crate) fn reset() {
    REQUESTED.with(|requested| requested.borrow_mut().clear());
}

/// Whether a value of `type_` crosses the thread boundary as its bare word: a scalar
/// or an enum member (its ordinal) has no block to copy, so its per-type function
/// address is 0. The same set `copy_value_to_current_arena`'s first arm moves as a
/// register (bug-656 put enums there).
impl CodeBuilder<'_> {
    pub(crate) fn thread_value_is_word(&self, type_: &ParameterType) -> bool {
        matches!(
            type_,
            ParameterType::Nothing
                | ParameterType::Boolean
                | ParameterType::Byte
                | ParameterType::Integer
                | ParameterType::Float
                | ParameterType::Fixed
                | ParameterType::Money
        ) || type_.is_named("Scalar")
            || self.is_enum_type(type_)
    }

    /// Load the address of `kind`'s per-type function for `type_` into `dst`, or 0
    /// when the value crosses as its word ([`Self::thread_value_is_word`]). Requests
    /// the function for this module.
    pub(crate) fn emit_thread_value_fn_address(
        &mut self,
        kind: ThreadValueFn,
        type_: &ParameterType,
        dst: impl Into<Operand>,
    ) {
        let dst = dst.into();
        if self.thread_value_is_word(type_) {
            self.emit(abi::move_immediate(dst, "Integer", "0"));
            return;
        }
        let symbol = request(kind, type_);
        let from = self.current_symbol.clone();
        push_symbol_address(&from, &symbol, dst, &mut self.instructions, &mut self.relocations);
    }
}

/// Lower `kind`'s per-type function for `type_` as a standalone function named
/// `symbol` (see the module doc). Both return a tagged result: a copy returns
/// `ErrOutOfMemory` when an allocation fails (the caller discards it and raises its
/// own), a drop always succeeds.
///
/// The drop frees what the value's owner would: a flat block or a graph through the
/// ordinary owned-value drop, a resource record (and its `STATE` block) by size after
/// closing its handle if it is still open. A
/// value of any other shape — a resource-carrying union or collection — is left to
/// the block arena's destruction, which returns every chunk to the OS when the
/// control block is freed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_thread_value_function<'a>(
    kind: ThreadValueFn,
    type_: &ParameterType,
    symbol: &str,
    function_symbols: &'a HashMap<String, String>,
    functions: &'a HashMap<String, &'a NirFunction>,
    package_return_types: &'a HashMap<String, ParameterType>,
    platform_imports: &'a HashMap<String, String>,
    platform: &'a dyn CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &'a HashMap<String, GlobalValue>,
    string_symbols: &'a HashMap<String, String>,
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
    match kind {
        ThreadValueFn::Copy => {
            let source = builder.allocate_register();
            builder.emit(abi::move_register(&source, abi::c_arg(0)));
            let copied = builder.copy_value_to_current_arena(type_, &source)?;
            builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &copied));
        }
        ThreadValueFn::Drop => {
            let slot = builder.allocate_stack_object("thread_drop_value", 8);
            builder.emit(abi::store_u64(abi::c_arg(0), abi::stack_pointer(), slot));
            if builder.is_resource_nominal(type_) {
                // A record still open is a transfer nobody accepted: close its handle
                // first (a read's record was flagged `moved|closed` by the reader's
                // copy, so its close is skipped). A close error has no one to report
                // to at a thread's teardown and is dropped.
                if let Some(close) = builder.resource_cleanup_symbol(type_) {
                    let closed = builder.label("thread_drop_value_closed");
                    let record = builder.allocate_register();
                    let flags = builder.allocate_register();
                    builder.emit(abi::load_u64(&record, abi::stack_pointer(), slot));
                    builder.emit(abi::load_u64(&flags, &record, RESOURCE_OFFSET_CLOSED));
                    builder.emit(abi::compare_immediate(&flags, "0"));
                    builder.emit(abi::branch_ne(&closed));
                    builder.emit(abi::move_register(abi::c_arg(0), &record));
                    builder.emit_symbol_call(&close);
                    builder.emit(abi::label(&closed));
                }
                if let Some(state_type) = type_.state() {
                    builder.emit_free_resource_state_block(slot, &state_type)?;
                }
                builder.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), slot));
                builder.emit(abi::move_immediate(
                    abi::c_arg(1),
                    "Integer",
                    RESOURCE_RECORD_SIZE,
                ));
                builder.emit_arena_free_call();
            } else if builder.is_freeable_flat_value(type_) || builder.owns_graph(type_) {
                builder.emit_owned_value_drop(&OwnedValueCleanup {
                    type_: type_.clone(),
                    stack_offset: slot,
                    closure_captures: None,
                    capacity_slot: None,
                    loop_alias_slot: None,
                    result_wrapper: None,
                })?;
            }
        }
    }
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    // Same tail as the other synthesized functions (`lower_to_string_helper`):
    // allocate, run the two peepholes, then build the frame.
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
    let verb = match kind {
        ThreadValueFn::Copy => "copy",
        ThreadValueFn::Drop => "drop",
    };
    Ok(CodeFunction {
        name: format!("thread_{verb}_value.{type_}"),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_distinguish_kind_and_type() {
        let a = thread_value_fn_symbol(ThreadValueFn::Copy, &ParameterType::String);
        let b = thread_value_fn_symbol(ThreadValueFn::Drop, &ParameterType::String);
        let c = thread_value_fn_symbol(
            ThreadValueFn::Copy,
            &ParameterType::ListOf(Box::new(ParameterType::String)),
        );
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("_mfb_rt_thread_copy_value_String_"));
    }

    #[test]
    fn requests_are_deduplicated_and_drained() {
        reset();
        let first = request(ThreadValueFn::Copy, &ParameterType::String);
        let again = request(ThreadValueFn::Copy, &ParameterType::String);
        assert_eq!(first, again);
        request(ThreadValueFn::Drop, &ParameterType::String);
        let taken = take_requested();
        assert_eq!(taken.len(), 2);
        assert!(take_requested().is_empty());
    }
}
