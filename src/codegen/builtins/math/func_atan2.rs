//! `math::atan2` — arctangent of `y / x` using the quadrant of both signs.

use crate::codegen::registry::RegistryPackage;
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType::{Fixed, Float};

const INTRO: &str = r#"Arctangent of y / x, using the signs of both to pick the quadrant."#;
const DESC: &str = r#"`atan2` returns the angle in radians (in `(-pi, pi]`) between the positive x-axis
and the point `(x, y)`, using the signs of both arguments to select the correct
quadrant. Both arguments must be the same type (`Float` or `Fixed`), echoing that
type; the `List OF Float` array form takes two equal-length lists (mismatched
lengths raise `ErrInvalidArgument`)."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::atan2(1.0, 1.0)))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    super::preserving_binary(
        "atan2",
        INTRO,
        DESC,
        EX,
        "Float | Fixed, same type",
        ("y", &[]),
        ("x", &[]),
        &[Float, Fixed],
        &[Float],
        &["ErrFloatNaN", "ErrInvalidArgument"],
        lower_math_atan2,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::atan2`. Slice B shim.
pub(crate) fn lower_math_atan2(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    builder.lower_math_call("atan2", args)
}
