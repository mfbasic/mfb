//! `bits::rl64` — rotate all 64 bits of an integer left.
//!
//! Descriptor + docs migrated from `src/docs/man/builtins/bits/rl64.md`; lowering
//! from the former `src/target/shared/code/builder_bits.rs::lower_bits_rotate`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const INTRO: &str = r#"Rotate all 64 bits of an integer left."#;
const DESC: &str = r#"`rl64` rotates all 64 bits of `value` left by `count` bit positions and returns
the result. The rotate is a full-width 64-bit barrel rotate: bits shifted out of
bit 63 re-enter at bit 0, so no information is lost and every bit of `value`
appears in the result. Unlike `bits::rl32`, no bits are ignored and no part of
the result is forced to zero.

The rotate amount is reduced modulo 64, so every `count` produces a defined
result: a `count` of `0` or `64` leaves `value` unchanged, and a negative
`count` is also reduced modulo 64, making a left rotate by a negative amount
equivalent to a right rotate. Unlike the `bits` shifts (`sl`/`sr`/`sra`), the
rotates do not validate `count` and never raise an error.

Both `value` and the result are raw two's-complement 64-bit `Integer` bit
patterns; `rl64` does not interpret sign. AArch64 provides only a rotate-right
instruction (`RORV`), so a left rotate is lowered as a 64-bit rotate-right by
`0 - count` (the hardware uses only the low 6 bits of that amount, giving the
modulo-64 reduction); the
operation has no side effects and lowers inline rather than calling a runtime
helper, producing identical results on the native and Binary Representation
execution paths."#;
const EX: &str = r#"Rotate all 64 bits left by four positions:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::rl64(1, 4)
  io::print(toString(result))
END SUB
```

Move the top byte of a 64-bit value into the low byte with a left rotate:

```
IMPORT bits
IMPORT io

SUB main()
  LET rotated AS Integer = bits::rl64(0xFF00000000000000, 8)
  io::print(toString(rotated))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "rl64",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The value whose 64 bits are rotated. Treated as a raw bit pattern.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "count",
                    desc: "The number of bit positions to rotate left, reduced modulo 64. Any value, including negative counts, is accepted.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::native(None, None, Some(lower_bits_rl64)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::rl64`.
pub(crate) fn lower_bits_rl64(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    super::native::lower_bits_rotate(builder, "rl64", args)
}
