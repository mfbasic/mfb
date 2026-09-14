//! `big::add` — the sum of two `big::Int` values.

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

const INTRO: &str = r#"Add two `big::Int` values."#;
const DESC: &str = r#"`big::add(a, b)` returns `a + b`.

The result is exact at every size and the call never raises: there is no overflow to
report, because the result grows to hold the sum. It is in canonical form, so adding a
value to its own negation gives plain zero, never a negative zero.

Neither operand is changed. `big::subtract` is the difference; `big::sum` adds a whole
list in one call."#;
const EX: &str = r#"Add past the `Integer` range and come back:

```
IMPORT big
IMPORT io

SUB main()
  LET top AS big::Int = big::fromInteger(9223372036854775807)
  LET past AS big::Int = big::add(top, big::fromInteger(1))
  LET back AS big::Int = big::add(past, big::fromInteger(-1))
  io::print(toString(big::toInteger(back)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "add",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The first addend.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "The second addend.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_add),
        }],
    });
}

/// `big::add`: load both operands and dispatch on sign with `emit_add_int`.
pub(crate) fn lower_add(
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
    emit_add_int(builder, &mut vregs, &a, &b, false, "r", &alloc_fail);
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
        text: "big.add".to_string(),
    })
}
