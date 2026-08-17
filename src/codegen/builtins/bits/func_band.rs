//! `bits::band` — bitwise AND of two 64-bit integers.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::{CodeBuilder, Operand, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const INTRO: &str = r#"Bitwise AND of two 64-bit integers."#;
const DESC: &str = r#"`band` returns the bitwise AND of `a` and `b`, computed independently across all
64 bit positions: bit *i* of the result is `1` only when bit *i* is `1` in both
operands, and `0` otherwise.

Both operands and the result are raw two's-complement 64-bit `Integer` bit
patterns; `band` does not interpret sign. The operation is total — it is defined
for every pair of inputs and never raises — has no side effects, and lowers to a
single native AArch64 `and` instruction inline rather than calling a runtime
helper, producing identical results on the native and Binary Representation
execution paths.

The name is `band` rather than `and` because `AND` is a reserved logical
(Boolean) keyword and cannot be a package member identifier."#;
const EX: &str = r#"Mask off all but the low byte of a value:

```
IMPORT bits
IMPORT io

SUB main()
  LET value AS Integer = 0x1234
  LET low AS Integer = bits::band(value, 255)
  io::print(toString(low))
END SUB
```

Test whether a specific bit is set by ANDing with a single-bit mask:

```
IMPORT bits
IMPORT io

SUB main()
  LET flags AS Integer = 6
  LET bit1Set AS Integer = bits::band(flags, 2)
  io::print(toString(bit1Set))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "band",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The first operand. Any 64-bit value; treated as a raw bit pattern.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "The second operand. Any 64-bit value; treated as a raw bit pattern.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::native(None, None, Some(lower_bits_band)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::band`.
pub(crate) fn lower_bits_band(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let (left_reg, right_reg, left_text, right_text) =
        super::gen_two_integers::lower_bits_two_integers(builder, "band", args)?;
    let dst = builder.allocate_register()?;
    builder.emit(abi::and_registers(dst, left_reg, right_reg));
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.band({left_text}, {right_text})"),
    })
}
