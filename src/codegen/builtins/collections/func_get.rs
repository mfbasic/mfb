//! `collections::get` — the first fully self-owned builtin (plan-95 Phase 4).
//!
//! This file owns everything specific to `get`: its `BuiltinFunction` descriptor
//! entry (with doc strings and declared errors), and its target-generic lowering
//! (`lower_get`, reached through `Implementation::Native` by the codegen
//! dual-path seam). The lowering emits only through the `abi::` seam, so it is
//! target-generic; `CodeBuilder` and the `NirValue`/`ValueResult` value types
//! still live in `src/target` and are referenced here (the accepted temporary
//! `codegen → target` edge until `CodeBuilder` itself relocates).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::{typed_list_element_type, typed_map_type_parts};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;
const INTO_GET: &str = "Read a list item by index or a map value by key.";
const DESC_GET: &str = r#"`collections::get` reads one element out of a collection. The collection itself
is neither copied nor mutated: the lowering stores only a handle to it, walks
its lookup table, and materializes just the selected payload.

The value returned is **owned** by the caller. Scalars are returned by value and
a `String` payload is materialized fresh, while a composite payload stored
inline in the collection's data region is copied into a standalone arena block
before it is handed back, so binding, storing, and freeing the result cannot
disturb the source collection.

`get` is the only fallible member of this group. It reports a missing element as
a trappable domain error rather than substituting anything, and it is
raw-supported, so an inline `TRAP` on a `collections::get` call catches the real
runtime error. When a fallback value is more convenient than an error, use
`collections::getOr`; when only presence matters, use `collections::hasKey`.

For the map overload, key comparison is a comparison of the stored key payload:
fixed-width keys compare their raw 64-bit (or 32-bit, or single-byte) stored
bits and `String` keys compare length first and then bytes. A `Float` key is
therefore matched bit-for-bit, so `NaN` never matches any key and `-0.0` does
not match a stored `0.0`.

Map lookup for the common key types `String`, `Integer`, `Float`, `Fixed`,
`Byte`, and `Boolean` goes through the map's hash bucket index; other key types
fall back to a linear scan of the entry table. This is a performance difference
only — both paths select the same entry and raise the same error when the key is
absent."#;

/// The descriptor entry for `collections::get`, wired to `lower_get`.
const EX: &str = r#"Read a list item by index:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [10, 20, 30]
  io::print(toString(collections::get(numbers, 0)))
  RETURN 0
END FUNC
```

Read a map value by key:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  io::print(toString(collections::get(ages, "Ada")))
  RETURN 0
END FUNC
```

Guard the lookup so the missing-key error cannot be raised:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "Ada" := 36 }
  IF collections::hasKey(ages, "Grace") THEN
    io::print(toString(collections::get(ages, "Grace")))
  END IF
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "get",
        intro: INTO_GET,
        desc: DESC_GET,
        example: EX,
        expected_arguments: Some("List OF T, Integer or Map OF K TO V, K"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "",
                        aliases: &["collection"],
                        ty: ParameterType::list_of(ParameterType::var("T")),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "index",
                        desc: "",
                        aliases: &["key"],
                        ty: ParameterType::Integer,
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::var("T"),
                errors: vec!["ErrIndexOutOfRange", "ErrNotFound"],
                body: Body::abi_inline(lower_get),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "",
                        aliases: &["collection"],
                        ty: ParameterType::map_of(ParameterType::var("K"), ParameterType::var("V")),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "index",
                        desc: "",
                        aliases: &["key"],
                        ty: ParameterType::var("K"),
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::var("V"),
                errors: vec!["ErrIndexOutOfRange", "ErrNotFound"],
                body: Body::abi_inline(lower_get),
            },
        ],
    });
}

/// Target-generic lowering for `collections::get` (moved verbatim from the former
/// `CodeBuilder::lower_collection_get`). Emits through `abi::`; branches to the
/// list or map lowering by the collection's static type.
pub(crate) fn lower_get(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    // plan-86 G1: is `args[1]` a provably-in-range index into the list `args[0]`?
    // (Computed from the RAW NIR before lowering, against `provable_index_locals`.)
    let unchecked = builder.is_provable_index_access(&args[0], &args[1]);
    let collection = args[0].clone();
    let collection_slot = builder.allocate_stack_object("get_collection", 8);
    builder.emit(abi::store_u64(
        &collection.location,
        abi::stack_pointer(),
        collection_slot,
    ));

    let key = args[1].clone();
    let key_slot = builder.allocate_stack_object("get_key", 8);
    // A `d`-native float map key stores via `str d`, bit-identical to the
    // `str x` a later bitwise key compare reads (plan-01 float-dnative).
    builder.store_value_at(&key, abi::stack_pointer(), key_slot);

    if let Some(element_type) = typed_list_element_type(&collection.type_).cloned() {
        if key.type_ != ParameterType::Integer {
            return Err(format!(
                "native collection get list index must be Integer, got {}",
                key.type_
            ));
        }
        let result = builder.lower_list_get(
            collection_slot,
            key_slot,
            &collection.type_,
            &element_type,
            unchecked,
        )?;
        return builder.materialize_owned_element(result);
    }

    if let Some((key_type, value_type)) =
        typed_map_type_parts(&collection.type_).map(|(k, v)| (k.clone(), v.clone()))
    {
        if key.type_ != key_type {
            return Err(format!(
                "native collection get map key must be {}, got {}",
                key_type, key.type_
            ));
        }
        let result = builder.lower_map_get(
            collection_slot,
            key_slot,
            &collection.type_,
            &key_type,
            &value_type,
        )?;
        return builder.materialize_owned_element(result);
    }

    Err(format!(
        "native collection get does not accept {}",
        collection.type_
    ))
}
