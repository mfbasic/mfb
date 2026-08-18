//! `bits::bswap32` — reverse the byte order of the low 32 bits of an integer.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
const INTRO: &str = r#"Reverse the byte order of the low 32 bits of an integer."#;
const DESC: &str = r#"`bswap32` reverses the order of the four bytes that make up the low 32 bits of
`value`: byte `0` (bits `0`..`7`) and byte `3` (bits `24`..`31`) exchange places,
and byte `1` (bits `8`..`15`) and byte `2` (bits `16`..`23`) exchange places, so a
value laid out as `0xAABBCCDD` becomes `0xDDCCBBAA`. Every bit above bit `31`
(bits `32`..`63`) is cleared to zero in the result, so the output is always a
non-negative 32-bit quantity regardless of the high bits of `value`.

`value` is treated as a raw two's-complement 64-bit `Integer` bit pattern;
`bswap32` does not interpret sign. The operation is total — it is defined for
every `Integer` and never raises; only the variable-shift ops (`sl`/`sr`/`sra`)
can raise a `bits::` error — has no side effects, and lowers to a native word
byte-reversal instruction (`rev Wd, Wn`, which zero-extends into the upper half)
inline rather than calling a runtime helper, producing identical results on the
native and Binary Representation execution paths."#;
const EX: &str = r#"Swap the four low bytes of a 32-bit value:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::bswap32(0x000000FF)
  io::print(toString(result))
END SUB
```

Bits above bit 31 are cleared, so the result stays in `0`..`4294967295`:

```
IMPORT bits
IMPORT io

SUB main()
  io::print(toString(bits::bswap32(0x1122334455667788)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "bswap32",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The value whose low 32 bits are byte-reversed. Bits above bit `31` are ignored and do not appear in the result.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::native(None, None, Some(lower_bits_bswap32)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::bswap32`.
pub(crate) fn lower_bits_bswap32(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let value = super::gen_one_integer::lower_bits_one_integer(builder, "bswap32", &args[0])?;
    let dst = builder.allocate_register()?;
    // `REV` on the `W` register reverses the four bytes and zero-extends, so the
    // result is a non-negative 32-bit quantity regardless of the high bits.
    builder.emit(abi::reverse_bytes_word(dst, &value.location));
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.bswap32({})", value.text),
    })
}
