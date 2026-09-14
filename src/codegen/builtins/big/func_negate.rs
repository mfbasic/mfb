//! `big::negate` — a `big::Int` with its sign reversed.

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

const INTRO: &str = r#"Return a `big::Int` with the opposite sign."#;
const DESC: &str = r#"`big::negate(a)` returns `-a`: the same `magnitude` with the sign reversed.

Negating zero returns zero, never a negative zero, so `big::negate(big::negate(a))`
equals `a` for every value. Unlike unary `-` on an `Integer`, this never overflows.
The result is in canonical form, and `a` itself is not changed. The call never
raises."#;
const EX: &str = r#"Reverse a sign twice:

```
IMPORT big
IMPORT io

SUB main()
  LET a AS big::Int = big::fromInteger(40)
  io::print(toString(big::toInteger(big::negate(a))))
  io::print(toString(big::toInteger(big::negate(big::negate(a)))))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "negate",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "a",
                desc: "The value to negate.",
                aliases: &[],
                ty: ParameterType::named(INT_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_negate),
        }],
    });
}

/// `big::negate`: a copy of the magnitude with the sign flag flipped;
/// `emit_build_int` clears it again when the value is zero.
pub(crate) fn lower_negate(
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
    let flag = vregs.next();
    let one = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&flag, abi::stack_pointer(), a.negative),
        abi::move_immediate(&one, "Integer", "1"),
        abi::exclusive_or_registers(&flag, &flag, &one),
        abi::store_u64(&flag, abi::stack_pointer(), negative),
    ]);
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
        text: "big.negate".to_string(),
    })
}
