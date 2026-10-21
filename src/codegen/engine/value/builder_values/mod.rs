// --- codegen tier imports (migration) ---
use crate::codegen::builtins;
use crate::codegen::builtins::vector::vector_field_count;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::function::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::result::UnboxedPayloadSource;
use crate::codegen::memory::arena::builder_arena_transfer::{PayloadEdgeShape, RawSuccessBlock};
use crate::codegen::memory::arena::graph_drop::StoreShape;
use crate::codegen::memory::arena::TrappedErrorSource;
use crate::codegen::memory::data::*;
use crate::operators::{BinaryOp, UnaryOp};
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::target::shared::runtime;
use crate::types::ParameterType;

mod inline_calls;
mod lower_call;
mod lower_construct;
mod lower_inner;
mod ownership;
mod pending_temps;

impl<'a> CodeBuilder<'a> {
    /// Build an [`AbiCtx`] from the builder's own platform/imports/build-mode for an
    /// inline (`abi_inline`) lowering. The fields are `&'a`/`Copy`,
    /// so the returned `AbiCtx` borrows the underlying build data, not `self` — leaving
    /// `self` free to pass mutably to the lowering. The inline path has no OS-seam arena
    /// offsets (that is an `abi_function` concern), so both are `None`.
    pub(crate) fn inline_abi_ctx(&self) -> crate::codegen::registry::AbiCtx<'a> {
        crate::codegen::registry::AbiCtx {
            platform_imports: self.platform_imports,
            platform: self.platform,
            build_mode: self.build_mode,
            // The inline path takes no resource-path member, so no module identity
            // is needed (that is an `abi_function` concern).
            module_name: "",
            term_state_offset: None,
            presentation_mode_offset: None,
            // The mouse-state region is likewise an `abi_function` concern.
            mouse_state_offset: None,
            input_state_offset: None,
            // The inline path lowers per call site and hands the body its raw args +
            // target directly; no `abi_function` runtime-call name applies.
            call: "",
            // Worker-spawn build state (`thread.start`) is an `abi_function` concern;
            // the inline path never lowers `thread.start`.
            arena_global_slots: 0,
            uses_rng: false,
            debug_arena_registry: false,
        }
    }
}

/// Flatten the left-leaning spine of a `&` chain into its operands, in source
/// order. `a & b & c` parses as `(a & b) & c`, so the chain is the left
/// descendants; the caller appends the right operand itself.
///
/// The spine stops at the first non-`&` value, so a parenthesized subchain on
/// the right (`a & (b & c)`) keeps its own grouping and is lowered as its own
/// fused chain — the operand order the source wrote is preserved either way.
pub(crate) fn flatten_concat_spine<'a>(value: &'a NirValue, out: &mut Vec<&'a NirValue>) {
    if let NirValue::Binary {
        op, left, right, ..
    } = value
    {
        if *op == BinaryOp::Concat {
            flatten_concat_spine(left, out);
            out.push(right);
            return;
        }
    }
    out.push(value);
}

/// bug-567: where the value a control transfer carries out lives, while that
/// statement's INTERIOR temporaries are freed around it.
///
/// The two `RETURN` lowerings differ only here: the register fast path (no live
/// cleanups) still holds the block in a register, and the cleanup-bearing path has
/// already parked it in `pending_result_slots.value`. Both hand
/// [`CodeBuilder::drop_interior_temps_before_branch`] a slot to compare against,
/// so the runtime identity guard is emitted on both and neither can free the block
/// the caller is about to own.
#[derive(Clone)]
pub(crate) enum EscapingValue {
    /// Nothing leaves (a bare `RETURN`, or a `Nothing` result).
    None,
    /// Live in this operand; park it across the frees and reload it afterwards.
    InRegister(Operand),
    /// Already parked in this stack slot by the caller; guard against it, and do
    /// not park or reload.
    InSlot(usize),
}

/// Extract the source location carried by an error-originating NIR value, if any.
pub(crate) fn value_loc(value: &NirValue) -> Option<NirSourceLoc> {
    match value {
        NirValue::Call { loc, .. }
        | NirValue::CallResult { loc, .. }
        | NirValue::RuntimeCall { loc, .. }
        | NirValue::Binary { loc, .. }
        | NirValue::Unary { loc, .. } => Some(*loc),
        _ => None,
    }
}
