//! `collections::sum` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::list_element_type;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTO_SUM: &str = "Add up the elements of an Integer, Float, or Fixed list";
const DESC_SUM: &str = r#"`collections::sum` walks `value` from the first element to the last and adds
each element into a running total, returning that total. It is a **native**
member: the compiler emits the accumulation loop directly rather than
instantiating an MFBASIC generic.

There are exactly **three** overloads — `List OF Integer`, `List OF Float`, and
`List OF Fixed` — and the return type always matches the element type. There is
no `List OF Byte`, no `List OF Money`, and no general "any numeric list" form:
any other element type fails to resolve at compile time, and the lowering
rejects it a second time.

The accumulator is initialized to zero of the element type and the elements are
added in list order, so an empty `value` yields `0`, `0.0`, or `0.0F`
respectively without any addition being performed.

`value` is neither modified nor consumed. `sum` takes no callback and has no
optional argument; it is a single-argument member.

For the `Integer` and `Fixed` overloads each step is a **checked** 64-bit
addition: if the running total leaves the destination range, the addition fails
with `ErrOverflow` rather than wrapping. `Fixed` shares the `Integer` path
because it is a scaled 64-bit integer. The `Float` overload uses IEEE-754
double addition and never raises — an out-of-range total becomes `±Inf` in the
usual floating-point way.

Note a wrinkle worth knowing before writing a handler: the compiler's inline-
built-in fallibility census classifies `sum` as **infallible**, so attaching an
inline `TRAP` to a `sum` call raises the `TYPE_INLINE_TRAP_DEAD_HANDLER`
diagnostic and that handler does not receive the overflow. The overflow is still
raised at run time and still propagates out of the enclosing function, where an
ordinary function-level `TRAP` can handle it.

To total a list of some other element type, or to accumulate with different
rules, fold it with `collections::reduce`."#;

const EX: &str = r#"Total a list of integers:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET total AS Integer = collections::sum([1, 2, 3])
  io::print(toString(total))
  RETURN 0
END FUNC
```

Total `Float` and `Fixed` lists:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET floats AS List OF Float = [1.25, 2.5]
  LET fixeds AS List OF Fixed = [1.25F, 2.5F]
  io::print(toString(collections::sum(floats)))
  io::print(toString(collections::sum(fixeds)))
  RETURN 0
END FUNC
```

An empty list totals to zero:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET empty AS List OF Integer = []
  io::print(toString(collections::sum(empty)))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sum",
        intro: INTO_SUM,
        desc: DESC_SUM,
        example: EX,
        expected_arguments: Some("List OF Integer, List OF Float, or List OF Fixed"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![Parameter {
                    name: "value",
                    desc: "",
                    aliases: &["collection"],
                    ty: ParameterType::list_of(ParameterType::Integer),
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Integer,
                errors: vec!["ErrOverflow"],
                body: Body::abi_inline(lower_sum),
            },
            Implementation {
                params: vec![Parameter {
                    name: "value",
                    desc: "",
                    aliases: &["collection"],
                    ty: ParameterType::list_of(ParameterType::Float),
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Float,
                errors: vec!["ErrOverflow"],
                body: Body::abi_inline(lower_sum),
            },
            Implementation {
                params: vec![Parameter {
                    name: "value",
                    desc: "",
                    aliases: &["collection"],
                    ty: ParameterType::list_of(ParameterType::Fixed),
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Fixed,
                errors: vec!["ErrOverflow"],
                body: Body::abi_inline(lower_sum),
            },
        ],
    });
}

/// `collections::sum(List OF Integer|Float|Fixed)` — the accumulation loop; the
/// Integer/Fixed paths use checked addition (`ErrOverflow`), Float uses IEEE-754.
pub(crate) fn lower_sum(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let scratch8 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch14 = builder.temporary_vreg();
    let scratch15 = builder.temporary_vreg();
    let scratch16 = builder.temporary_vreg();
    let collection = &args[0];
    let Some(element_type) = list_element_type(&collection.type_.name()) else {
        return Err(format!(
            "native collection sum does not accept {}",
            collection.type_
        ));
    };
    if !matches!(element_type.as_str(), "Integer" | "Float" | "Fixed") {
        return Err(format!(
            "native collection sum does not accept {}",
            collection.type_
        ));
    }
    let collection_slot = builder.allocate_stack_object("sum_collection", 8);
    builder.emit(abi::store_u64(
        &collection.location,
        abi::stack_pointer(),
        collection_slot,
    ));
    let loop_label = builder.label("sum_loop");
    let done = builder.label("sum_done");
    builder.emit(abi::load_u64(
        &scratch8,
        abi::stack_pointer(),
        collection_slot,
    ));
    builder.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::move_immediate(&scratch10, "Integer", "0"));
    builder.emit(abi::add_immediate(
        &scratch11,
        &scratch8,
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit(abi::move_immediate(&scratch14, &element_type, "0"));
    builder.emit(abi::label(&loop_label));
    builder.emit(abi::compare_registers(&scratch10, &scratch9));
    builder.emit(abi::branch_ge(&done));
    // kind 2: the cursor (scratch11) already walks the data region, so it IS
    // the payload address — there is no entry to indirect through.
    if kind2_payload_size(&element_type).is_some() {
        builder.emit(abi::move_register(&scratch15, &scratch11));
    } else {
        builder.emit(abi::load_u64(
            &scratch12,
            &scratch11,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        builder.emit_collection_data_pointer_for(&scratch15, &scratch8, &element_type);
        builder.emit(abi::add_registers(&scratch15, &scratch15, &scratch12));
    }
    match element_type.as_str() {
        "Integer" => {
            builder.emit(abi::load_u64(&scratch16, &scratch15, 0));
            builder.emit_checked_integer_add(&scratch14, &scratch14, &scratch16)?;
        }
        "Float" => {
            builder.emit(abi::load_u64(&scratch16, &scratch15, 0));
            builder.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &scratch14));
            builder.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], &scratch16));
            builder.emit(abi::float_add_d(
                abi::FP_SCRATCH[0],
                abi::FP_SCRATCH[0],
                abi::FP_SCRATCH[1],
            ));
            builder.emit(abi::float_move_x_from_d(&scratch14, abi::FP_SCRATCH[0]));
        }
        "Fixed" => {
            builder.emit(abi::load_u64(&scratch16, &scratch15, 0));
            builder.emit_checked_integer_add(&scratch14, &scratch14, &scratch16)?;
        }
        _ => unreachable!(),
    }
    builder.emit(abi::add_immediate(
        &scratch11,
        &scratch11,
        kind2_payload_size(&element_type).unwrap_or(COLLECTION_ENTRY_SIZE),
    ));
    builder.emit(abi::add_immediate(&scratch10, &scratch10, 1));
    builder.emit(abi::branch(&loop_label));
    builder.emit(abi::label(&done));
    let result = builder.allocate_register()?;
    builder.emit(abi::move_register(&result, &scratch14));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::parse(&element_type),
        location: Operand::from(result.render()),
        text: format!("sum({})", collection.type_),
    })
}
