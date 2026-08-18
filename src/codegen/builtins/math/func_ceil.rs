//! `math::ceil` — round toward positive infinity, exiting to `Integer`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::RegistryPackage;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType::{Fixed, Float, Money};
const INTRO: &str = r#"Round toward positive infinity to a whole number."#;
const DESC: &str = r#"`ceil` returns the least integer not less than `value`. It accepts `Float`, `Fixed`,
and `Money` and returns `Integer` (a deliberate dimension exit — for `Money`, the
whole-unit count), plus the `List OF Float`/`List OF Fixed` array forms returning
`List OF Integer`. A magnitude too large for `Integer` raises `ErrOverflow`."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::ceil(2.1)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::rounding(
        "ceil",
        INTRO,
        DESC,
        EX,
        "Float | Fixed | Money",
        &[Float, Fixed, Money],
        &[Float, Fixed],
        &["ErrOverflow"],
        lower_math_ceil,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::ceil`. Slice B shim.
pub(crate) fn lower_math_ceil(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    builder.lower_math_call("ceil", args)
}
