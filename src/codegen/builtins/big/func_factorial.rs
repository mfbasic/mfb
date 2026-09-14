//! `big::factorial` — `n!` as a `big::Int`.

use super::gen_big::{
    emit_int_from_integer, emit_load_int, emit_mul_int, emit_reject_negative, emit_release_int,
    emit_spill_args,
};
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

const INTRO: &str = r#"Compute `n` factorial as a `big::Int`."#;
const DESC: &str = r#"`big::factorial(n)` returns `n!`, the product `1 * 2 * ... * n`. `0!` and `1!` are both
`1`.

`n` must be zero or more; a negative `n` raises `ErrInvalidArgument`. There is no upper
limit on the result's size — `20!` is the largest factorial an `Integer` holds, and larger
ones simply grow — but the work grows quickly with `n`.

The whole product is computed in one call, rather than a `big::multiply` call per
factor."#;
const EX: &str = r#"A factorial past the `Integer` range:

```
IMPORT big
IMPORT io

SUB main()
  io::print(big::toString(big::factorial(25)))
  io::print(big::toString(big::factorial(0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "factorial",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "n",
                desc: "How many factors to multiply. Zero or more; a negative `n` raises `ErrInvalidArgument`.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec!["ErrInvalidArgument"],
            body: Body::abi_function(lower_factorial),
        }],
    });
}

/// `big::factorial`: multiply an accumulator by `2, 3, ..., n`, releasing each replaced
/// accumulator and each factor as it goes.
pub(crate) fn lower_factorial(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let n_slot = arg_slots[0];
    let one_slot = builder.allocate_stack_object("big_one", 8);
    let counter_slot = builder.allocate_stack_object("big_counter", 8);
    let acc_slot = builder.allocate_stack_object("big_acc", 8);
    let factor_slot = builder.allocate_stack_object("big_factor", 8);
    let next_slot = builder.allocate_stack_object("big_next", 8);
    let invalid = format!("{symbol}_invalid");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let step = format!("{symbol}_step");
    let finished = format!("{symbol}_finished");

    emit_reject_negative(builder, &mut vregs, n_slot, &invalid);
    let one = vregs.next();
    builder.instructions.extend([
        abi::move_immediate(&one, "Integer", "1"),
        abi::store_u64(&one, abi::stack_pointer(), one_slot),
    ]);
    emit_int_from_integer(builder, &mut vregs, one_slot, "one", &alloc_fail);
    let two = vregs.next();
    builder.instructions.extend([
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), acc_slot),
        abi::move_immediate(&two, "Integer", "2"),
        abi::store_u64(&two, abi::stack_pointer(), counter_slot),
        abi::label(&step),
    ]);
    let (counter, n) = (vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&counter, abi::stack_pointer(), counter_slot),
        abi::load_u64(&n, abi::stack_pointer(), n_slot),
        abi::compare_registers(&counter, &n),
        abi::branch_gt(&finished),
    ]);
    emit_int_from_integer(builder, &mut vregs, counter_slot, "factor", &alloc_fail);
    builder.instructions.push(abi::store_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        factor_slot,
    ));
    let acc = emit_load_int(builder, &mut vregs, acc_slot, "acc");
    let factor = emit_load_int(builder, &mut vregs, factor_slot, "factor");
    emit_mul_int(builder, &mut vregs, &acc, &factor, "product", &alloc_fail);
    builder.instructions.push(abi::store_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        next_slot,
    ));
    emit_release_int(builder, &mut vregs, acc_slot);
    emit_release_int(builder, &mut vregs, factor_slot);
    let (moved, bump) = (vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&moved, abi::stack_pointer(), next_slot),
        abi::store_u64(&moved, abi::stack_pointer(), acc_slot),
        abi::load_u64(&bump, abi::stack_pointer(), counter_slot),
        abi::add_immediate(&bump, &bump, 1),
        abi::store_u64(&bump, abi::stack_pointer(), counter_slot),
        abi::branch(&step),
        abi::label(&finished),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), acc_slot),
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
        type_: ParameterType::named(INT_TYPE_ID),
        location: Operand::from("void"),
        text: "big.factorial".to_string(),
    })
}
