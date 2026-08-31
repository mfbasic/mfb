//! `bits::bnot` — bitwise NOT (one's complement) of a 64-bit integer.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTRO: &str = r#"Bitwise NOT (one's complement) of a 64-bit integer."#;
const DESC: &str = r#"`bnot` returns the one's complement of `a`: every one of the 64 bit positions is
inverted, so bit *i* of the result is `1` exactly when bit *i* of `a` is `0`, and
`0` otherwise. As a two's-complement arithmetic identity this equals `-(a) - 1`.

The operand and the result are raw two's-complement 64-bit `Integer` bit
patterns; `bnot` does not interpret sign. The operation is total — it is defined
for every input and never raises — has no side effects, and costs a single
native instruction, so there is no function call at run time.

The name is `bnot` rather than `not` because `NOT` is a reserved logical
(Boolean) keyword and cannot be a package member identifier."#;
const EX: &str = r#"Invert every bit of a value:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::bnot(255)
  io::print(toString(result))
END SUB
```

Clear the low byte by ANDing with an inverted mask:

```
IMPORT bits
IMPORT io

SUB main()
  LET value AS Integer = 0x1234
  LET highOnly AS Integer = bits::band(value, bits::bnot(255))
  io::print(toString(highOnly))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "bnot",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "a",
                desc: "The operand to invert. Any 64-bit value; treated as a raw bit pattern.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_inline(lower_bits_bnot),
        }],
    });
}

/// Target-generic call-site lowering for `bits::bnot` (one's complement).
///
/// `bnot` is the sole consumer of this emit sequence, so its body lives here; it
/// still borrows the shared `gen_one_integer::lower_bits_one_integer` operand
/// check.
pub(crate) fn lower_bits_bnot(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let value = &args[0];
    if value.type_ != ParameterType::Integer {
        return Err(format!("bits.bnot does not accept {}", value.type_));
    }
    let dst = builder.allocate_register();
    builder.emit(abi::bitwise_not(dst, &value.location));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(dst.render()),
        text: format!("bits.bnot({})", value.text),
    })
}
