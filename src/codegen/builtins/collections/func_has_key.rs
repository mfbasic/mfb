//! `collections::hasKey` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::{map_type_parts, set_element_type};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;
const INTO_HAS_KEY: &str = "Test whether a map contains an entry for a key.";
const DESC_HAS_KEY: &str = r#"`collections::hasKey` returns `TRUE` when `value` holds an entry whose key
matches `key`, and `FALSE` otherwise. The map is neither copied nor mutated, and
the matching value is never materialized — only the key is compared.

This is a map-only member. There is no list or `String` form: to test list
membership use `collections::contains`, and to test for a substring use the
`strings::` package.

Key comparison is a comparison of the stored key payload. Fixed-width keys
compare their raw stored bits (one byte for `Boolean` and `Byte`, four for
`Scalar`, eight for `Integer`, `Float`, `Fixed`, and `Money`), and a `String`
key compares its length first and then its bytes. Because the comparison is
bitwise, a `Float` key of `NaN` never reports as present and `-0.0` does not
match a stored `0.0`.

For the key types `String`, `Integer`, `Float`, `Fixed`, `Byte`, and `Boolean`
the probe uses the map's hash bucket index; other key types use a linear scan of
the entry table. Both paths compare exactly the same key bytes and return the
same answer.

`collections::hasKey` raises no trappable domain error, so an inline `TRAP` on a
`hasKey` call has a dead handler.

Use `hasKey` to guard a `collections::get`, which *does* fail on a missing key.
When the goal is simply to obtain a value with a fallback,
`collections::getOr` does it in one call and avoids the second lookup."#;

const EX: &str = r#"Test map membership:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  io::print(toString(collections::hasKey(ages, "Ada")))
  io::print(toString(collections::hasKey(ages, "Grace")))
  RETURN 0
END FUNC
```

Guard a lookup that would otherwise fail:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  IF collections::hasKey(ages, "Ada") THEN
    io::print(toString(collections::get(ages, "Ada")))
  END IF
  RETURN 0
END FUNC
```

Confirm that removing a key takes it out of the result:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  LET without AS Map OF String TO Integer = collections::removeKey(ages, "Ada")
  io::print(toString(collections::hasKey(without, "Ada")))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hasKey",
        intro: INTO_HAS_KEY,
        desc: DESC_HAS_KEY,
        example: EX,
        expected_arguments: Some("Map OF K TO V, K"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "",
                    aliases: &["map"],
                    ty: ParameterType::map_of(ParameterType::Var("K"), ParameterType::Var("V")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "key",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Var("K"),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_inline_self(lower_has_key),
        }],
    });
}

/// `collections::hasKey(Map OF K TO V, K) AS Boolean`: key membership via the
/// shared hash-probe / linear-scan (`emit_key_membership`).
pub(crate) fn lower_has_key(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let collection = builder.lower_value(&args[0])?;
    let collection_slot = builder.allocate_stack_object("has_key_collection", 8);
    builder.emit(abi::store_u64(
        &collection.location,
        abi::stack_pointer(),
        collection_slot,
    ));
    let key = builder.lower_value(&args[1])?;
    let key_slot = builder.allocate_stack_object("has_key_key", 8);
    // `d`-native float key stores via `str d` (plan-01 float-dnative).
    builder.store_value_at(&key, abi::stack_pointer(), key_slot);

    // A Map keys on its key type; a Set (reached via `contains`, plan-63-B)
    // keys on its element type — both are the bytes the probe compares.
    let Some(key_type) = map_type_parts(&collection.type_)
        .map(|(key, _)| key.to_string())
        .or_else(|| set_element_type(&collection.type_))
    else {
        return Err(format!(
            "native collection hasKey/contains does not accept {}",
            collection.type_
        ));
    };
    let key_type = key_type.as_str();
    if key.type_ != key_type {
        return Err(format!(
            "native collection hasKey key must be {}, got {}",
            key_type, key.type_
        ));
    }
    builder.emit_key_membership(
        collection_slot,
        key_slot,
        key_type,
        "has_key",
        &collection.type_,
    )
}
