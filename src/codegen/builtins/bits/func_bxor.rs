//! `bits::bxor` — bitwise exclusive-OR of two 64-bit integers.
//!
//! Descriptor + docs migrated from `src/docs/man/builtins/bits/bxor.md`; lowering
//! from the former `src/target/shared/code/builder_bits.rs::lower_bits_binary`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::{CodeBuilder, Operand, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const INTRO: &str = r#"Bitwise exclusive-OR of two 64-bit integers."#;
const DESC: &str = r#"`bxor` returns the bitwise exclusive-OR of `a` and `b`, computed independently
across all 64 bit positions: bit *i* of the result is `1` when bit *i* differs
between the two operands, and `0` when the two bits are equal.

Both operands and the result are raw two's-complement 64-bit `Integer` bit
patterns; `bxor` does not interpret sign. The operation is total — it is defined
for every pair of inputs and never raises — has no side effects, and lowers to a
single native AArch64 `eor` instruction inline rather than calling a runtime
helper, producing identical results on the native and Binary Representation
execution paths.

XORing a value with itself yields `0`, and XORing with `0` returns the value
unchanged, so `bxor` is its own inverse: `bits::bxor(bits::bxor(x, k), k)`
recovers `x`."#;
const EX: &str = r#"Toggle the low byte of a value by XORing with an all-ones mask:

```
IMPORT bits
IMPORT io

SUB main()
  LET value AS Integer = 0x1234
  LET toggled AS Integer = bits::bxor(value, 255)
  io::print(toString(toggled))
END SUB
```

Swap two integers without a temporary using the XOR identity:

```
IMPORT bits
IMPORT io

SUB main()
  LET x AS Integer = 5
  LET y AS Integer = 9
  LET x2 AS Integer = bits::bxor(x, y)
  LET y2 AS Integer = bits::bxor(x2, y)
  LET x3 AS Integer = bits::bxor(x2, y2)
  io::print(toString(x3))
  io::print(toString(y2))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "bxor",
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
            body: Body::native(None, None, Some(lower_bits_bxor)),
        }],
    });
}

/// Target-generic call-site lowering for `bits::bxor`.
pub(crate) fn lower_bits_bxor(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let (left_reg, right_reg, left_text, right_text) =
        super::gen_two_integers::lower_bits_two_integers(builder, "bxor", args)?;
    let dst = builder.allocate_register()?;
    builder.emit(abi::exclusive_or_registers(dst, left_reg, right_reg));
    Ok(ValueResult {
        type_: "Integer".to_string(),
        location: Operand::from(dst.render()),
        text: format!("bits.bxor({left_text}, {right_text})"),
    })
}
