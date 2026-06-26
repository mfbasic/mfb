# Control Block

The native thread handle points to a runtime control block. The current native
layout is an implementation ABI between helper lowering and generated code:

```text
offset  field
0       state
8       cancelled
16      result tag
24      result value
32      result error
40      inbound queue handle
48      outbound queue handle
56      OS handle
64      entry function pointer
72      input data
80      worker arena state
88      parent arena state
```

`state = 0` means running. `state = 1` means complete with an unretrieved result. `state = 2` means the parent `Thread` handle is closed because the result was retrieved or the handle was dropped.

The `result tag`, `result value`, and `result error` fields describe the
completed `Result OF Out`. Heap-backed success or error payloads stored through
these fields must either be runtime-owned transfer values, values materialized
into a receiver-valid arena, or values whose producer arena is kept live by the
control block until the one result retrieval materializes its receiver-owned
copy.

The inbound and outbound queue handle fields point to runtime-owned bounded
queue records, not directly to a single queued message. A queue record stores
its requested capacity, current occupancy, synchronization state, and a backing
ring/buffer of value slots. The source-level contract is bounded queues with the
behavior specified in `standard_package.md`; implementation changes must
preserve that contract.

Queue storage must preserve enough type metadata to drop or close queued values
without receiving them. For queued resource handles, the runtime uses the
resource close function recorded in package metadata. For queued composite
values, the runtime uses the type metadata table to walk owned fields or payloads
that require cleanup.
