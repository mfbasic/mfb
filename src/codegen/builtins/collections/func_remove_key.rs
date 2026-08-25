//! `collections::removeKey` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::typed_map_type_parts;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTO_REMOVE_KEY: &str = "Return a copy of a map with the entry for one key removed.";
const DESC_REMOVE_KEY: &str = r#"`collections::removeKey` produces a **new** map containing every entry of
`value` except the one whose key matches `key`. It does not edit `value` in
place: the lowering scans the entry table to count the entries it will retain
and size their payloads, allocates a fresh map block, and copies the retained
entries into it. The original map is left untouched and remains usable.

Retained entries are copied in their existing order, so the surviving entries of
the result keep the relative order they had in `value`.

Removing a key that is not present is not an error. The scan simply retains
every entry, and the call returns a fresh map with the same contents as `value`.
Note that this is a new map rather than the same map object — a `removeKey` for
an absent key still allocates and copies, it does not return the argument
itself. The result therefore has `len(value)` entries when `key` was absent, or
`len(value) - 1` entries when it was present. Because a map holds at most one
entry per key, at most one entry is ever dropped.

Key comparison is a comparison of the stored key payload: fixed-width keys
compare their raw stored bits and a `String` key compares length and then bytes.
Since the comparison is bitwise, a `Float` key of `NaN` matches no entry, so
such a call always returns an unchanged copy.

`collections::removeKey` raises no trappable domain error — neither a missing
key nor an empty map fails — so an inline `TRAP` on a `removeKey` call has a
dead handler. Building the result map does allocate, and an allocation failure
is not a trappable domain error in this language."#;

const EX: &str = r#"Remove a key:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36, "Grace" := 85 }
  LET smaller AS Map OF String TO Integer = collections::removeKey(ages, "Ada")
  io::print(toString(len(smaller)))
  io::print(toString(collections::hasKey(smaller, "Ada")))
  RETURN 0
END FUNC
```

The original map is unchanged:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  LET smaller AS Map OF String TO Integer = collections::removeKey(ages, "Ada")
  io::print(toString(collections::hasKey(ages, "Ada")))
  RETURN 0
END FUNC
```

Removing an absent key is harmless:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  LET same AS Map OF String TO Integer = collections::removeKey(ages, "Grace")
  io::print(toString(len(same)))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "removeKey",
        intro: INTO_REMOVE_KEY,
        desc: DESC_REMOVE_KEY,
        example: EX,
        expected_arguments: Some("Map OF K TO V, K"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "",
                    aliases: &["map"],
                    ty: ParameterType::map_of(ParameterType::var("K"), ParameterType::var("V")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "key",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::var("K"),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Arg(0),
            errors: vec![],
            body: Body::abi_inline(lower_remove_key),
        }],
    });
}

/// `collections::removeKey` — a new map with the entry for `key` dropped (no-op
/// if absent). Reuses `lower_map_remove_key`.
pub(crate) fn lower_remove_key(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let map = args[0].clone();
    let Some((key_type, _)) = typed_map_type_parts(&map.type_)
        .map(|(key, value)| (key.name().into_owned(), value.name().into_owned()))
    else {
        return Err(format!(
            "native collection removeKey does not accept {}",
            map.type_
        ));
    };
    let map_slot = builder.allocate_stack_object("remove_key_map", 8);
    builder.emit(abi::store_u64(
        &map.location,
        abi::stack_pointer(),
        map_slot,
    ));
    let key = args[1].clone();
    if key.type_.name() != key_type.as_str() {
        return Err(format!(
            "native collection removeKey key must be {}, got {}",
            key_type, key.type_
        ));
    }
    let key_slot = builder.allocate_stack_object("remove_key_key", 8);
    // `d`-native float key stores via `str d` (plan-01 float-dnative).
    builder.store_value_at(&key, abi::stack_pointer(), key_slot);
    builder.lower_map_remove_key(map_slot, key_slot, &map.type_.name(), &key_type)
}
