//! `math::round` — round half away from zero, exiting to `Integer`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float, Money};
const INTRO: &str = r#"Round to the nearest whole number, half away from zero."#;
const DESC: &str = r#"`round` returns the nearest integer to `value`, rounding halves away from zero. It
accepts `Float`, `Fixed`, and `Money` and returns an `Integer`; for `Money` the
result is a whole number of currency units, always rounding halves away from zero,
unlike `money::round`. A `List OF Float` or `List OF Fixed` returns a new
`List OF Integer` with each element rounded in the same order, leaving the input
unchanged; an empty list gives an empty list. Only a `Float` can be too large for
`Integer`, and then it raises `ErrOverflow`: a `Fixed` or `Money` result always
fits."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::round(2.5)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::rounding(
        "round",
        INTRO,
        DESC,
        EX,
        "Float | Fixed | Money",
        "The number to round to the nearest whole value, or a list of them. Halves round away from zero.",
        &[Float, Fixed, Money],
        &[Float, Fixed],
        // bug-617: only the `Float` family range-checks the rounded result against
        // `Integer` (`emit_float_rounding_integer_range_check`). The `Fixed` and
        // `Money` rounding paths contain no raise at all — their rounded value always
        // fits, measured to within two whole units of the Money maximum.
        &[],
        &[(Float, &["ErrOverflow"])],
        lower_math_round,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::round`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_round(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("round", args)
}
