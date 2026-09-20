//! `math::asin` — arcsine of a `Float`/`Fixed` value or `Float` list (radians).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Arcsine (inverse sine), returning radians."#;
const DESC: &str = r#"`asin` returns the arcsine of `value` in radians, echoing the operand type (`Float`
or `Fixed`), plus the `List OF Float` vectorized form. `value` must be in `[-1, 1]`;
outside that domain a `Float` or `List OF Float` argument raises `ErrFloatDomain`,
and a `Fixed` argument raises `ErrInvalidArgument`."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::asin(1.0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary_typed_errors(
        "asin",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        "The sine to invert, or a list of them. Must be within -1 through 1; outside that there is no angle and the call raises.",
        &[Float, Fixed],
        &[Float],
        // bug-617: disjoint per-type split, measured. The `Float` family raises
        // `ErrFloatDomain` (77050012) from its kernel's domain trap; the `Fixed`
        // family raises `ErrInvalidArgument` (77050002) from its own bare check.
        // Neither can raise the other's, on either the scalar or the `List OF` form.
        &[],
        &[
            (Float, &["ErrFloatDomain"]),
            (Fixed, &["ErrInvalidArgument"]),
        ],
        lower_math_asin,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::asin`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_asin(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("asin", args)
}
