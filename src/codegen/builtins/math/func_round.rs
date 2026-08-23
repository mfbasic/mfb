//! `math::round` — round half away from zero, exiting to `Integer`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float, Money};
const INTRO: &str = r#"Round to the nearest whole number, half away from zero."#;
const DESC: &str = r#"`round` returns the nearest integer to `value`, rounding halves away from zero. It
accepts `Float`, `Fixed`, and `Money` and returns `Integer` (a deliberate dimension
exit — for `Money`, the whole-unit count under a fixed half-away rule, distinct from
`money::round`), plus the `List OF Float`/`List OF Fixed` array forms returning
`List OF Integer`. A magnitude too large for `Integer` raises `ErrOverflow`."#;
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
        &[Float, Fixed, Money],
        &[Float, Fixed],
        &["ErrOverflow"],
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
