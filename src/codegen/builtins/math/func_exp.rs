//! `math::exp` — natural exponential of a `Float`/`Fixed` value or `Float` list.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Natural exponential (e raised to the power) of a value or list."#;
const DESC: &str = r#"`exp` returns `e ** value`, echoing the operand type (`Float` or `Fixed`), plus the
`List OF Float` vectorized form. `exp(0.0)` is `1`, and a
negative argument gives a fraction; a very negative `Float` argument gives `0`
(`math::exp(-800.0)`). A result too large to hold raises `ErrFloatInf` for a `Float`
or `List OF Float` (`math::exp(710.0)`) and `ErrOverflow` for a `Fixed`
(`math::exp(30.0F)`)."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::exp(1.0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary_typed_errors(
        "exp",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        "The exponent to raise e to, or a `List OF Float` of them. A large value overflows the result range.",
        &[Float, Fixed],
        &[Float],
        // bug-617: `ErrOverflow` is FIXED-only — `emit_fixed_exp`'s recombination
        // raises it (measured: exp(toFixed(30.0)) -> 77050010), while the `Float`
        // kernel has no overflow path and signals an unrepresentable result as
        // `ErrFloatInf` instead (measured: exp(710.0) -> 77050014). `ErrFloatNaN` is
        // the Float kernel's own guard and stays with it.
        &[],
        &[
            (Float, &["ErrFloatInf", "ErrFloatNaN"]),
            (Fixed, &["ErrOverflow"]),
        ],
        lower_math_exp,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::exp`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_exp(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("exp", args)
}
