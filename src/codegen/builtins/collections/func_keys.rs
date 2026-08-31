//! `collections::keys` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::typed_map_type_parts;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;
const INTO_KEYS: &str = "Return a map's keys as a list.";
const DESC_KEYS: &str = r#"`collections::keys` builds a new `List OF K` holding the key of every entry in
`value`. It walks the map's lookup-entry table front to back, copying each
entry's key payload into a new list. The source map is not
mutated and its own storage is not aliased by the result — the returned list is
an independent collection.

The result has exactly one item per map entry, so its length equals
`len(value)`. An empty map yields an empty list. Each key appears exactly once,
because a map holds at most one entry per key.

**Ordering.** The projection walks the lookup-entry array directly, and that
array is maintained in insertion order; the hash bucket index is separate
derived metadata that does not reorder it. `collections::keys` and
`collections::values` walk the same array over the same entries and differ only
in which payload field of each entry they copy, so the two results are
index-aligned: item `i` of `collections::keys(m)` is the key of the entry whose
value is item `i` of `collections::values(m)`. The language specification
describes map iteration order as implementation-defined but stable for a given
unchanged map, so treat insertion order as the current implementation's behavior
rather than a guarantee to rely on across versions.

`collections::keys` raises no trappable domain error, so an inline `TRAP` on a
`keys` call has a dead handler. Building the result list needs memory, and
running out of memory is not something a `TRAP` can catch."#;

const EX: &str = r#"Get the keys of a map:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  LET names AS List OF String = collections::keys(ages)
  io::print(toString(len(names)))
  RETURN 0
END FUNC
```

Iterate a map by key:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36, "Grace" := 85 }
  FOR EACH name IN collections::keys(ages)
    io::print(name & " is " & toString(collections::getOr(ages, name, 0)))
  NEXT
  RETURN 0
END FUNC
```

The keys and values projections line up index for index:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36, "Grace" := 85 }
  LET names AS List OF String = collections::keys(ages)
  LET numbers AS List OF Integer = collections::values(ages)
  io::print(collections::get(names, 0) & "=" & toString(collections::get(numbers, 0)))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "keys",
        intro: INTO_KEYS,
        desc: DESC_KEYS,
        example: EX,
        expected_arguments: Some("Map OF K TO V"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The map whose keys to collect. Not modified. The order matches `collections::values` for the same unchanged map.",
                aliases: &["map"],
                ty: ParameterType::map_of(ParameterType::var("K"), ParameterType::var("V")),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::var("K")),
            errors: vec![],
            body: Body::abi_inline(lower_keys),
        }],
    });
}

/// `collections::keys(Map OF K TO V) AS List OF K`: project each entry's key.
pub(crate) fn lower_keys(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let collection = &args[0];
    let Some((key_type, _)) =
        typed_map_type_parts(&collection.type_).map(|(k, v)| (k.clone(), v.clone()))
    else {
        return Err(format!(
            "native collection keys does not accept {}",
            collection.type_
        ));
    };
    builder.lower_map_projection(collection, &key_type, true)
}
