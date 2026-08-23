//! `math::log10` — base-10 logarithm of a `Float`/`Fixed` value or list.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Base-10 logarithm of a Float or Fixed value or list."#;
const DESC: &str = r#"`log10` returns the base-10 logarithm of `value`, echoing the operand type (`Float`
or `Fixed`), plus the `List OF Float`/`List OF Fixed` forms. A non-positive argument
is outside the domain and raises `ErrFloatDomain` (scalar) or `ErrInvalidArgument`
(array)."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::log10(1000.0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary(
        "log10",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        &[Float, Fixed],
        &[Float, Fixed],
        &["ErrFloatDomain", "ErrInvalidArgument"],
        lower_math_log10,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::log10`. Slice B shim.
pub(crate) fn lower_math_log10(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("log10", args)
}
