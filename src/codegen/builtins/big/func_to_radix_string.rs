//! `big::toRadixString` — the text of a `big::Int` in a base from 2 to 36.

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

const INTRO: &str = r#"Write a `big::Int` as text in a base from 2 to 36."#;
const DESC: &str = r#"`big::toRadixString(value, radix)` returns `value` written in base `radix`: a `-` for
a negative value, then the digits, most significant first, with no leading zeros. Digits
above 9 are the lowercase letters `a`–`z`, and there is no prefix such as `0x`. Zero is
`"0"`, with no sign.

`radix` must be from 2 to 36; any other radix raises `ErrInvalidArgument`. Radix 10 gives
the same text as `big::toString`, which takes no radix and never raises.

`big::parse(big::toRadixString(a, r), r)` is `a` for every value and every valid radix."#;
const EX: &str = r#"Hexadecimal and binary:

```
IMPORT big
IMPORT io

SUB main()
  io::print(big::toRadixString(big::fromInteger(255), 16))
  io::print(big::toRadixString(big::fromInteger(-5), 2))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toRadixString",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The value to write.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "radix",
                    desc: "The base to write in, 2 to 36. Any other radix raises `ErrInvalidArgument`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec!["ErrInvalidArgument"],
            body: Body::abi_function(lower_to_radix_string),
        }],
    });
}

/// `big::toRadixString`: check the radix, then `emit_int_to_string`.
pub(crate) fn lower_to_radix_string(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let invalid = format!("{symbol}_invalid");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let radix = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&radix, abi::stack_pointer(), arg_slots[1]),
        abi::compare_immediate(&radix, "2"),
        abi::branch_lt(&invalid),
        abi::compare_immediate(&radix, "36"),
        abi::branch_gt(&invalid),
    ]);
    let value = emit_load_int(builder, &mut vregs, arg_slots[0], "v");
    emit_int_to_string(builder, &mut vregs, &value, arg_slots[1], "text", &alloc_fail);
    builder.instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(
        &symbol,
        "ErrInvalidArgument",
        &mut builder.instructions,
        &mut builder.relocations,
        &done,
    );
    builder.instructions.push(abi::label(&alloc_fail));
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
        text: "big.toRadixString".to_string(),
    })
}
