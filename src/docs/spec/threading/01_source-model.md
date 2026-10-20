# Source Model

The thread entry shape, the `thread::start`/`transfer`/`accept` signatures, the
`Thread`/`ThreadWorker` type grammar, and the sendability rules are defined as a
source-level API by `./mfb spec language threads` and `./mfb man thread`. A thread
entry point is an `ISOLATED FUNC` whose first parameter is
`ThreadWorker OF Msg TO Out`, passed to `thread::start` by function identifier.
This topic specifies only the compiler-side *enforcement* of those rules.

## Entry-point enforcement

The compiler rejects:

- Lambdas and closures as thread entry points (only a function identifier is
  accepted).
- Non-isolated functions (the signature must be `isolated`).
- Functions whose first parameter is not `ThreadWorker OF Msg TO Out`.
- Functions whose return type does not match `Out`.

The entry-name and isolation checks require an identifier resolving to an
isolated function in the current project or an imported signature; failure reports
`TYPE_CALL_ARGUMENT_MISMATCH` with the message
`thread.start entry point must name an ISOLATED FUNC.`. The parameter-shape and
return-type checks are the ordinary function-reference signature match.
[[src/ir/shape/expressions.rs:thread_start_entry_valid]] [[src/codegen/registry/unify.rs:resolve_call]]

**Provenance is not a criterion.** `ISOLATED` is the sole marker: an entry may be
a function this project declares, named by its bare identifier at any visibility
(`PRIVATE`, `PUBLIC` or `EXPORT`) and in a project of any `kind`, or an imported
package's `EXPORT ISOLATED FUNC` named through its import binding. Both spellings lower to a `FunctionRef`; entry validation resolves the bare
name through local functions or the qualified name through imported signatures. `self` is an ordinary identifier, not a reserved import
specifier.
[[src/ir/shape/expressions.rs:thread_start_entry_valid]]

## Thread type grammar (parsing)

The three `Thread`/`ThreadWorker OF …` spellings — `<Msg> TO <Out>`,
`<Msg> RES <Res> TO <Out>`, and resource-only `RES <Res> TO <Out>` (with `Msg`
defaulting to `Nothing`) — are documented by `./mfb spec language threads`. The
compiler parses them, producing
the internal structural view `(message, resource, output)` where `resource`
is an `Option`. `thread::start` derives the parent `Thread` type from the worker's
`ThreadWorker` first parameter, preserving the `RES` clause. [[src/types.rs:split_thread_types]]

## Sendability enforcement

The `In`/`Msg`/`Out`/`Res` sendability rules (which scalar, collection, record,
union, and resource types may cross a boundary) are owned by
`./mfb spec language threads`. The compiler implements them as follows:

- Thread sendability is a type property decided by the thread-sendability
  predicate; it is not stored as a per-value flag in every value's memory
  block. Opaque resource handles opt in through resource metadata. [[src/ir/verify/resources/sendability.rs:thread_unsendable_cause]]
- Boundary checks run during IR verification. Non-sendable types report
  `TYPE_THREAD_NOT_SENDABLE`; resource/data-plane violations report
  `TYPE_THREAD_RESOURCE_PLANE_REQUIRED`; transferring a worker view reports
  `TYPE_THREAD_WORKER_NOT_TRANSFERABLE`. The declared handle planes and the
  boundary call arguments are checked separately.
  [[src/ir/verify/resources/sendability.rs:require_thread_sendable]]
  [[src/ir/verify/resources/sendability.rs:check_thread_sendability]]
  [[src/ir/verify/resources/sendability.rs:check_thread_boundary_sendability]]
- Runtime queue helpers are type-agnostic. Codegen passes per-type copy/drop
  function pointers so values are materialized and resources closed correctly.
  [[src/codegen/runtime/thread/value_functions.rs:emit_thread_value_fn_address]]

## See Also

* ./mfb man thread — the source-level thread API (`send`/`receive`/`transfer`/`accept`)
* ./mfb spec language threads — the source-level thread model, signatures, and sendability table
* ./mfb spec threading queue-semantics — move, timeout, and cancellation behavior
