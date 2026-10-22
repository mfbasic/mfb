// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

mod arrays;
mod rng;
mod scalar;
pub(crate) enum FloatInfinityError {
    Infinity,
    Overflow,
}

impl CodeBuilder<'_> {
    pub(crate) fn lower_math_call(
        &mut self,
        function: &str,
        args: &[ValueResult],
    ) -> Result<ValueResult, String> {
        match function {
            "abs" if args.len() == 1 && self.is_list_argument(&args[0]) => {
                self.lower_math_abs_array(&args[0])
            }
            "abs" if args.len() == 1 => self.lower_math_abs(&args[0]),
            "sqrt" if args.len() == 1 && self.is_list_argument(&args[0]) => {
                self.lower_math_sqrt_array(&args[0])
            }
            "floor" | "ceil" | "round" if args.len() == 1 && self.is_list_argument(&args[0]) => {
                self.lower_math_rounding_array(function, &args[0])
            }
            "min" | "max" if args.len() == 2 && self.is_list_argument(&args[0]) => {
                self.lower_math_min_max_array(function, args)
            }
            "clamp" if args.len() == 3 && self.is_list_argument(&args[0]) => {
                self.lower_math_clamp_array(args)
            }
            "log" | "log10" if args.len() == 1 && self.is_list_argument(&args[0]) => {
                self.lower_math_log_array(function, &args[0])
            }
            "exp" if args.len() == 1 && self.is_list_argument(&args[0]) => {
                self.lower_math_exp_array(&args[0])
            }
            "atan2" | "pow" if args.len() == 2 && self.is_list_argument(&args[0]) => {
                self.lower_math_atan2_pow_array(function, args)
            }
            "sin" | "cos" | "tan" | "atan" | "asin" | "acos"
                if args.len() == 1 && self.is_list_argument(&args[0]) =>
            {
                self.lower_math_trig_array(function, &args[0])
            }
            "min" | "max" if args.len() == 2 => self.lower_math_min_max(function, args),
            "clamp" if args.len() == 3 => self.lower_math_clamp(args),
            "floor" | "ceil" | "round" if args.len() == 1 => {
                self.lower_math_rounding(function, &args[0])
            }
            "rand" if args.len() == 2 => self.lower_math_rand(args),
            "seed" if args.len() == 1 => self.lower_math_seed(&args[0]),
            "sqrt" if args.len() == 1 => self.lower_math_sqrt(&args[0]),
            // Scalar Float pow shares the strict array Pow kernel (negative-base
            // integer exponents included); Fixed keeps the Q32.32 path.
            "pow" if args.len() == 2 => self.lower_math_scalar_binary(function, args),
            // Scalar Float atan2 shares the strict array Atan2 kernel (so
            // `math::atan2(y, x)` and `math::atan2([y], [x])[0]` are bit-identical
            // and no libm `atan2` is imported); Fixed keeps the Q32.32 path.
            "atan2" if args.len() == 2 => self.lower_math_scalar_binary(function, args),
            // Scalar Float exp/log/log10/sin/cos share the array NEON kernels
            // (plan-01-simd §4.7: one deterministic surface, <=1 ULP of libm, and
            // the kernels' float error codes — ErrFloatDomain/ErrFloatInf/
            // ErrFloatNan — already match the scalar man pages). Fixed stays on
            // the deterministic Q32.32 path; tan/asin/acos/atan stay on libm until
            // their kernels reach strict <=1 ULP.
            "exp" | "log" | "log10" | "sin" | "cos" | "tan" | "atan" | "asin" | "acos"
                if args.len() == 1 =>
            {
                self.lower_math_scalar_transcendental(function, &args[0])
            }
            other => Err(format!(
                "native math lowering does not support math.{other}"
            )),
        }
    }

    /// Whether `arg`'s static type is a `List OF …` (selects the SIMD array
    /// overloads over the scalar `math::` lowerings).
    pub(crate) fn is_list_argument(&self, arg: &ValueResult) -> bool {
        matches!(arg.type_, ParameterType::ListOf(_))
    }

    pub(crate) fn emit_float_result_check(
        &mut self,
        bits: impl Into<Reg>,
        infinity_error: FloatInfinityError,
    ) -> Result<(), String> {
        let bits: Reg = bits.into();
        // A finite f64 has a biased exponent below 0x7FF. Drop the sign bit by
        // shifting left one — the remaining magnitude pattern orders the three
        // classes under a single unsigned compare: every finite value is
        // strictly below `+inf << 1` (0xFFE0000000000000), `+inf`/`-inf` land
        // exactly on it, and every NaN is above it. This replaces the old
        // exponent+mantissa mask pair (two 64-bit immediates, two `and`s, two
        // scratch registers) with one shift, one immediate, and one compare, and
        // it never hardcodes a result scratch register (see the negation note in
        // `lower_numeric_unary_negation`).
        let magnitude = self.allocate_register();
        let inf_bits = self.temporary_vreg();
        let ok = self.label("float_result_finite");
        let nan = self.label("float_result_nan");
        self.emit(abi::shift_left_immediate(&magnitude, bits, 1));
        // 0xFFE0000000000000 == (+inf bits) << 1.
        self.emit(abi::move_immediate(
            &inf_bits,
            "Integer",
            "18437736874454810624",
        ));
        self.emit(abi::compare_registers(&magnitude, &inf_bits));
        self.emit(abi::branch_lo(&ok)); // unsigned < +inf<<1 => finite
        self.emit(abi::branch_hi(&nan)); // unsigned > +inf<<1 => NaN
                                         // Equal => the value is exactly ±inf.
        match infinity_error {
            FloatInfinityError::Infinity => self.raise_error_bare("ErrFloatInf")?,
            FloatInfinityError::Overflow => self.raise_error_bare("ErrFloatOverflow")?,
        }
        self.emit(abi::label(&nan));
        self.raise_error_bare("ErrFloatNaN")?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    /// FP-domain twin of [`Self::emit_float_result_check`] (plan-16 Piece B). It
    /// computes the **same** three-way finite/inf/NaN predicate, but on the value
    /// while it is still resident in a `d`-register, so the result never has to be
    /// shuttled into a GPR just to be bit-tested. `value` is a `d`-register; it is
    /// read-only here (the `fabs` lands in a scratch FP register), so a chained
    /// float op keeps using it directly.
    ///
    /// `fabs` folds ±Inf onto +Inf, then a single `fcmp` against +Inf orders the
    /// three classes exactly as the integer magnitude compare did: a finite `|x|`
    /// is strictly less than +Inf, ±Inf compares equal, and a NaN compares
    /// unordered (the V flag, caught by `b.vs`). The error each class raises — and
    /// the line/char it stamps — is byte-identical to the GPR path with
    /// `FloatInfinityError::Overflow`: this twin's only callers are the observation
    /// boundaries, and an infinity escaping one is `ErrFloatOverflow`.
    pub(crate) fn emit_float_result_check_fp(
        &mut self,
        value: impl Into<Reg>,
    ) -> Result<(), String> {
        let value: Reg = value.into();
        let nan = self.label("float_result_nan");
        let ok = self.label("float_result_finite");
        let magnitude = self.allocate_fp_register();
        let positive_inf = self.allocate_fp_register();
        let inf_bits = self.temporary_vreg();
        // +inf bits == 0x7FF0000000000000. Materialized through a scratch vreg,
        // never a pooled physical GPR.
        self.emit(abi::move_immediate(
            &inf_bits,
            "Integer",
            F64_POSITIVE_INF_BITS,
        ));
        self.emit(abi::float_move_d_from_x(&positive_inf, &inf_bits));
        self.emit(abi::float_abs_d(&magnitude, value));
        self.emit(abi::float_compare_d(&magnitude, &positive_inf));
        self.emit(abi::branch_vs(&nan)); // unordered => NaN
        self.emit(abi::branch_ne(&ok)); // |x| < +inf (ordered, not equal) => finite
                                        // Fall-through: |x| == +inf, i.e. the value is exactly ±inf.
        self.raise_error_bare("ErrFloatOverflow")?;
        self.emit(abi::label(&nan));
        self.raise_error_bare("ErrFloatNaN")?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    /// Observation-boundary finiteness check (plan-17). A `Float` becomes
    /// user-accessible — bound to a named local/global, stored into a
    /// collection element or record field, returned, printed/converted, or
    /// passed as an argument — at a handful of NIR sites; this is the single
    /// choke point each calls so a non-finite traps *there*, at the boundary's
    /// `line:char`, rather than after every arithmetic op.
    ///
    /// Only a fresh float-arithmetic result can be non-finite: the `+ - * /
    /// MOD ^` operators (and unary negation, which propagates a non-finite
    /// operand) are the sole producers — a constant, a read of an
    /// already-finite binding/element/field, or a call result (the callee
    /// checked its own boundary) is finite by the boundary invariant. So the
    /// check is emitted only when `value` is such an arithmetic node, which
    /// also leaves comparisons untouched (they are IEEE, never a boundary) and
    /// lets a transient that recovers to finite — `1.0 / (1e200 * 1e200)` →
    /// `+0.0` — flow through with no trap.
    ///
    /// An escaping `±Inf` raises `ErrFloatOverflow` (the spec's "arithmetic
    /// overflow to infinity", byte-identical to the old per-op arithmetic
    /// check); a `NaN` raises `ErrFloatNaN`.
    pub(crate) fn observe_float(
        &mut self,
        value: &NirValue,
        result: &ValueResult,
    ) -> Result<(), String> {
        if result.type_ == ParameterType::Float && float_arith_node(value) {
            let saved = self.current_loc;
            // Every node `float_arith_node` admits carries a location.
            self.current_loc = crate::codegen::engine::value::value_loc(value).unwrap_or(saved);
            // A `d`-native result is checked in the FP domain (plan-01
            // float-dnative): the value is still resident in its FP register, so
            // the boundary check reads it there and the store-back stays a plain
            // `str d` with no GPR shuttle — exactly the promoted-accumulator path
            // (plan-16/17). A GP-native result is checked on its bit pattern.
            let outcome = if Self::float_is_dnative(result) {
                self.emit_float_result_check_fp(&result.location)
            } else {
                self.emit_float_result_check(&result.location, FloatInfinityError::Overflow)
            };
            self.current_loc = saved;
            outcome?;
        }
        Ok(())
    }

    /// Observation-boundary check for a loop-promoted float accumulator whose
    /// value lives in `d` after an assignment update (plan-17). Uses the
    /// FP-domain variant so the store-back stays peephole-foldable, and fires
    /// only when the assigned `value` is a fresh arithmetic node.
    pub(crate) fn observe_promoted_float(
        &mut self,
        value: &NirValue,
        d: impl Into<Reg>,
    ) -> Result<(), String> {
        let d: Reg = d.into();
        if float_arith_node(value) {
            let saved = self.current_loc;
            // Every node `float_arith_node` admits carries a location.
            self.current_loc = crate::codegen::engine::value::value_loc(value).unwrap_or(saved);
            let outcome = self.emit_float_result_check_fp(d);
            self.current_loc = saved;
            outcome?;
        }
        Ok(())
    }
}

/// Whether lowering `value` can yield a non-finite `Float`, i.e. whether an
/// observation boundary must re-check its result (`mfb spec language types`, the
/// `Float` bullet).
///
/// The float arithmetic operators are the obvious producers: a `Binary` (any
/// operator — the `observe_float` type guard skips the Boolean-result
/// comparisons) or a `Unary` (negation propagates a non-finite operand).
///
/// **A builtin `Call` is the other one (bug-590).** This predicate used to answer
/// `false` for every non-operator node, on the premise that "every other node is
/// finite by construction" — but `collections::sum`'s `Float` arm accumulates with
/// a bare `abi::float_add_d` and raises nothing, so `LET total AS Float =
/// collections::sum(xs)` bound an `+Inf` to a named local and printed `f64::MAX`'s
/// digits. Which builtin results are constrained and which are not is a TOTAL,
/// registry-checked enumeration in
/// [`float_result`](crate::codegen::builtins::float_result).
///
/// The match is deliberately **wildcard-free**: a new `NirValue` variant is a
/// build error here rather than a node that silently declines to be observed.
pub(crate) fn float_arith_node(value: &NirValue) -> bool {
    match value {
        // The float arithmetic operators — the original plan-17 producers.
        NirValue::Binary { .. } | NirValue::Unary { .. } => true,
        // A call result is finite only if the callee constrains it. `Call`,
        // `CallResult` (the `TRAP` twin) and `RuntimeCall` (a `Body::abi_function`
        // call site) all spell a builtin member as `"<package>.<member>"`.
        NirValue::Call { target, .. }
        | NirValue::CallResult { target, .. }
        | NirValue::RuntimeCall { target, .. } => {
            crate::codegen::builtins::float_result::builtin_float_result_may_be_nonfinite(target)
        }
        // `Checked` yields a `Result OF T`, never a bare `Float`, and
        // `lower_checked_value` already calls `observe_float` on the wrapped node
        // INSIDE the capture (`.ai/codegen-invariants.md`) — observing it again
        // out here would trap where the program asked to catch.
        NirValue::Checked { .. } => false,
        // Finite by construction: a literal, a read of an already-observed
        // binding / capture / element / field, or a value built out of operands
        // that were each observed as they were stored.
        NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Closure { .. }
        | NirValue::Capture { .. }
        | NirValue::Constructor { .. }
        | NirValue::UnionWrap { .. }
        | NirValue::UnionExtract { .. }
        | NirValue::ResultIsOk { .. }
        | NirValue::ResultValue { .. }
        | NirValue::ResultError { .. }
        | NirValue::WithUpdate { .. }
        | NirValue::ListLiteral { .. }
        | NirValue::SetLiteral { .. }
        | NirValue::MapLiteral { .. }
        | NirValue::MemberAccess { .. } => false,
    }
}
