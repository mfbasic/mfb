//! `big::toString` — the decimal text of a `big::Int`.

use super::gen_big::{emit_int_to_string, emit_load_int, emit_spill_args};
use super::INT_TYPE_ID;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::emit_fail;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Write a `big::Int` as decimal text."#;
const DESC: &str = r#"`big::toString(value)` returns `value` written in base 10: a `-` for a negative value,
then the digits, most significant first, with no leading zeros. Zero is `"0"`, with no
sign.

The call never raises and has no size limit. It is the exact inverse of `big::parse`:
`big::parse(big::toString(a))` is `a` for every value, and the text contains nothing
`big::parse` would reject.

Use `big::toRadixString` for another base. Because a `big::Int` cannot be a `Map` key,
this text is also the way to key a map by one."#;
const EX: &str = r#"Write a value past the `Integer` range:

```
IMPORT big
IMPORT io

SUB main()
  LET top AS big::Int = big::fromInteger(9223372036854775807)
  io::print(big::toString(big::multiply(top, top)))
  io::print(big::toString(big::fromInteger(-42)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toString",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The value to write.",
                aliases: &[],
                ty: ParameterType::named(INT_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_to_string),
        }],
    });
}

/// `big::toString`: `emit_int_to_string` with radix 10.
pub(crate) fn lower_to_string(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let radix_slot = builder.allocate_stack_object("big_radix", 8);
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let ten = vregs.next();
    builder.instructions.extend([
        abi::move_immediate(&ten, "Integer", "10"),
        abi::store_u64(&ten, abi::stack_pointer(), radix_slot),
    ]);
    let value = emit_load_int(builder, &mut vregs, arg_slots[0], "v");
    emit_int_to_string(builder, &mut vregs, &value, radix_slot, "text", &alloc_fail);
    builder.instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_fail),
    ]);
    emit_fail(
        &symbol,
        "ErrOutOfMemory",
        &mut builder.instructions,
        &mut builder.relocations,
        &done,
    );
    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::String,
        location: Operand::from("void"),
        text: "big.toString".to_string(),
    })
}
