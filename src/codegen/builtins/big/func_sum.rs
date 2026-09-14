//! `big::sum` — the total of a list of `big::Int` values.

use super::gen_big::{emit_fold_list, emit_spill_args};
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

const INTRO: &str = r#"Add up every `big::Int` in a list."#;
const DESC: &str = r#"`big::sum(values)` returns the total of every element of `values`. An empty list
sums to zero.

The result is exact at every size and the call never raises. It equals adding the
elements one at a time with `big::add`, in any order, but it is a single call, so a long
list is added up without a call per element. The result is in canonical form, and the
list is not changed."#;
const EX: &str = r#"Total a list whose sum leaves the `Integer` range:

```
IMPORT big
IMPORT io

SUB main()
  LET top AS big::Int = big::fromInteger(9223372036854775807)
  LET values AS List OF big::Int = [top, top, big::fromInteger(2)]
  LET total AS big::Int = big::sum(values)
  io::print(toString(big::equals(total, big::multiply(big::add(top, big::fromInteger(1)), big::fromInteger(2)))))
END SUB
```

An empty list sums to zero:

```
IMPORT big
IMPORT io

SUB main()
  LET none AS List OF big::Int = []
  io::print(toString(big::isZero(big::sum(none))))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sum",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "values",
                desc: "The values to add. May be empty, which sums to zero.",
                aliases: &[],
                ty: ParameterType::list_of(ParameterType::named(INT_TYPE_ID)),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_sum),
        }],
    });
}

/// `big::sum`: fold the list with `emit_fold_list`.
pub(crate) fn lower_sum(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    emit_fold_list(builder, &mut vregs, arg_slots[0], false, "r", &alloc_fail);
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
        text: "big.sum".to_string(),
    })
}
