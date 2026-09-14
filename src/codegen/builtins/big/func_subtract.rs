//! `big::subtract` — the difference of two `big::Int` values.

use super::gen_big::{emit_add_int, emit_load_int, emit_spill_args};
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

const INTRO: &str = r#"Subtract one `big::Int` from another."#;
const DESC: &str = r#"`big::subtract(a, b)` returns `a - b`.

The result is exact at every size and the call never raises: there is no overflow to
report, because the result grows or shrinks to hold the difference. It is in canonical
form, so subtracting a value from itself gives plain zero, never a negative zero, and
`big::subtract(a, b)` always equals `big::negate(big::subtract(b, a))`.

Neither operand is changed. `big::add` is the sum."#;
const EX: &str = r#"Go below the `Integer` minimum and come back:

```
IMPORT big
IMPORT io

SUB main()
  LET bottom AS big::Int = big::fromInteger(-9223372036854775807 - 1)
  LET below AS big::Int = big::subtract(bottom, big::fromInteger(1))
  LET back AS big::Int = big::subtract(below, big::fromInteger(-1))
  io::print(toString(big::toInteger(back)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "subtract",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The value subtracted from.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "The value subtracted.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_subtract),
        }],
    });
}

/// `big::subtract`: `emit_add_int` with `b`'s sign flipped.
pub(crate) fn lower_subtract(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let a = emit_load_int(builder, &mut vregs, arg_slots[0], "a");
    let b = emit_load_int(builder, &mut vregs, arg_slots[1], "b");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    emit_add_int(builder, &mut vregs, &a, &b, true, "r", &alloc_fail);
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
        type_: ParameterType::named(INT_TYPE_ID),
        location: Operand::from("void"),
        text: "big.subtract".to_string(),
    })
}
