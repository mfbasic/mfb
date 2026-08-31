//! `bits::rr32` — rotate the low 32 bits of an integer right.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
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
patterns; `rr32` does not interpret sign. `count` is reduced modulo 32, so every
count is defined and `rr32` never raises — rotating by 32 returns the value
unchanged, and rotating by 33 is the same as rotating by 1. The operation has no
side effects and costs a single native instruction, so there is no function call
at run time."#;
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

pub(crate) fn register(pkg: &mut RegistryPackage) {
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
            body: Body::abi_inline(lower_bits_rr32),
        }],
    });
}

/// Target-generic call-site lowering for `bits::rr32`.
pub(crate) fn lower_bits_rr32(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args[0].type_ != ParameterType::Integer {
        return Err(format!("bits.rr32 does not accept {}", args[0].type_));
    }
    if args[1].type_ != ParameterType::Integer {
        return Err(format!("bits.rr32 does not accept {}", args[1].type_));
    }
    let value_reg = args[0].location.clone();
    let count_reg = args[1].location.clone();
    let value_text = &args[0].text;
    let count_text = &args[1].text;
    let dst = builder.allocate_register();
    builder.emit(abi::rotate_right_word_registers(dst, value_reg, count_reg));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(dst.render()),
        text: format!("bits.rr32({value_text}, {count_text})"),
    })
}
