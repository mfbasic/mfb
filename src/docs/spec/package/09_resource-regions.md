# Resource regions

Resource cleanup is derived from typed bindings, nested scopes, and each function's encoded `resource_owners` map. A `Float` owner can transfer cleanup responsibility to another binding; it is not necessarily the binding that first introduces the handle. There are no explicit `RESOURCE_ENTER`, `RESOURCE_LEAVE`, or `RESOURCE_CLOSE` nodes. Native lowering reconstructs cleanup using these ownership decisions. Explicit close and scope cleanup are distinct operations; their source-level contract belongs to `./mfb spec language resource-management`. [[src/ir/binary/mod.rs:encode_function]] [[src/ir/binary/mod.rs:decode_resource_owners]]

Concretely for the encoding: because regions are nested in the IR tree, every exit path is bounded by its enclosing region — there are no PC ranges to reconstruct and no "jump into a cleanup region" to reject. The compiler-generated drop is not itself encoded as an op; it is reconstructed at merge/lower time, keyed off the binding's resource type and the close function recorded in `RESOURCE_TABLE`. The structured Binary Representation therefore makes the lifetime directly verifiable from each binding's type and scope.

## Relationship to the function-table cleanup fields

The `FUNCTION_TABLE` entry format still contains a `cleanupCount`/`cleanupOffset` pair and a per-function cleanup table whose records carry `startPc`/`endPc`/`resourceRegister`/`closeFunctionId` (see `functions`). The producer always emits an **empty** cleanup table for structured functions, so no resource lifetime is actually encoded there. [[src/binary_repr/writer.rs:lower_function]] The implicit-drop model described above — keyed off each `Bind`'s resource type and the close function in `RESOURCE_TABLE` — is the real and only mechanism. An implementer should treat the cleanup-table fields as reserved/empty and must not reconstruct resource lifetime from PC ranges.

The resource → close-function mapping that the lexical drop relies on lives in `RESOURCE_TABLE` (see `native-bindings`): each entry maps a resource `typeId` to a `closeFunctionId` and flags (native/standard/sendable/close-may-fail).

## See Also

* ./mfb spec language resource-management — the source-level resource model
* ./mfb spec memory — runtime value and resource representation
