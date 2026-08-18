//! `collections::removeAt` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::list_element_type;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
const INTO_REMOVE_AT: &str = "Return a list with the element at a given index removed";
const DESC_REMOVE_AT: &str = r#"`collections::removeAt` returns a new list containing every element of `value`
except the one at `index`, with the elements above `index` shifted down by one to
close the gap and all other relative order preserved. The result is always
exactly one element shorter than `value`. It takes exactly two arguments; neither
is optional and neither is variadic.

`index` is zero-based and is validated as `0 <= index < len(value)`. The upper
bound is **exclusive**: unlike `collections::insert`, `index` equal to the length
is not a valid position — there is nothing there to remove — and raises
`ErrIndexOutOfRange`, as does any negative `index`. Removing from an empty list
therefore always raises, since no index satisfies the range.

`removeAt` is value-semantic. The list named by `value` is unchanged; the
shortened list is the returned value, and a program observes the update only
through what it does with that return value. There is no in-place fast path for
`removeAt` — the compiler's in-place assignment recognizers cover `append`, bulk
`append`, `prepend`, `set`, and string concatenation, not `removeAt`.

`removeAt` is **fallible**: the range check is a real trappable domain error, so
an inline `TRAP` on a `removeAt` call compiles and catches the out-of-range
failure rather than being reported as a dead handler. The bounds test runs before
the result block is allocated, so a rejected index allocates nothing.

`removeAt` operates on lists only. To drop a key from a `Map OF K TO V`, use
`collections::removeKey`, which takes a key rather than an index and does not
raise when the key is absent."#;

const EX: &str = r#"Remove the second element:

```
IMPORT collections

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::removeAt([1, 2, 3], 1)
  RETURN 0
END FUNC
```

Remove the last element:

```
IMPORT collections

FUNC main AS Integer
  LET source AS List OF String = ["a", "b", "c"]
  LET shorter AS List OF String = collections::removeAt(source, len(source) - 1)
  RETURN 0
END FUNC
```

Catch an out-of-range index with an inline `TRAP`:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2]
  LET shorter AS List OF Integer = collections::removeAt(numbers, 2) TRAP(e)
    io::print(e.message)
    RECOVER numbers
  END TRAP
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "removeAt",
        intro: INTO_REMOVE_AT,
        desc: DESC_REMOVE_AT,
        example: EX,
        expected_arguments: Some("List OF T, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "",
                    aliases: &["list"],
                    ty: ParameterType::list_of(ParameterType::Var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "index",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Arg(0),
            errors: vec!["ErrIndexOutOfRange"],
            body: Body::native(None, None, Some(lower_remove_at)),
        }],
    });
}

/// `collections::removeAt(List OF T, Integer) AS List OF T`: drop the element at
/// `index` (range-checked -> `ErrIndexOutOfRange`).
pub(crate) fn lower_remove_at(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let list = builder.lower_value(&args[0])?;
    let Some(element_type) = list_element_type(&list.type_) else {
        return Err(format!(
            "native collection removeAt does not accept {}",
            list.type_
        ));
    };
    let list_slot = builder.allocate_stack_object("remove_at_list", 8);
    builder.emit(abi::store_u64(
        &list.location,
        abi::stack_pointer(),
        list_slot,
    ));
    let index = builder.lower_value(&args[1])?;
    if index.type_ != "Integer" {
        return Err(format!(
            "native collection removeAt index must be Integer, got {}",
            index.type_
        ));
    }
    let index_slot = builder.allocate_stack_object("remove_at_index", 8);
    builder.emit(abi::store_u64(
        &index.location,
        abi::stack_pointer(),
        index_slot,
    ));
    builder.lower_list_remove_at(list_slot, index_slot, &list.type_, &element_type)
}
