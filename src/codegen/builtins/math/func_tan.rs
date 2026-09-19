//! `math::tan` — tangent of a `Float`/`Fixed` value or `Float` list (radians).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Tangent of an angle in radians."#;
const DESC: &str = r#"`tan` returns the tangent of `value` (an angle in radians), echoing the operand type
(`Float` or `Fixed`), plus the `List OF Float` vectorized form. Near an odd multiple of
pi/2 the tangent grows without bound, and the two operand types answer differently
there. A `Float` result is very large but finite (`math::tan(math::pi2)` is about
1.6e16), because no `Float` lands exactly on pi/2. A `Fixed` result covers only
about ±2.1e9, so an angle close enough to pi/2 has no representable tangent at
all: that raises `ErrOverflow` rather than returning a wrong number, and
`math::tan(math::pi2Fixed)` is such an angle. Use `Float` when the angle can come
close to pi/2 and you want a value instead of an error.

A `Float` angle of any size is measured against pi/2 exactly, so a huge angle is as
accurate as a small one: `math::tan(100000000000000000000.0)` is about `-0.845`."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::tan(0.0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary_typed_errors(
        "tan",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        "The angle in radians, or a `List OF Float` of them. Near an odd multiple of pi/2 the result becomes very large; see the description for how Float and Fixed differ there.",
        &[Float, Fixed],
        &[Float],
        &["ErrFloatInf", "ErrFloatNaN", "ErrInvalidArgument"],
        // bug-615: only the `Fixed` overload can overflow — its result type has a
        // ±2.1e9 range the true tangent leaves near an odd multiple of pi/2.
        Some((Fixed, &["ErrOverflow"])),
        lower_math_tan,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::tan`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_tan(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("tan", args)
}
