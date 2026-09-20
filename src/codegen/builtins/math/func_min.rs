//! `math::min` — element-wise minimum of two same-type numeric values or lists.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float, Integer, Money};
const INTRO: &str = r#"The smaller of two same-type numeric values or lists."#;
const DESC: &str = r#"`min` returns the smaller of `a` and `b`, which must be the same numeric type
(`Integer`, `Float`, `Fixed`, or `Money`), echoing that type. The `List OF
Integer`/`Float`/`Fixed` array forms take two lists of the same element type and
length and return a new list of the element-wise minimum, leaving both inputs
unchanged; two empty lists give an empty list, and mismatched lengths raise
`ErrInvalidArgument`."#;
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
        ("a", &["left"], "The first value, or a `List OF Integer`, `Float`, or `Fixed`."),
        (
            "b",
            &["right"],
            "The second value, or a list. Must be the same type as the first; two lists must also have the same length.",
        ),
        &[Integer, Float, Fixed, Money],
        &[Integer, Float, Fixed],
        // bug-617: `lower_math_min_max` (the scalar arm) contains no raise site at
        // all — it is compare-and-select. The only raise is `lower_simd_binary`'s
        // length check, which the scalar forms have no length to fail.
        &[],
        &["ErrInvalidArgument"],
        &[],
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
