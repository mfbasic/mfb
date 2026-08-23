//! `math::clamp` — restrict a value (or list) to an inclusive `[low, high]` range.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType::{self, Fixed, Float, Integer, Money};

use super::{overload, req};
const INTRO: &str = r#"Restrict a value or list to an inclusive [low, high] range."#;
const DESC: &str = r#"`clamp` returns `value` restricted to `[low, high]`: `low` when `value < low`,
`high` when `value > high`, and `value` otherwise. All three arguments must be the
same numeric type (`Integer`, `Float`, `Fixed`, or `Money`), echoing that type. The
array form clamps a `List OF Integer`/`Float`/`Fixed` against two scalar bounds of
the element type. `low` must not exceed `high`, else `ErrInvalidArgument`."#;
const EX: &str = r#"```
IMPORT math
IMPORT io
SUB main()
  io::print(toString(math::clamp(12, 0, 10)))
END SUB
```"#;

const LOW: &[&str] = &["minimum"];
const HIGH: &[&str] = &["maximum"];

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let mut impls: Vec<Implementation> = Vec::new();
    // Array `(List OF T, T, T) AS List OF T` first (lenient resolution coarsely
    // accepts a scalar pattern against a `List OF` concrete, so a scalar-first order
    // would echo the wrong shape for a list argument — see `super::preserving_unary`).
    for ty in [Integer, Float, Fixed] {
        impls.push(overload(
            vec![
                req("value", &[], ParameterType::list_of(ty.clone())),
                req("low", LOW, ty.clone()),
                req("high", HIGH, ty.clone()),
            ],
            ParameterType::Arg(0),
            vec!["ErrInvalidArgument"],
            lower_math_clamp,
        ));
    }
    // Scalar `(T, T, T) AS T`.
    for ty in [Integer, Float, Fixed, Money] {
        impls.push(overload(
            vec![
                req("value", &[], ty.clone()),
                req("low", LOW, ty.clone()),
                req("high", HIGH, ty.clone()),
            ],
            ParameterType::Arg(0),
            vec!["ErrInvalidArgument"],
            lower_math_clamp,
        ));
    }
    pkg.add_function(RegistryFunction {
        name: "clamp",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("numeric value, numeric low, numeric high of the same type"),
        internal_only: false,
        implementations: impls,
    });
}

/// Target-generic call-site lowering for `math::clamp`. Slice B shim.
pub(crate) fn lower_math_clamp(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_math_call("clamp", args)
}
