//! `math::max` — element-wise maximum of two same-type numeric values or lists.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType::{Fixed, Float, Integer, Money};
const INTRO: &str = r#"The larger of two same-type numeric values or lists."#;
const DESC: &str = r#"`max` returns the larger of `a` and `b`, which must be the same numeric type
(`Integer`, `Float`, `Fixed`, or `Money`), echoing that type. The `List OF
Integer`/`Float`/`Fixed` array forms take two equal-length lists and return the
element-wise maximum; mismatched lengths raise `ErrInvalidArgument`."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::max(3, 5)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_binary(
        "max",
        INTRO,
        DESC,
        EX,
        "same numeric type, same numeric type",
        ("a", &["left"]),
        ("b", &["right"]),
        &[Integer, Float, Fixed, Money],
        &[Integer, Float, Fixed],
        &["ErrInvalidArgument"],
        lower_math_max,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::max`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_max(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("max", args)
}
