//! `bits::sr` — logical (zero-filling) right shift of a 64-bit integer.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
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

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sr",
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
            body: Body::abi_inline(lower_bits_sr),
        }],
    });
}

/// Target-generic call-site lowering for `bits::sr`.
pub(crate) fn lower_bits_sr(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args[0].type_ != ParameterType::Integer {
        return Err(format!("bits.sr does not accept {}", args[0].type_));
    }
    if args[1].type_ != ParameterType::Integer {
        return Err(format!("bits.sr does not accept {}", args[1].type_));
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
    let dst = builder.allocate_register()?;
    builder.emit(abi::shift_right_variable(dst, value_reg, &count_reg));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(dst.render()),
        text: format!("bits.sr({value_text}, {count_text})"),
    })
}
