//! `bits::bswap16` — reverse the byte order of the low 16 bits of an integer.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::{CodeBuilder, Operand, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const INTRO: &str = r#"Reverse the byte order of the low 16 bits of an integer."#;
const DESC: &str = r#"`bswap16` swaps the two bytes that make up the low 16 bits of `value`: byte `0`
(bits `0`..`7`) and byte `1` (bits `8`..`15`) exchange places, so a value laid
out as `0xHHLL` becomes `0xLLHH`. Every bit above bit `15` (bits `16`..`63`) is
cleared to zero in the result, so the output is always a non-negative 16-bit
quantity regardless of the high bits of `value`.

`value` is treated as a raw two's-complement 64-bit `Integer` bit pattern;
`bswap16` does not interpret sign. The operation is total — it is defined for
every `Integer` and never raises — has no side effects, and lowers to native
byte-reversal instructions inline rather than calling a runtime helper."#;
const EX: &str = r#"Swap the two low bytes of a 16-bit value:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::bswap16(0x00FF)
  io::print(toString(result))
END SUB
```

Bits above bit 15 are cleared, so the result stays in `0`..`65535`:

```
IMPORT bits
IMPORT io

SUB main()
  io::print(toString(bits::bswap16(0x11223344)))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "bswap16",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The value whose low 16 bits are byte-reversed. Bits above bit `15` are ignored and do not appear in the result.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::native(None, None, Some(lower_bits_bswap16)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::bswap16`.
pub(crate) fn lower_bits_bswap16(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let value = super::gen_one_integer::lower_bits_one_integer(builder, "bswap16", &args[0])?;
    let dst = builder.allocate_register()?;
    // REV of the low word puts the two low bytes at bits [31:16]; a logical
    // >>16 drops the other two bytes and clears bits 16..63.
    builder.emit(abi::reverse_bytes_word(dst, &value.location));
    builder.emit(abi::shift_right_immediate(dst, dst, 16));
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.bswap16({})", value.text),
    })
}
