//! `bits::rl64` — rotate all 64 bits of an integer left.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
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
patterns; `rl64` does not interpret sign. The operation has no side effects."#;
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

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "rl64",
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
                    desc: "The number of bit positions to rotate left, reduced modulo 64. Any value, including negative counts, is accepted.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_inline(lower_bits_rl64),
        }],
    });
}

/// Target-generic call-site lowering for `bits::rl64`.
pub(crate) fn lower_bits_rl64(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args[0].type_ != ParameterType::Integer {
        return Err(format!("bits.rl64 does not accept {}", args[0].type_));
    }
    if args[1].type_ != ParameterType::Integer {
        return Err(format!("bits.rl64 does not accept {}", args[1].type_));
    }
    let value_reg = args[0].location.clone();
    let count_reg = args[1].location.clone();
    let value_text = &args[0].text;
    let count_text = &args[1].text;
    let dst = builder.allocate_register();
    // AArch64 has only rotate-right (`RORV`), so rotate-left by `count` is a
    // rotate-right by `-count` (the hardware reduces the amount modulo the width).
    let neg = builder.allocate_register();
    builder.emit(abi::subtract_registers(neg, abi::ZERO, count_reg));
    builder.emit(abi::rotate_right_registers(dst, value_reg, neg));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(dst.render()),
        text: format!("bits.rl64({value_text}, {count_text})"),
    })
}
