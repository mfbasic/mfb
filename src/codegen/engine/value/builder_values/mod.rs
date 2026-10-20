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
pub(super) fn flatten_concat_spine<'a>(value: &'a NirValue, out: &mut Vec<&'a NirValue>) {
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

#[cfg(test)]
mod concat_spine_tests {
    use super::*;
    use crate::target::shared::nir::NirSourceLoc;
    use crate::types::ParameterType;

    fn text(value: &str) -> NirValue {
        NirValue::Const {
            type_: ParameterType::String,
            value: value.to_string(),
        }
    }

    fn concat(left: NirValue, right: NirValue) -> NirValue {
        NirValue::Binary {
            op: BinaryOp::Concat,
            left: Box::new(left),
            right: Box::new(right),
            loc: NirSourceLoc::default(),
        }
    }

    fn spine(value: &NirValue) -> Vec<String> {
        let mut parts = Vec::new();
        flatten_concat_spine(value, &mut parts);
        parts
            .into_iter()
            .map(|part| match part {
                NirValue::Const { value, .. } => value.clone(),
                _ => "?".to_string(),
            })
            .collect()
    }

    /// `a & b & c` parses left-leaning, so the spine is `a`, `b` and the
    /// caller appends `c`.
    #[test]
    fn a_left_leaning_chain_flattens_in_source_order() {
        let chain = concat(concat(text("a"), text("b")), text("c"));
        let NirValue::Binary { left, .. } = &chain else {
            panic!("expected a concat")
        };
        assert_eq!(spine(left), vec!["a", "b"]);
    }

    /// A four-operand chain keeps going.
    #[test]
    fn a_longer_chain_keeps_its_order() {
        let chain = concat(concat(concat(text("a"), text("b")), text("c")), text("d"));
        let NirValue::Binary { left, .. } = &chain else {
            panic!("expected a concat")
        };
        assert_eq!(spine(left), vec!["a", "b", "c"]);
    }

    /// The spine stops at a non-`&` value: a parenthesized right subchain
    /// keeps its own grouping and is fused as its own chain.
    #[test]
    fn the_spine_stops_at_a_non_concat() {
        let inner = concat(text("b"), text("c"));
        assert_eq!(spine(&inner), vec!["b", "c"]);
        let other = NirValue::Binary {
            op: BinaryOp::Add,
            left: Box::new(text("a")),
            right: Box::new(text("b")),
            loc: NirSourceLoc::default(),
        };
        assert_eq!(spine(&other), vec!["?"]);
    }

    /// A lone operand is a one-element spine, which the caller's arity check
    /// then sends down the pairwise path.
    #[test]
    fn a_lone_operand_is_its_own_spine() {
        assert_eq!(spine(&text("a")), vec!["a"]);
    }
}
