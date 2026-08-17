//! `bits::rr64` — rotate all 64 bits of an integer right.
//!
//! Descriptor + docs migrated from `src/docs/man/builtins/bits/rr64.md`; lowering
//! from the former `src/target/shared/code/builder_bits.rs::lower_bits_rotate`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::{CodeBuilder, Operand, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const INTRO: &str = r#"Rotate all 64 bits of an integer right."#;
const DESC: &str = r#"`rr64` rotates all 64 bits of `value` right by `count` bit positions and returns
the result. The rotate is a full-width 64-bit barrel rotate: bits shifted out of
bit 0 re-enter at bit 63, so no information is lost and every bit of `value`
appears in the result. Unlike `bits::rr32`, no bits are ignored and no part of
the result is forced to zero.

The rotate amount is reduced modulo 64, so every `count` produces a defined
result: a `count` of `0` or `64` leaves `value` unchanged, and a negative
`count` is also reduced modulo 64, making a right rotate by a negative amount
equivalent to a left rotate. Unlike the `bits` shifts (`sl`/`sr`/`sra`), the
rotates do not validate `count` and never raise an error.

Both `value` and the result are raw two's-complement 64-bit `Integer` bit
patterns; `rr64` does not interpret sign. AArch64 provides a rotate-right
instruction directly (`RORV`), so `rr64` lowers to a 64-bit rotate-right by
`count`, with the hardware reducing the amount modulo the register width; the
operation has no side effects and lowers inline rather than calling a runtime
helper, producing identical results on the native and Binary Representation
execution paths."#;
const EX: &str = r#"Rotate all 64 bits right by four positions:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::rr64(1, 4)
  io::print(toString(result))
END SUB
```

Move the low byte of a 64-bit value into the top byte with a right rotate:

```
IMPORT bits
IMPORT io

SUB main()
  LET rotated AS Integer = bits::rr64(0xFF, 8)
  io::print(toString(rotated))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "rr64",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
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
                    desc: "The number of bit positions to rotate right, reduced modulo 64. Any value, including negative counts, is accepted.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::native(None, None, Some(lower_bits_rr64)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::rr64`.
pub(crate) fn lower_bits_rr64(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let (value_reg, count_reg, value_text, count_text) =
        super::gen_two_integers::lower_bits_two_integers(builder, "rr64", args)?;
    let dst = builder.allocate_register()?;
    builder.emit(abi::rotate_right_registers(dst, value_reg, count_reg));
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.rr64({value_text}, {count_text})"),
    })
}
