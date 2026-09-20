//! `math::pow` — raise a base to an exponent (scalar + vectorized).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Raise a base to an exponent."#;
const DESC: &str = r#"`pow` returns `base ** exponent`. Both arguments must be the same type (`Float` or
`Fixed`), echoing that type; the `List OF Float` form raises two lists of the same length element-wise
into a new list, and lists of different lengths raise `ErrInvalidArgument`.
`pow(0.0, 0.0)` is `1`, and a negative exponent gives a reciprocal
(`pow(2.0, -2.0)` is `0.25`). A negative base needs a whole-number exponent: a
fractional one raises `ErrFloatNaN` for `Float` and `ErrInvalidArgument` for
`Fixed`. A result too large to hold — including zero to a negative power — raises
`ErrFloatInf` for `Float` and `ErrOverflow` for `Fixed`."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::pow(2.0, 10.0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_binary(
        "pow",
        INTRO,
        DESC,
        EX,
        "Float | Fixed, same type",
        ("base", &["value"], "The base, or a `List OF Float` of them."),
        (
            "exponent",
            &["power"],
            "The exponent, or a `List OF Float` of them. Must be the same type as the base; two lists must also have the same length.",
        ),
        &[Float, Fixed],
        &[Float],
        // bug-617: all four errors were declared on all three overloads; each form
        // raises a strict subset. The `Float` path's only raise site is
        // `emit_float_result_check` (ErrFloatInf / ErrFloatNaN) — it has no
        // ErrOverflow. The `Fixed` path never leaves Q32.32, so it raises
        // ErrOverflow and (via `emit_fixed_log`, for a fractional exponent on a
        // non-positive base) ErrInvalidArgument, and no float-class error. The DESC
        // prose already stated this split correctly; only the machine-readable list
        // disagreed.
        &[],
        &["ErrInvalidArgument"],
        &[
            (Float, &["ErrFloatInf", "ErrFloatNaN"]),
            (Fixed, &["ErrInvalidArgument", "ErrOverflow"]),
        ],
        lower_math_pow,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::pow`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_pow(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("pow", args)
}
