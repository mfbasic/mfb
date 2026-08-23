//! `math::pow` — raise a base to an exponent (scalar + vectorized).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Raise a base to an exponent."#;
const DESC: &str = r#"`pow` returns `base ** exponent`. Both arguments must be the same type (`Float` or
`Fixed`), echoing that type; the `List OF Float` array form raises two equal-length
lists element-wise. A result that overflows the finite range or is otherwise
non-finite raises `ErrOverflow`/`ErrFloatInf`/`ErrFloatNaN`; a negative base with a
non-integer exponent is outside the domain."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::pow(2.0, 10.0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_binary(
        "pow",
        INTRO,
        DESC,
        EX,
        "Float | Fixed, same type",
        ("base", &["value"]),
        ("exponent", &["power"]),
        &[Float, Fixed],
        &[Float],
        &[
            "ErrFloatInf",
            "ErrFloatNaN",
            "ErrInvalidArgument",
            "ErrOverflow",
        ],
        lower_math_pow,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::pow`. Slice B shim.
pub(crate) fn lower_math_pow(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("pow", args)
}
