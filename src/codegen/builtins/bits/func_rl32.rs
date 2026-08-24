//! `bits::rl32` — rotate the low 32 bits of an integer left.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTRO: &str = r#"Rotate the low 32 bits of an integer left."#;
const DESC: &str = r#"`rl32` rotates the low 32 bits of `value` left by `count` bit positions and
returns the result zero-extended into bits 32..63. The rotate is a 32-bit barrel
rotate: bits shifted out of bit 31 re-enter at bit 0, so no information is lost.
The high 32 bits of `value` are ignored, and bits 32..63 of the result are always
zero.

The rotate amount is reduced modulo 32, so every `count` produces a defined
result: a `count` of `0` or `32` leaves the low 32 bits unchanged, and a negative
`count` is also reduced modulo 32, making a left rotate by a negative amount
equivalent to a right rotate. Unlike the `bits` shifts (`sl`/`sr`/`sra`), the
rotates do not validate `count` and never raise an error.

Both `value` and the result are raw two's-complement 64-bit `Integer` bit
patterns; `rl32` does not interpret sign. AArch64 provides only a rotate-right
instruction, so a left rotate is lowered as a 32-bit rotate-right by `0 - count`
(the hardware uses only the low 5 bits of that amount, giving the modulo-32
reduction); the operation has no side effects and lowers inline rather than calling a runtime
helper, producing identical results on the native and Binary Representation
execution paths."#;
const EX: &str = r#"Rotate the low 32 bits left by four positions:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::rl32(1, 4)
  io::print(toString(result))
END SUB
```

Rotate a 32-bit mask and recombine it with `bits::bor`:

```
IMPORT bits
IMPORT io

SUB main()
  LET rotated AS Integer = bits::rl32(0x0000FFFF, 8)
  LET merged AS Integer = bits::bor(rotated, 0xFF)
  io::print(toString(merged))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "rl32",
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
                    desc: "The number of bit positions to rotate left, reduced modulo 32. Any value, including negative counts, is accepted.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_inline(lower_bits_rl32),
        }],
    });
}

/// Target-generic call-site lowering for `bits::rl32`.
pub(crate) fn lower_bits_rl32(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args[0].type_ != ParameterType::Integer {
        return Err(format!("bits.rl32 does not accept {}", args[0].type_));
    }
    if args[1].type_ != ParameterType::Integer {
        return Err(format!("bits.rl32 does not accept {}", args[1].type_));
    }
    let value_reg = args[0].location.clone();
    let count_reg = args[1].location.clone();
    let value_text = &args[0].text;
    let count_text = &args[1].text;
    let dst = builder.allocate_register()?;
    // AArch64 has only rotate-right (`RORV`), so rotate-left by `count` is a
    // rotate-right by `-count` (the hardware reduces the amount modulo the width).
    let neg = builder.allocate_register()?;
    builder.emit(abi::subtract_registers(neg, abi::ZERO, count_reg));
    builder.emit(abi::rotate_right_word_registers(dst, value_reg, neg));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(dst.render()),
        text: format!("bits.rl32({value_text}, {count_text})"),
    })
}
