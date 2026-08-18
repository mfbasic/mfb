//! `bits::bswap64` — reverse the byte order of all 64 bits of an integer.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
const INTRO: &str = r#"Reverse the byte order of all 64 bits of an integer."#;
const DESC: &str = r#"`bswap64` reverses the order of the eight bytes that make up the full 64 bits of
`value`: byte `0` (bits `0`..`7`) and byte `7` (bits `56`..`63`) exchange places,
byte `1` (bits `8`..`15`) and byte `6` (bits `48`..`55`) exchange places, byte `2`
(bits `16`..`23`) and byte `5` (bits `40`..`47`) exchange places, and byte `3`
(bits `24`..`31`) and byte `4` (bits `32`..`39`) exchange places, so a value laid
out as `0x1122334455667788` becomes `0x8877665544332211`. This converts the value
between little-endian and big-endian byte order. Unlike `bswap16` and `bswap32`,
every one of the 64 bits participates in the swap, so no bits are cleared.

`value` is treated as a raw two's-complement 64-bit `Integer` bit pattern;
`bswap64` does not interpret sign. The operation is total — it is defined for
every `Integer` and never raises; only the variable-shift ops (`sl`/`sr`/`sra`)
can raise a `bits::` error — has no side effects, and lowers to a native
doubleword byte-reversal instruction (`rev Xd, Xn`) inline rather than calling a
runtime helper, producing identical results on the native and Binary
Representation execution paths."#;
const EX: &str = r#"Swap the eight bytes of a 64-bit value:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::bswap64(255)
  io::print(toString(result))
END SUB
```

Byte order flips between little-endian and big-endian:

```
IMPORT bits
IMPORT io

SUB main()
  io::print(toString(bits::bswap64(0x1122334455667788)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "bswap64",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The value whose eight bytes are reversed. All 64 bits participate in the swap.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::native(None, None, Some(lower_bits_bswap64)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::bswap64`.
pub(crate) fn lower_bits_bswap64(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let value = super::gen_one_integer::lower_bits_one_integer(builder, "bswap64", &args[0])?;
    let dst = builder.allocate_register()?;
    builder.emit(abi::reverse_bytes(dst, &value.location));
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.bswap64({})", value.text),
    })
}
