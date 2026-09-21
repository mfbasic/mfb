//! plan-142-C: in-place arms for the self-updates that keep a list's length and
//! rewrite or reorder its elements — the `math` element-wise functions,
//! `collections::replace`, `transform`, `sort` and `sortBy`.
//!
//! Each obeys plan-142-A's failure-atomicity rule: every error the operation can
//! raise is raised before `x`'s block is written, so a failing statement leaves
//! `x` as it was (the assignment never happened).

use crate::codegen::collection::assign::self_update::SelfUpdateSite;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::typed_list_element_type;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

/// The `math` functions with a `List` self-update overload
/// (`registry::self_update_shaped`; plan-142-A Phase 1).
pub(crate) const MATH_SELF_UPDATE: &[&str] = &[
    "abs", "acos", "asin", "atan", "atan2", "clamp", "cos", "exp", "log", "log10", "max", "min",
    "pow", "sin", "sqrt", "tan",
];

/// The `math` function a call target names, when it is one of
/// [`MATH_SELF_UPDATE`].
pub(crate) fn math_self_update_function(target: &str) -> Option<&str> {
    target
        .strip_prefix("math.")
        .filter(|function| MATH_SELF_UPDATE.contains(function))
}

impl CodeBuilder<'_> {
    /// `x = math::f(x, …)` on a `List OF Integer`/`Float`/`Fixed`.
    ///
    /// The member's own array lowering runs unchanged, with its result list
    /// redirected into the function's self-update scratch (`simd_result_into`,
    /// honoured by `emit_alloc_result_list`, which every `math` array driver
    /// allocates through). Every kernel reduces its per-lane error mask and raises
    /// **after** its loop, so the lanes cannot be written straight into `x`: they
    /// land in scratch, and only once the kernel has returned without raising are
    /// they copied over `x`'s data. A domain error therefore leaves `x` untouched,
    /// and nothing is allocated per statement.
    pub(crate) fn try_inplace_math_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        if self.self_update_scratch.is_none() {
            return Ok(false);
        }
        let NirValue::Call { target, args, .. } = value else {
            return Ok(false);
        };
        let Some(function) = math_self_update_function(target) else {
            return Ok(false);
        };
        let Some(resolved) = self.resolve_self_update(site, value, function, args.len()) else {
            return Ok(false);
        };
        // G9 — the list overloads are over 8-byte lanes only.
        let Some(element_type) = typed_list_element_type(&resolved.collection_type).cloned() else {
            return Ok(false);
        };
        if !matches!(
            element_type,
            ParameterType::Integer | ParameterType::Float | ParameterType::Fixed
        ) {
            return Ok(false);
        }
        let lower = crate::codegen::registry::abi_inline_lower(target)
            .ok_or_else(|| format!("native in-place math: `{target}` has no inline lowering"))?;
        let buffer_slot = resolved.dest.block_slot();
        let marker = self.allocate_stack_object("inplace_math_result", 8);
        // The arguments are lowered before the redirect is armed, so a nested
        // array call in an operand (`min(x, abs(ys))`) allocates its own result.
        let arg_values = self.lower_abi_inline_args(args)?;
        let ctx = self.inline_abi_ctx();
        self.simd_result_into = Some(());
        let lowered = lower(self, &arg_values, &ctx);
        let unconsumed = self.simd_result_into.take().is_some();
        let result = lowered?;
        if unconsumed {
            return Err(format!(
                "native in-place math: `{target}` over {} did not allocate its result \
                 through emit_alloc_result_list",
                resolved.collection_type
            ));
        }
        // Copy the scratch lanes over `x`'s data: count * 8 bytes.
        self.emit(abi::store_u64(
            &result.location,
            abi::stack_pointer(),
            marker,
        ));
        let src = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let base = self.temporary_vreg();
        let len = self.temporary_vreg();
        let copy = self.temporary_vreg();
        self.emit(abi::load_u64(&src, abi::stack_pointer(), marker));
        self.emit_collection_data_pointer_for(&src, &src, &ParameterType::Integer);
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit_collection_data_pointer_for(&dst, &base, &element_type);
        self.emit(abi::load_u64(&len, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::shift_left_immediate(&len, &len, 3));
        self.emit_block_copy_advance(&dst, &src, &len, &copy, "inplace_math_copy");
        if let Some(local) = self.locals.get_mut(site.name) {
            local.constant = None;
        }
        Ok(true)
    }
}
