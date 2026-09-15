//! `math::floor` — round toward negative infinity, exiting to `Integer`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float, Money};
const INTRO: &str = r#"Round toward negative infinity to a whole number."#;
const DESC: &str = r#"`floor` returns the greatest integer not greater than `value`. It accepts `Float`,
`Fixed`, and `Money` and returns an `Integer`; for `Money` the result is a whole
number of currency units. A `List OF Float` or `List OF Fixed` returns a new
`List OF Integer` with each element rounded down in the same order, leaving the
input unchanged; an empty list gives an empty list. Only a `Float` can be too large
for `Integer`, and then it raises `ErrOverflow`: a `Fixed` or `Money` result always
fits."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::floor(2.7)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::rounding(
        "floor",
        INTRO,
        DESC,
        EX,
        "Float | Fixed | Money",
        "The number to round down, or a `List OF Float` or `List OF Fixed` of them. Rounds toward negative infinity, so -1.5 becomes -2.",
        &[Float, Fixed, Money],
        &[Float, Fixed],
        &["ErrOverflow"],
        lower_math_floor,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::floor`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_floor(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("floor", args)
}
