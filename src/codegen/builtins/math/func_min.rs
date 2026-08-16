//! `math::min` — element-wise minimum of two same-type numeric values or lists.

use crate::codegen::registry::RegistryPackage;
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
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

pub(super) fn register(pkg: &mut RegistryPackage) {
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

/// Target-generic call-site lowering for `math::min`. Slice B shim.
pub(crate) fn lower_math_min(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    builder.lower_math_call("min", args)
}
