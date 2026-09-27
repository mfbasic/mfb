//! `collections::repeat` — descriptor entry + target-generic lowering (plan-153).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::typed_list_element_type;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = "Return a list with a run of positions all holding one value";

const DESC: &str = r#"`collections::repeat` returns a copy of `value` in which the `count` positions
beginning at index `start` all hold `item`. Positions that already exist are
overwritten; positions past the end are added, so the list grows when the run
reaches beyond it. Every other element keeps its value and its position.

For a list of length `n`, the result has length `max(n, start + count)`. That one
rule covers three everyday shapes:

- **build** a list of `n` equal elements from an empty one:
  `collections::repeat([], 0.0, n)`;
- **reset** every element: `collections::repeat(xs, 0, len(xs))`;
- **fill a range** in the middle or at the end:
  `collections::repeat(xs, item, count, start)`.

`start` defaults to `0`. It must name an existing position or the end of the
list: `0 <= start <= len(value)`. `start` equal to `len(value)` is legal and
simply extends the list. A `start` past the end raises `ErrIndexOutOfRange`,
because the positions between the end and `start` would have no value to hold;
so does a negative `start`, which is never an offset from the end. A negative
`count` raises `ErrInvalidArgument`. A `count` of `0` returns the list
unchanged. Both checks happen before anything is written.

`repeat` does not change its argument. The list you pass is unchanged; the
filled list is the returned value. Every position gets its own copy of
`item`, so changing one element of the result later never changes another.
Assigning straight back to the same variable — `xs = collections::repeat(xs,
item, count, start)` — is the cheap shape: it updates the list in place rather
than rebuilding it. Assigning the result anywhere else builds a new list. The
result is the same either way.

`repeat` works on lists only; a map or set has no positions. To repeat text,
use `strings::repeat`. To add a single element, use `collections::append`; to
replace one, `collections::set`. `ErrOutOfMemory` means the program ran out of memory."#;

const EX: &str = r#"Build, reset, and fill part of a list:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  MUT xs AS List OF Integer = collections::repeat([], 7, 4)
  io::print(toString(len(xs)) & " " & toString(collections::get(xs, 3)))
  xs = collections::repeat(xs, 0, len(xs))
  io::print(toString(collections::get(xs, 0)))
  xs = collections::repeat(xs, 5, 3, 2)
  io::print(toString(len(xs)) & " " & toString(collections::get(xs, 4)))
  RETURN 0
END FUNC
```

prints:

```
4 7
0
5 5
```

A `start` past the end is out of range; the end itself is not:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET xs AS List OF String = ["a", "b"]
  LET longer AS List OF String = collections::repeat(xs, "c", 2, len(xs))
  io::print(toString(len(longer)))
  LET gap AS List OF String = collections::repeat(xs, "c", 1, 3) TRAP(e)
    io::print(e.message)
    RECOVER xs
  END TRAP
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "repeat",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF T, T, Integer[, Integer]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The list to fill. Not modified — you get a new list back. May be empty.",
                    aliases: &["collection"],
                    ty: ParameterType::list_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "item",
                    desc: "The value every position in the run holds. Each position gets its own copy.",
                    aliases: &[],
                    ty: ParameterType::var("T"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "count",
                    desc: "How many positions to fill. `0` returns the list unchanged; a negative count raises `ErrInvalidArgument`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "start",
                    desc: "The zero-based index of the first position to fill. Defaults to `0`. Must satisfy `0 <= start <= len(value)`; `len(value)` extends the list, anything else raises `ErrIndexOutOfRange`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::Integer,
                        expr: "0",
                    },
                },
            ],
            return_type: ParameterType::Arg(0),
            errors: vec!["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrOutOfMemory"],
            body: Body::abi_inline(lower_repeat),
        }],
    });
}

/// `collections::repeat` — validate (negative count, then start out of
/// `[0, len]`) before building anything, then take a tight copy of the argument
/// and run the shared in-place core on it (`lower_list_repeat_in_place`), the
/// `lower_set` shape. The copy's payloads are byte copies whose graph edges
/// still belong to the argument, so the core must not drop the elements it
/// overwrites; the abi-inline dispatch gives the result its own graphs
/// (`own_collection_payload_edges`), one per position.
pub(crate) fn lower_repeat(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let collection = args[0].clone();
    let Some(element_type) = typed_list_element_type(&collection.type_).cloned() else {
        return Err(format!(
            "native collection repeat does not accept {}",
            collection.type_
        ));
    };
    let list_slot = builder.allocate_stack_object("repeat_list", 8);
    builder.emit(abi::store_u64(
        &collection.location,
        abi::stack_pointer(),
        list_slot,
    ));
    let item = args[1].clone();
    if item.type_ != element_type {
        return Err(format!(
            "native collection repeat item must be {}, got {}",
            element_type, item.type_
        ));
    }
    // Materialize a `d`-native float before the payload spill (plan-01).
    let item = builder.materialize_value(item)?;
    let item_slot = builder.allocate_stack_object("repeat_item", 8);
    builder.emit(abi::store_u64(
        &item.location,
        abi::stack_pointer(),
        item_slot,
    ));
    let int_slot = |builder: &mut CodeBuilder, arg: &ValueResult, name: &str| {
        if arg.type_ != ParameterType::Integer {
            return Err(format!(
                "native collection repeat {name} must be Integer, got {}",
                arg.type_
            ));
        }
        let slot = builder.allocate_stack_object(&format!("repeat_{name}"), 8);
        builder.emit(abi::store_u64(&arg.location, abi::stack_pointer(), slot));
        Ok(slot)
    };
    let count_slot = int_slot(builder, &args[2], "count")?;
    let start_slot = int_slot(builder, &args[3], "start")?;
    builder.emit_list_repeat_validate(list_slot, count_slot, start_slot)?;
    let source = builder.allocate_register();
    builder.emit(abi::load_u64(&source, abi::stack_pointer(), list_slot));
    let copy = builder.copy_collection_tight(&collection.type_, &source)?;
    let copy_slot = builder.allocate_stack_object("repeat_copy", 8);
    builder.emit(abi::store_u64(&copy, abi::stack_pointer(), copy_slot));
    builder.lower_list_repeat_in_place(
        copy_slot,
        item_slot,
        None,
        count_slot,
        start_slot,
        &collection.type_,
        &element_type,
        false,
        None,
    )
}
