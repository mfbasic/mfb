//! `collections::values` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::typed_map_type_parts;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;
const INTO_VALUES: &str = "Return a map's values as a list.";
const DESC_VALUES: &str = r#"`collections::values` builds a new `List OF V` holding the value of every entry
in `value`. It walks the map's lookup-entry table front to back, copying each
entry's value payload into a freshly allocated list block. The source map is not
mutated and its own storage is not aliased by the result — the returned list is
an independent, owned collection.

The result has exactly one item per map entry, so its length equals
`len(value)`. An empty map yields an empty list. Unlike the key projection, the
result may contain duplicates, because distinct keys may store equal values.

**Ordering.** The projection walks the lookup-entry array directly, and that
array is maintained in insertion order; the hash bucket index is separate
derived metadata that does not reorder it. `collections::values` and
`collections::keys` are the same traversal over the same entries and differ only
in which payload field of each entry they copy, so the two results are
index-aligned: item `i` of `collections::values(m)` is the value of the entry
whose key is item `i` of `collections::keys(m)`. The language specification
describes map iteration order as implementation-defined but stable for a given
unchanged map, so treat insertion order as the current implementation's behavior
rather than a guarantee to rely on across versions.

`collections::values` raises no trappable domain error, so an inline `TRAP` on a
`values` call has a dead handler. Building the result list does allocate, and an
allocation failure is not a trappable domain error in this language."#;

const EX: &str = r#"Get the values of a map:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  LET numbers AS List OF Integer = collections::values(ages)
  io::print(toString(len(numbers)))
  RETURN 0
END FUNC
```

Sum a map's values:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36, "Grace" := 85 }
  io::print(toString(collections::sum(collections::values(ages))))
  RETURN 0
END FUNC
```

Iterate the values directly:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36, "Grace" := 85 }
  FOR EACH age IN collections::values(ages)
    io::print(toString(age))
  NEXT
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "values",
        intro: INTO_VALUES,
        desc: DESC_VALUES,
        example: EX,
        expected_arguments: Some("Map OF K TO V"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "",
                aliases: &["map"],
                ty: ParameterType::map_of(ParameterType::var("K"), ParameterType::var("V")),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::var("V")),
            errors: vec![],
            body: Body::abi_inline(lower_values),
        }],
    });
}

/// `collections::values(Map OF K TO V) AS List OF V`: project each entry's value.
pub(crate) fn lower_values(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let collection = &args[0];
    let Some((_, value_type)) = typed_map_type_parts(&collection.type_)
        .map(|(key, value)| (key.name().into_owned(), value.name().into_owned()))
    else {
        return Err(format!(
            "native collection values does not accept {}",
            collection.type_
        ));
    };
    builder.lower_map_projection(collection, &value_type, false)
}
