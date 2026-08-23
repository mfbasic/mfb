//! `math::min` — element-wise minimum of two same-type numeric values or lists.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float, Integer, Money};
const INTRO: &str = r#"The smaller of two same-type numeric values or lists."#;
const DESC: &str = r#"`min` returns the smaller of `a` and `b`, which must be the same numeric type
(`Integer`, `Float`, `Fixed`, or `Money`), echoing that type. The `List OF
Integer`/`Float`/`Fixed` array forms take two equal-length lists and return the
element-wise minimum; mismatched lengths raise `ErrInvalidArgument`."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::min(3, 5)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_binary(
        "min",
        INTRO,
        DESC,
        EX,
        "same numeric type, same numeric type",
        ("a", &["left"]),
        ("b", &["right"]),
        &[Integer, Float, Fixed, Money],
        &[Integer, Float, Fixed],
        &["ErrInvalidArgument"],
        lower_math_min,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::min`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_min(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("min", args)
}
