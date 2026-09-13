//! `big::abs` — the absolute value of a `big::Int`.

use super::gen_big::{emit_copy_int, emit_load_int, emit_spill_args};
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

const INTRO: &str = r#"Return the absolute value of a `big::Int`."#;
const DESC: &str = r#"`big::abs(a)` returns `a` with its sign removed: the same `magnitude`, never
negative. A value that is already zero or positive comes back unchanged.

Unlike `math::abs` on an `Integer`, this never overflows: the absolute value of any
`big::Int` is itself a `big::Int`. The result is in canonical form, and `a` itself is
not changed. The call never raises."#;
const EX: &str = r#"Drop the sign of a negative value:

```
IMPORT big
IMPORT io

SUB main()
  LET a AS big::Int = big::abs(big::fromInteger(-12))
  io::print(toString(big::toInteger(a)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "abs",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "a",
                desc: "The value whose absolute value is returned.",
                aliases: &[],
                ty: ParameterType::named(INT_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_abs),
        }],
    });
}

/// `big::abs`: a copy of the magnitude with a cleared sign.
pub(crate) fn lower_abs(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let a = emit_load_int(builder, &mut vregs, arg_slots[0], "a");
    let negative = builder.allocate_stack_object("big_negative", 8);
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), negative));
    emit_copy_int(builder, &mut vregs, &a, negative, "r", &alloc_fail);
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
        text: "big.abs".to_string(),
    })
}
