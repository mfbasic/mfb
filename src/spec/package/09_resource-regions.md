# Resource regions

Resource lifetime is represented implicitly by the lexical scope of the binding that owns the resource — not by explicit resource ops and not by a side cleanup table. There is **no** `RESOURCE_ENTER`, `RESOURCE_LEAVE`, or `RESOURCE_CLOSE` op in the Binary Representation.

* A resource is owned by the `Bind` that introduces it and lives for the lexical extent of the region (function body, loop body, branch, or trap) that contains that `Bind`.
* When the owning region exits, a compiler-generated lexical drop closes the resource exactly once if it is still owned. The drop is keyed off the binding's resource type and the close function recorded in `RESOURCE_TABLE`; it is not itself encoded as an op.
* An explicit close (the resource's declared consuming close operation) marks the binding moved, so the lexical drop does not close it again.
* Because regions are nested in the IR tree, every structured exit path — fall-through, `Return`, `Fail`, `ExitLoop`, `ContinueLoop`, `ExitProgram`, and trap routing — is bounded by the enclosing region; there are no PC ranges to reconstruct and no "jump into a cleanup region" to reject.

The resource model closes files, sockets, and similar handles by lexical drop when their owning binding leaves scope. The structured Binary Representation makes that rule directly verifiable from each binding's type and scope. (User-defined source resources reuse this same implicit-drop model; see `plan-resource.md`.)

## Relationship to the function-table cleanup fields

For historical layout reasons the `FUNCTION_TABLE` entry format still contains a `cleanupCount`/`cleanupOffset` pair and a per-function cleanup table whose records carry `startPc`/`endPc`/`resourceRegister`/`closeFunctionId` (see `functions`). Those fields are a remnant of the retired flat machine. The current producer (`lower_function`) always emits an **empty** cleanup table for structured functions, so no resource lifetime is actually encoded there. The implicit-drop model described above — keyed off each `Bind`'s resource type and the close function in `RESOURCE_TABLE` — is the real and only mechanism. An implementer should treat the cleanup-table fields as reserved/empty and must not reconstruct resource lifetime from PC ranges.

The resource → close-function mapping that the lexical drop relies on lives in `RESOURCE_TABLE` (see `native-bindings`): each entry maps a resource `typeId` to a `closeFunctionId` and flags (native/standard/sendable/close-may-fail).
