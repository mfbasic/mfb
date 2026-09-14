//! `big::bitLength` — how many bits the magnitude of a `big::Int` needs.

use super::gen_big::{emit_load_int, emit_spill_args};
use super::INT_TYPE_ID;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Count the significant bits in a `big::Int`'s absolute value."#;
const DESC: &str = r#"`big::bitLength(a)` returns the number of bits needed to write the absolute value of
`a` in binary, without leading zeros: `0` for zero, `1` for one, `8` for 255 and `9` for
256.

The sign does not count, so `a` and `big::negate(a)` have the same bit length. This is
the figure that states a key or modulus size. The call never raises."#;
const EX: &str = r#"Bit lengths around a byte boundary:

```
IMPORT big
IMPORT io

SUB main()
  io::print(toString(big::bitLength(big::fromInteger(0))))
  io::print(toString(big::bitLength(big::fromInteger(255))))
  io::print(toString(big::bitLength(big::fromInteger(-256))))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "bitLength",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "a",
                desc: "The value to measure. Its sign is ignored.",
                aliases: &[],
                ty: ParameterType::named(INT_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(lower_bit_length),
        }],
    });
}

/// `big::bitLength`: `8 * (count - 1)` plus the significant bits of the top byte.
pub(crate) fn lower_bit_length(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let a = emit_load_int(builder, &mut vregs, arg_slots[0], "a");
    let count = vregs.next();
    let top = vregs.next();
    let zeros = vregs.next();
    let answer = vregs.next();
    let finished = format!("{symbol}_finished");
    builder.instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), a.count),
        abi::move_immediate(&answer, "Integer", "0"),
        abi::compare_immediate(&count, "0"),
        abi::branch_eq(&finished),
        abi::load_u64(&top, abi::stack_pointer(), a.data),
        abi::add_registers(&top, &top, &count),
        abi::subtract_immediate(&top, &top, 1),
        abi::load_u8(&top, &top, 0),
        // The top byte is non-zero after trimming, so its leading-zero count on the full
        // word is 56..63 and `64 - zeros` is 1..8.
        abi::count_leading_zeros(&zeros, &top),
        abi::shift_left_immediate(&answer, &count, 3),
        abi::add_immediate(&answer, &answer, 56),
        abi::subtract_registers(&answer, &answer, &zeros),
        abi::label(&finished),
        abi::move_register(RESULT_VALUE_REGISTER, &answer),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from("void"),
        text: "big.bitLength".to_string(),
    })
}
