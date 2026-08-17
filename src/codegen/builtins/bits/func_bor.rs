//! `bits::bor` — bitwise OR of two 64-bit integers.
//!
//! Descriptor + docs migrated from `src/docs/man/builtins/bits/bor.md`; lowering
//! from the former `src/target/shared/code/builder_bits.rs::lower_bits_binary`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::{CodeBuilder, Operand, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const INTRO: &str = r#"Bitwise OR of two 64-bit integers."#;
const DESC: &str = r#"`bor` returns the bitwise OR of `a` and `b`, computed independently across all
64 bit positions: bit *i* of the result is `1` when bit *i* is `1` in either
operand (or both), and `0` only when bit *i* is `0` in both operands.

Both operands and the result are raw two's-complement 64-bit `Integer` bit
patterns; `bor` does not interpret sign. The operation is total — it is defined
for every pair of inputs and never raises — has no side effects, and lowers to a
single native AArch64 `orr` instruction inline rather than calling a runtime
helper, producing identical results on the native and Binary Representation
execution paths.

The name is `bor` rather than `or` because `OR` is a reserved logical (Boolean)
keyword and cannot be a package member identifier."#;
const EX: &str = r#"Combine two single-bit flag masks into one value:

```
IMPORT bits
IMPORT io

SUB main()
  LET flags AS Integer = bits::bor(1, 4)
  io::print(toString(flags))
END SUB
```

Force the low two bits of a value on, leaving the rest unchanged:

```
IMPORT bits
IMPORT io

SUB main()
  LET value AS Integer = 0x1234
  LET result AS Integer = bits::bor(value, 3)
  io::print(toString(result))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "bor",
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
            body: Body::native(None, None, Some(lower_bits_bor)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::bor`.
pub(crate) fn lower_bits_bor(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let (left_reg, right_reg, left_text, right_text) =
        super::gen_two_integers::lower_bits_two_integers(builder, "bor", args)?;
    let dst = builder.allocate_register()?;
    builder.emit(abi::or_registers(dst, left_reg, right_reg));
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.bor({left_text}, {right_text})"),
    })
}
