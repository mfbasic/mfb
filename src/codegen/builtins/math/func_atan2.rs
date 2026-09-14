//! `math::atan2` — arctangent of `y / x` using the quadrant of both signs.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Arctangent of y / x, using the signs of both to pick the quadrant."#;
const DESC: &str = r#"`atan2` returns the angle in radians (in `[-pi, pi]`) between the positive x-axis
and the point `(x, y)`, using the signs of both arguments to select the correct
quadrant. On the axes: `atan2(0.0, 0.0)` is `0`; with `x` zero the result is `pi/2`
or `-pi/2` by the sign of `y`; and on the negative x-axis the sign of `y` still
counts, so `atan2(0.0, -1.0)` is `pi` and a `y` of `-0.0` gives `-pi`. Both arguments must be the same type (`Float` or `Fixed`), echoing that
type; the `List OF Float` array form takes two equal-length lists and returns a new
list of angles, leaving both inputs unchanged (mismatched lengths raise
`ErrInvalidArgument`)."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::atan2(1.0, 1.0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_binary(
        "atan2",
        INTRO,
        DESC,
        EX,
        "Float | Fixed, same type",
        ("y", &[], "The y coordinate — the numerator of the ratio whose angle is wanted."),
        ("x", &[], "The x coordinate. Its sign is what places the result in the correct quadrant, which is why `atan2(y, x)` beats `atan(y / x)`."),
        &[Float, Fixed],
        &[Float],
        &["ErrFloatNaN", "ErrInvalidArgument"],
        lower_math_atan2,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::atan2`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_atan2(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("atan2", args)
}
