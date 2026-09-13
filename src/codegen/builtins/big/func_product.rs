//! `big::product` — the product of a list of `big::Int` values.

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

const INTRO: &str = r#"Multiply together every `big::Int` in a list."#;
const DESC: &str = r#"`big::product(values)` returns every element of `values` multiplied together. An
empty list multiplies to one.

The result is exact at every size and the call never raises. It equals multiplying the
elements one at a time with `big::multiply`, in any order, but it is a single call, so a
long list is multiplied without a call per element. A zero anywhere in the list makes the
result plain zero, never a negative zero. The result is in canonical form, and the list
is not changed."#;
const EX: &str = r#"Multiply a list whose product leaves the `Integer` range:

```
IMPORT big
IMPORT io

SUB main()
  LET twoTo32 AS big::Int = big::fromInteger(4294967296)
  LET values AS List OF big::Int = [twoTo32, twoTo32, big::fromInteger(-1)]
  LET result AS big::Int = big::product(values)
  io::print(toString(big::sign(result)) & " " & toString(len(result.magnitude)))
END SUB
```

An empty list multiplies to one:

```
IMPORT big
IMPORT io

SUB main()
  LET none AS List OF big::Int = []
  io::print(toString(big::toInteger(big::product(none))))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "product",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "values",
                desc: "The values to multiply. May be empty, which multiplies to one.",
                aliases: &[],
                ty: ParameterType::list_of(ParameterType::named(INT_TYPE_ID)),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_product),
        }],
    });
}

/// `big::product`: fold the list with `emit_fold_list` in multiply mode.
pub(crate) fn lower_product(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    emit_fold_list(builder, &mut vregs, arg_slots[0], true, "r", &alloc_fail);
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
        text: "big.product".to_string(),
    })
}
