# Resource regions

Resource lifetime is represented implicitly by the lexical scope of the binding that owns the resource — not by explicit resource ops and not by a side cleanup table. There is **no** `RESOURCE_ENTER`, `RESOURCE_LEAVE`, or `RESOURCE_CLOSE` op in the Binary Representation.

* A resource is owned by the `Bind` that introduces it and lives for the lexical extent of the region (function body, loop body, branch, or trap) that contains that `Bind`.
* When the owning region exits, a compiler-generated lexical drop closes the resource exactly once if it is still owned. The drop is keyed off the binding's resource type and the close function recorded in `RESOURCE_TABLE`; it is not itself encoded as an op.
* An explicit close (the resource's declared consuming close operation) marks the binding moved, so the lexical drop does not close it again.
* Because regions are nested in the IR tree, every structured exit path — fall-through, `Return`, `Fail`, `ExitLoop`, `ContinueLoop`, `ExitProgram`, and trap routing — is bounded by the enclosing region; there are no PC ranges to reconstruct and no "jump into a cleanup region" to reject.

The resource model closes files, sockets, and similar handles by lexical drop when their owning binding leaves scope. The structured Binary Representation makes that rule directly verifiable from each binding's type and scope. (User-defined source resources reuse this same implicit-drop model; see `plan-resource.md`.)
