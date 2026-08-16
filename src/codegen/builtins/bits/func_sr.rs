//! `bits::sr` — logical (zero-filling) right shift of a 64-bit integer.
//!
//! Descriptor + docs migrated from `src/docs/man/builtins/bits/sr.md`; lowering
//! from the former `src/target/shared/code/builder_bits.rs::lower_bits_shift`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const INTRO: &str = r#"Logical (zero-filling) right shift of a 64-bit integer."#;
const DESC: &str = r#"`sr` shifts `value` right by `count` bit positions as an unsigned quantity.
Vacated high bits are filled with zero, and bits shifted past bit 0 are
discarded. A `count` of `0` returns `value` unchanged.

Both `value` and the result are raw two's-complement 64-bit `Integer` bit
patterns; `sr` does not interpret sign. Because the vacated high bits are always
zeroed, the sign bit is *not* replicated — this is the distinction from the
arithmetic right shift `bits::sra`, which preserves the sign bit. For the
left shift see `bits::sl`.

Unlike the total bitwise operations, `sr` validates `count`: it first checks
that `count` is in the range `0` to `63` inclusive and raises
`ErrInvalidArgument` for any value outside it, before performing the shift.
Larger shift amounts are not
implicitly clamped or reduced modulo the width. The operation has no side
effects and lowers to a native variable-shift instruction inline rather than
calling a runtime helper, producing identical results on the native and Binary
Representation execution paths."#;
const EX: &str = r#"Shift a value right by four bits (unsigned divide by 16):

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::sr(256, 4)
  io::print(toString(result))
END SUB
```

Extract a byte-packed field by shifting it down into place and masking with
`bits::band`:

```
IMPORT bits
IMPORT io

SUB main()
  LET packed AS Integer = 0xABCD
  LET high AS Integer = bits::band(bits::sr(packed, 8), 255)
  io::print(toString(high))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sr",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The value to shift. Any 64-bit value; treated as a raw bit pattern.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "count",
                    desc: "The shift amount in bits. Must be in the range `0` to `63` inclusive; any other value raises `ErrInvalidArgument`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec!["ErrInvalidArgument"],
            body: Body::native(None, None, Some(lower_bits_sr)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::sr`.
pub(crate) fn lower_bits_sr(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    super::native::lower_bits_shift(builder, "sr", args)
}
