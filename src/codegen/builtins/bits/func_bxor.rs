//! `bits::bxor` — bitwise exclusive-OR of two 64-bit integers.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTRO: &str = r#"Bitwise exclusive-OR of two 64-bit integers."#;
const DESC: &str = r#"`bxor` returns the bitwise exclusive-OR of `a` and `b`, computed independently
across all 64 bit positions: bit *i* of the result is `1` when bit *i* differs
between the two operands, and `0` when the two bits are equal.

Both operands and the result are raw two's-complement 64-bit `Integer` bit
patterns; `bxor` does not interpret sign. The operation is total — it is defined
for every pair of inputs and never raises — has no side effects, and costs a
single native instruction, so there is no function call at run time.

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

pub(crate) fn register(pkg: &mut RegistryPackage) {
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
            body: Body::abi_inline(lower_bits_bxor),
        }],
    });
}

/// Target-generic call-site lowering for `bits::bxor`.
pub(crate) fn lower_bits_bxor(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args[0].type_ != ParameterType::Integer {
        return Err(format!("bits.bxor does not accept {}", args[0].type_));
    }
    if args[1].type_ != ParameterType::Integer {
        return Err(format!("bits.bxor does not accept {}", args[1].type_));
    }
    let left_reg = args[0].location.clone();
    let right_reg = args[1].location.clone();
    let left_text = &args[0].text;
    let right_text = &args[1].text;
    let dst = builder.allocate_register();
    builder.emit(abi::exclusive_or_registers(dst, left_reg, right_reg));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(dst.render()),
        text: format!("bits.bxor({left_text}, {right_text})"),
    })
}
