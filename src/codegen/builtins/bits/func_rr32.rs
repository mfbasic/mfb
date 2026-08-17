//! `bits::rr32` — rotate the low 32 bits of an integer right.
//!
//! Descriptor + docs migrated from `src/docs/man/builtins/bits/rr32.md`; lowering
//! from the former `src/target/shared/code/builder_bits.rs::lower_bits_rotate`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::{CodeBuilder, Operand, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const INTRO: &str = r#"Rotate the low 32 bits of an integer right."#;
const DESC: &str = r#"`rr32` rotates the low 32 bits of `value` right by `count` bit positions and
returns the result zero-extended into bits 32..63. The rotate is a 32-bit barrel
rotate: bits shifted out of bit 0 re-enter at bit 31, so no information is lost.
The high 32 bits of `value` are ignored, and bits 32..63 of the result are always
zero.

The rotate amount is reduced modulo 32, so every `count` produces a defined
result: a `count` of `0` or `32` leaves the low 32 bits unchanged, and a negative
`count` is also reduced modulo 32, making a right rotate by a negative amount
equivalent to a left rotate. Unlike the `bits` shifts (`sl`/`sr`/`sra`), the
rotates do not validate `count` and never raise an error.

Both `value` and the result are raw two's-complement 64-bit `Integer` bit
patterns; `rr32` does not interpret sign. AArch64 provides a rotate-right
instruction directly, so `rr32` lowers to a 32-bit word rotate-right by `count`
(the hardware uses only the low 5 bits of `count`, giving the modulo-32
reduction); the operation has no side effects and lowers inline rather than
calling a runtime helper, producing identical results on the native and Binary
Representation execution paths."#;
const EX: &str = r#"Rotate the low 32 bits right by four positions:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::rr32(1, 4)
  io::print(toString(result))
END SUB
```

Rotate a 32-bit mask and recombine it with `bits::bor`:

```
IMPORT bits
IMPORT io

SUB main()
  LET rotated AS Integer = bits::rr32(0x0000FFFF, 8)
  LET merged AS Integer = bits::bor(rotated, 0xFF)
  io::print(toString(merged))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "rr32",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The value whose low 32 bits are rotated. Bits 32..63 are ignored; treated as a raw bit pattern.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "count",
                    desc: "The number of bit positions to rotate right, reduced modulo 32. Any value, including negative counts, is accepted.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::native(None, None, Some(lower_bits_rr32)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::rr32`.
pub(crate) fn lower_bits_rr32(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let (value_reg, count_reg, value_text, count_text) =
        super::gen_two_integers::lower_bits_two_integers(builder, "rr32", args)?;
    let dst = builder.allocate_register()?;
    builder.emit(abi::rotate_right_word_registers(dst, value_reg, count_reg));
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.rr32({value_text}, {count_text})"),
    })
}
