//! bug-590: which builtin call results a `Float` observation boundary must re-check.
//!
//! # The contract
//!
//! `mfb spec language types` (`src/docs/spec/language/04_types.md`, the `Float`
//! bullet of "Numeric edge cases") states it:
//!
//! > MFBASIC guarantees that **no user-accessible `Float` is non-finite**,
//! > enforced at *observation boundaries* rather than after each operation: a
//! > finiteness check fires only where a `Float` becomes observable — bound to a
//! > named local/global, assigned, stored into a collection element or record
//! > field, returned, passed as an argument, or printed/converted. An anonymous
//! > intermediate expression result may be non-finite transiently and may recover
//! > to finite without trapping … At a boundary a `NaN` fails with `ErrFloatNaN`
//! > (`77050013`) and an infinity fails with `ErrFloatOverflow` (`77050015`).
//!
//! [`observe_float`](crate::codegen::engine::builder::CodeBuilder::observe_float)
//! emits that check, but only when the node that produced the value *can* be
//! non-finite — otherwise every boundary in every program would carry a compare
//! and a branch. Before bug-590 that predicate
//! ([`float_arith_node`](crate::codegen::builtins::math::gen_math::float_arith_node))
//! answered `true` for `Binary`/`Unary` alone, on the stated premise that "every
//! other node is finite by construction".
//!
//! **A builtin `Call` is neither, and the premise was false for one member.**
//! `collections::sum`'s `Float` arm folded the elements with a bare
//! `abi::float_add_d` (`collections/func_sum.rs`) — no check, no raise — so
//!
//! ```text
//! LET total AS Float = collections::sum(xs)   ' xs = [1e308, 1e308, 1e308]
//! ```
//!
//! bound `+Inf` to a named local and printed `f64::MAX`'s digits, exit 0. bug-590
//! re-checked its result at each boundary; that left every place a boundary did
//! not see it — `1.0 / collections::sum(xs)` recovered to `0.0`, and an inline
//! `TRAP` received `Ok(+Inf)` (bug-644). Since bug-754 `sum` checks its own total at
//! the call (reason 4 below), so no member is left for a boundary to re-check.

//! # The enumeration is TOTAL
//!
//! **A member whose `Float` can leave binary64 must raise at the call.** The
//! boundary re-check is not a substitute: it fires only where some boundary sees the
//! value, so the same overflow raised when bound and vanished as an operand
//! (bug-754). There is therefore no "re-check at the boundary" classification; a
//! member that cannot prove one of the five reasons below must be fixed to raise.
//!
//! [`FLOAT_RESULTS`] carries one row per builtin member that can return a scalar
//! `Float`, and `float_results_is_total` (below) walks the registry and fails if a
//! member is missing from it, or if a row no longer names such a member. Adding a
//! `Float`-returning builtin without classifying it is therefore a red test, not a
//! silent hole — the lookup itself additionally fails CLOSED (an unclassified
//! member is re-checked at its boundaries), so even a skipped test leaks an `Inf`
//! only through a non-boundary position.
//!
//! The classification is *not* per-member folklore; every `Finite` row falls into
//! one of five documented reasons, recorded on the row:
//!
//! 1. **`Body::Mfb` delegation** — the member's body is injected MFBASIC, so the
//!    value is observed at that `FUNC`'s own `RETURN`, in the callee's frame.
//! 2. **The kernel raises first** — a `math::` kernel with a genuine domain error
//!    or an overflow raises `ErrFloatDomain` (`77050012`) / `ErrFloatInf`
//!    (`77050014`) *at the call*, per the same spec paragraph. Re-checking one
//!    here would raise `ErrFloatOverflow` where the spec assigns `ErrFloatInf`,
//!    which the bug-590 non-goals forbid. (Measured: `math::pow(1.0e300, 2.0)` and
//!    `math::exp(1000.0)` each exit 255 with `7-705-0014`.)
//! 3. **The member only re-reads an already-observed value** — a collection
//!    element, a callback's `RETURN` value, or a value the thread runtime
//!    marshalled out of a frame that observed it.
//! 4. **The member rejects a non-finite itself** — `toFloat` raises `ErrOverflow`
//!    (`77050010`) rather than handing back an infinity (measured:
//!    `toFloat("1e400")` exits 255 with `7-705-0010`), and `collections::sum`
//!    checks its `Float` total and raises `ErrFloatOverflow` (`77050015`).
//! 5. **The call never runs** — `os::getConfigOr` is replaced by a constant
//!    before codegen (plan-167-B): the config's Float, which the manifest rejects
//!    when it is not finite (`1e999` is `PROJECT_JSON_CONFIG_VALUE`), or the call's
//!    literal default.

use crate::codegen::registry::registry;

/// Every builtin member that can return a scalar `Float`, each with the reason
/// (recorded on the row) its result is finite when it reaches its caller. Kept
/// TOTAL by `float_results_is_total`.
///
/// Keyed by member NAME, not by overload: the boundary predicate runs on an
/// untyped NIR node, and `observe_float` has already established that *this* call
/// produced a `Float`.
pub(crate) const FLOAT_RESULTS: &[(&str, &str)] = &[
    // --- collections ---------------------------------------------------------
    // (4) bug-754: the `Float` arm checks its total after the fold and raises
    // `ErrFloatOverflow`. (bug-590 re-checked it at boundaries instead, which missed
    // it as an operand and under an inline `TRAP`.)
    ("collections", "sum"),
    // (3) A read of an element that was observed when it was stored.
    ("collections", "get"),
    ("collections", "getOr"),
    // (3) The accumulator is the user callback's `RETURN` value, observed in the
    // callback's frame. (Measured: reducing [1e308; 3] with `LAMBDA(a, e) -> a + e`
    // exits 255 with `7-705-0015` already.)
    ("collections", "reduce"),
    ("collections", "reduceRight"),
    // --- color ---------------------------------------------------------------
    // (1) `Body::Mfb` — `__color_luminance` / `__color_contrastRatio`.
    ("color", "luminance"),
    ("color", "contrastRatio"),
    // --- general -------------------------------------------------------------
    // (4) `toFloat` raises `ErrOverflow` on text that does not fit binary64 and
    // `ErrInvalidFormat` on text naming a non-finite, per the same spec paragraph.
    ("general", "toFloat"),
    // --- math ----------------------------------------------------------------
    // (2) A `math::` kernel's `Float` overload. `abs`/`min`/`max`/`clamp` echo one
    // of their (already observed) arguments; `sqrt`/`sin`/`cos`/`tan`/`asin`/
    // `acos`/`atan`/`atan2` are finite for every finite argument; `exp`/`log`/
    // `log10`/`pow` raise `ErrFloatInf` / `ErrFloatDomain` themselves.
    ("math", "abs"),
    ("math", "acos"),
    ("math", "asin"),
    ("math", "atan"),
    ("math", "atan2"),
    ("math", "clamp"),
    ("math", "cos"),
    ("math", "exp"),
    ("math", "log"),
    ("math", "log10"),
    ("math", "max"),
    ("math", "min"),
    ("math", "pow"),
    ("math", "sin"),
    ("math", "sqrt"),
    ("math", "tan"),
    // --- os ------------------------------------------------------------------
    // (5) Folded to a finite constant before codegen; no call is emitted.
    ("os", "getConfigOr"),
    // --- thread --------------------------------------------------------------
    // (3) The value was observed at its producer's boundary in the sending frame —
    // `thread::send`'s argument, or the worker's `RETURN`.
    ("thread", "accept"),
    ("thread", "receive"),
    ("thread", "waitFor"),
    // --- vector --------------------------------------------------------------
    // (1) `Body::Mfb` — `__vector_dot_float3` and friends observe at their own
    // `RETURN`. `length`/`distance` additionally feed their sum to `math::sqrt` as
    // an ARGUMENT, which is a boundary in its own right.
    //
    // `dot` is inlined at the call site by `try_inline_vector_op` (plan-01-vector),
    // which bypasses that `RETURN`; bug-590 made the inline arm observe the sum it
    // builds, so both paths raise. The fixture is
    // `tests/rt-error/vector/func_vector_dot_float_overflow`.
    ("vector", "angle"),
    ("vector", "distance"),
    ("vector", "dot"),
    ("vector", "length"),
];

/// Whether a call to `target` can hand back a non-finite `Float`, so an
/// observation boundary consuming it must re-check.
///
/// `target` is a NIR call target: `"collections.sum"` for a builtin member
/// (`Call`, `CallResult` and a `Body::abi_function`'s `RuntimeCall` all spell it
/// with a dot), a bare name for a general builtin (`"toFloat"`) or for a user /
/// injected-package `FUNC` (`"__vector_length_float3"`, `"#vector_dot_float3"`).
pub(crate) fn builtin_float_result_may_be_nonfinite(target: &str) -> bool {
    let (package, member) = target.split_once('.').unwrap_or(("general", target));
    match FLOAT_RESULTS
        .iter()
        .find(|(p, m)| *p == package && *m == member)
    {
        Some(_) => false,
        // Fail CLOSED, but only for a member the registry actually owns and that
        // actually returns a `Float`. An unclassified builtin is re-checked (a
        // redundant check is dead code; a missing one is silently wrong output),
        // while a user `FUNC` or an injected package body keeps the boundary
        // invariant it earns at its own `RETURN` and stays byte-identical.
        None => registry().resolve_func(target).is_some_and(|resolved| {
            resolved
                .function
                .implementations()
                .iter()
                .any(implementation_can_return_float)
        }),
    }
}

/// Whether one overload's return type can resolve to a scalar `Float`.
///
/// `Arg(n)` is the shape that makes a grep census wrong: every `math::` kernel
/// declares `ParameterType::Arg(0)` ("echo the operand's type"), so searching the
/// descriptors for `return_type: ParameterType::Float` finds 4 members and misses
/// 16. `Var`/`Unknown` are counted too — a type variable can bind `Float`.
fn implementation_can_return_float(imp: &crate::codegen::registry::Implementation) -> bool {
    use crate::types::ParameterType;
    fn is_float_like(ty: Option<&ParameterType>) -> bool {
        matches!(
            ty,
            Some(ParameterType::Float) | Some(ParameterType::Var(_)) | Some(ParameterType::Unknown)
        )
    }
    match &imp.return_type {
        ParameterType::Arg(n) => is_float_like(imp.params.get(*n).map(|p| &p.ty)),
        other => is_float_like(Some(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The registry census that makes [`FLOAT_RESULTS`] TOTAL: every builtin member
    /// that can return a scalar `Float` is classified, and every classified row
    /// still names such a member.
    ///
    /// A new `Float`-returning builtin lands here as a red test naming the member,
    /// rather than as a boundary that silently declines to look at its result —
    /// which is the whole of bug-590.
    #[test]
    fn float_results_is_total() {
        let mut census: BTreeSet<(&str, &str)> = BTreeSet::new();
        for package in registry().packages() {
            for function in package.functions() {
                if function
                    .implementations()
                    .iter()
                    .any(implementation_can_return_float)
                {
                    census.insert((package.import_name(), function.name));
                }
            }
        }
        let classified: BTreeSet<(&str, &str)> = FLOAT_RESULTS.iter().copied().collect();

        let missing: Vec<_> = census.difference(&classified).collect();
        assert!(
            missing.is_empty(),
            "bug-590: these builtin members can return a `Float` but are not \
             classified in FLOAT_RESULTS, so an observation boundary would decline \
             to re-check their result: {missing:?}"
        );
        let stale: Vec<_> = classified.difference(&census).collect();
        assert!(
            stale.is_empty(),
            "bug-590: these FLOAT_RESULTS rows no longer name a `Float`-returning \
             builtin member: {stale:?}"
        );
    }

    /// bug-754: no classified member is re-checked at a boundary — `sum` raises at
    /// the call, and a `math::` kernel re-checked here would raise `ErrFloatOverflow`
    /// where the spec assigns `ErrFloatInf`.
    #[test]
    fn no_classified_member_is_rechecked_at_a_boundary() {
        for finite in [
            "collections.sum",
            "collections.get",
            "collections.reduce",
            "color.luminance",
            "math.exp",
            "math.pow",
            "math.sqrt",
            "thread.waitFor",
            "vector.dot",
        ] {
            assert!(
                !builtin_float_result_may_be_nonfinite(finite),
                "{finite} must not be re-checked at a boundary"
            );
        }
    }

    /// A bare target is a general builtin or a user / injected-package `FUNC`, and
    /// neither is re-checked: `toFloat` rejects a non-finite itself, and a `FUNC`
    /// observes its result at its own `RETURN`.
    #[test]
    fn bare_targets_are_general_builtins_or_user_functions() {
        assert!(!builtin_float_result_may_be_nonfinite("toFloat"));
        assert!(!builtin_float_result_may_be_nonfinite("#vector_dot_float3"));
        assert!(!builtin_float_result_may_be_nonfinite("myUserFunction"));
    }

    /// `Arg(0)` is the return shape a grep census misses (see
    /// [`implementation_can_return_float`]): the `math::` kernels are only in the
    /// census because it resolves `Arg(n)` against the overload's parameters.
    #[test]
    fn arg_returns_are_in_the_census() {
        let classified: BTreeSet<(&str, &str)> = FLOAT_RESULTS.iter().copied().collect();
        assert!(classified.contains(&("math", "sqrt")));
        assert!(classified.contains(&("math", "exp")));
    }
}
