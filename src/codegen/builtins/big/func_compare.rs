//! `big::compare` — order two `big::Int` values: `-1`, `0` or `1`.

use super::gen_big::{emit_compare_int, emit_load_int, emit_spill_args};
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

const INTRO: &str = r#"Order two `big::Int` values, returning `-1`, `0` or `1`."#;
const DESC: &str = r#"`big::compare(a, b)` returns `-1` when `a` is less than `b`, `0` when they are
equal, and `1` when `a` is greater. It is the ordering `<` and `>` would give if they
applied to a `big::Int`, which they do not.

The order is the ordinary numeric one over every value, negative, zero and positive.
Two values that spell the same number compare equal even when one was built by hand
with trailing zero bytes in its `magnitude`, or with `negative` set on zero.

Use `big::equals` when only equality matters. The call never raises."#;
const EX: &str = r#"Order a negative and a positive value:

```
IMPORT big
IMPORT io

SUB main()
  LET a AS big::Int = big::fromInteger(-5)
  LET b AS big::Int = big::fromInteger(3)
  io::print(toString(big::compare(a, b)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "compare",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The left value.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "The right value.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(lower_compare),
        }],
    });
}

/// `big::compare`: load both operands and order them with `emit_compare_int`.
pub(crate) fn lower_compare(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let a = emit_load_int(builder, &mut vregs, arg_slots[0], "a");
    let b = emit_load_int(builder, &mut vregs, arg_slots[1], "b");
    let order = vregs.next();
    emit_compare_int(builder, &mut vregs, &a, &b, &order, "cmp");
    builder.instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, &order),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from("void"),
        text: "big.compare".to_string(),
    })
}
