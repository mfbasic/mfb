//! `bits::sl` — logical left shift of a 64-bit integer.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
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
operation has no side effects and costs a single native instruction, so there is
no function call at run time."#;
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

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sl",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
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
            body: Body::abi_inline(lower_bits_sl),
        }],
    });
}

/// Target-generic call-site lowering for `bits::sl`.
pub(crate) fn lower_bits_sl(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args[0].type_ != ParameterType::Integer {
        return Err(format!("bits.sl does not accept {}", args[0].type_));
    }
    if args[1].type_ != ParameterType::Integer {
        return Err(format!("bits.sl does not accept {}", args[1].type_));
    }
    let value_reg = args[0].location.clone();
    let count_reg = args[1].location.clone();
    let value_text = &args[0].text;
    let count_text = &args[1].text;
    let valid = builder.label("bits_shift_valid");
    let out_of_range = builder.label("bits_shift_out_of_range");
    builder.emit(abi::compare_immediate(&count_reg, "0"));
    builder.emit(abi::branch_lt(&out_of_range));
    builder.emit(abi::compare_immediate(&count_reg, "63"));
    builder.emit(abi::branch_le(&valid));
    builder.emit(abi::label(&out_of_range));
    builder.raise_error_bare("ErrInvalidArgument")?;
    builder.emit(abi::label(&valid));
    let dst = builder.allocate_register();
    builder.emit(abi::shift_left_variable(dst, value_reg, &count_reg));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(dst.render()),
        text: format!("bits.sl({value_text}, {count_text})"),
    })
}
