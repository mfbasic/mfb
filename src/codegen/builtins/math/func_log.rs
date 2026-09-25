//! `math::log` — natural logarithm of a `Float`/`Fixed` value or list.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, RegistryPackage};
use crate::types::ParameterType::{Fixed, Float};
const INTRO: &str = r#"Natural logarithm of a Float or Fixed value or list."#;
const DESC: &str = r#"`log` returns the natural logarithm (base `e`) of `value`, echoing the operand type
(`Float` or `Fixed`), plus `List OF Float`/`List OF Fixed` forms that return a new
list of the same type in the same order, leaving the input unchanged; an empty list
gives an empty list. A non-positive
argument is outside the domain: a `Float` or `List OF Float` argument raises
`ErrFloatDomain`, and a `Fixed` or `List OF Fixed` argument raises
`ErrInvalidArgument`."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::log(math::e)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    super::preserving_unary_typed_errors(
        "log",
        INTRO,
        DESC,
        EX,
        "Float | Fixed",
        "The number to take the natural logarithm of, or a list of them. It, or every element, must be greater than zero.",
        &[Float, Fixed],
        &[Float, Fixed],
        // bug-617: disjoint per-type split, measured. The `Float` family raises
        // `ErrFloatDomain` (77050012) from its kernel's domain trap; the `Fixed`
        // family raises `ErrInvalidArgument` (77050002) from its own bare check.
        // Neither can raise the other's, on either the scalar or the `List OF` form.
        &[],
        &[
            (Float, &["ErrFloatDomain"]),
            (Fixed, &["ErrInvalidArgument"]),
        ],
        lower_math_log,
        pkg,
    );
}

/// Target-generic call-site lowering for `math::log`, delegating to the shared `lower_math_call` carrier in `gen_math.rs`.
pub(crate) fn lower_math_log(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("log", args)
}
