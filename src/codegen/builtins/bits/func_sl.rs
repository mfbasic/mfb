//! `bits::sl` — logical left shift of a 64-bit integer.
//!
//! Descriptor + docs migrated from `src/docs/man/builtins/bits/sl.md`; lowering
//! from the former `src/target/shared/code/builder_bits.rs::lower_bits_shift`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const INTRO: &str = r#"Logical left shift of a 64-bit integer."#;
const DESC: &str = r#"`sl` shifts `value` left by `count` bit positions. Vacated low bits are filled
with zero, and bits shifted past bit 63 are discarded, so the result keeps only
the low 64 bits of the shifted value. A `count` of `0` returns `value`
unchanged.

Both `value` and the result are raw two's-complement 64-bit `Integer` bit
patterns; `sl` does not interpret sign, and it makes no distinction between a
logical and an arithmetic left shift. For the sign-preserving right shift see
`bits::sra`; for the zero-filling right shift see `bits::sr`.

Unlike the total bitwise operations, `sl` validates `count`: it first checks
that `count` is in the range `0` to `63` inclusive and raises
`ErrInvalidArgument` for any value outside it, before performing the shift. The
operation has no side effects and lowers to a native variable-shift instruction
inline rather than calling a runtime helper, producing identical results on the
native and Binary Representation execution paths."#;
const EX: &str = r#"Shift a value left by four bits (multiply by 16):

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::sl(1, 4)
  io::print(toString(result))
END SUB
```

Build a byte-packed field by shifting a value into place and combining it with
`bits::bor`:

```
IMPORT bits
IMPORT io

SUB main()
  LET high AS Integer = bits::sl(0xAB, 8)
  LET packed AS Integer = bits::bor(high, 0xCD)
  io::print(toString(packed))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sl",
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
            body: Body::native(None, None, Some(lower_bits_sl)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::sl`.
pub(crate) fn lower_bits_sl(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    super::native::lower_bits_shift(builder, "sl", args)
}
