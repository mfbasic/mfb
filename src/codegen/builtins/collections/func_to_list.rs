//! `collections::toList` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::set_element_type;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;
const INTO_TO_LIST: &str = "Return the elements of a set as a list, in insertion order";
const DESC_TO_LIST: &str = r#"`collections::toList` returns a new `List OF T` holding every element of the set
`value` exactly once, in the set's stable insertion order. It takes exactly one
argument, which is neither optional nor variadic.

The set is neither copied for the caller nor mutated: the result is a freshly
built list. Because a set already holds each element at most once, the resulting
list has no duplicates and its length equals `len(value)`. An empty set yields an
empty list.

`toList` is **infallible**: no path in its lowering raises a trappable domain
error, so an inline `TRAP` written on a `toList` call has a dead handler."#;

const EX: &str = r#"List the elements of a set:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET elems AS List OF Integer = collections::toList(Set OF Integer { 3, 1, 2 })
  io::print(toString(len(elems)))
  RETURN 0
END FUNC
```

Duplicate elements never appear in the listed result:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  MUT s AS Set OF Integer = Set OF Integer { 1, 2 }
  s = collections::add(s, 2)
  io::print(toString(len(collections::toList(s))))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toList",
        intro: INTO_TO_LIST,
        desc: DESC_TO_LIST,
        example: EX,
        expected_arguments: Some("Set OF T"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "",
                aliases: &["set"],
                ty: ParameterType::set_of(ParameterType::var("T")),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::var("T")),
            errors: vec![],
            body: Body::abi_inline(lower_to_list),
        }],
    });
}

/// `collections::toList(Set OF T) AS List OF T` (plan-63-B): the elements in
/// stable insertion order. Reuses the Map key projection (the Set's elements
/// are its keys).
pub(crate) fn lower_to_list(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let set = &args[0];
    let Some(element_type) = set_element_type(&set.type_.name()) else {
        return Err(format!(
            "native collection toList does not accept {}",
            set.type_
        ));
    };
    builder.lower_map_projection(set, &element_type, true)
}
