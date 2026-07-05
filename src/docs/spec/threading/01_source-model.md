# Source Model

The thread entry shape, the `thread::start`/`transfer`/`accept` signatures, the
`Thread`/`ThreadWorker` type grammar, and the sendability rules are defined as a
source-level API by `./mfb spec language threads` and `./mfb man thread`. A thread
entry point is an exported `ISOLATED FUNC` whose first parameter is
`ThreadWorker OF Msg TO Out`, passed to `thread::start` by bare function
identifier. This topic specifies only the compiler-side *enforcement* of those
rules.

## Entry-point enforcement

The compiler rejects:

- Lambdas and closures as thread entry points (only a bare function identifier is
  accepted).
- `SUB` thread entry points (the entry must be `FunctionKind::Func`).
- Non-isolated functions (the signature must be `isolated`).
- Current-package functions (the signature must be an `imported_package_export`).
- Functions that are not exported from an imported package.
- Functions whose first parameter is not `ThreadWorker OF Msg TO Out`.
- Functions whose return type does not match `Out`.

The first four are enforced together by `check_thread_builtin_call` in
`syntaxcheck.rs`, which requires the entry argument to resolve to a visible
signature that is simultaneously `imported_package_export`, `FunctionKind::Func`,
and `isolated`; failure reports `TYPE_CALL_ARGUMENT_MISMATCH` with the message
`thread.start entry point must be an exported ISOLATED FUNC from an imported
package.`. The parameter-shape and return-type checks are the ordinary
function-reference signature match performed by `builtins::thread::resolve_call`. [[src/syntaxcheck/builtins.rs:check_thread_builtin_call]]

## Thread type grammar (parsing)

The three `Thread`/`ThreadWorker OF …` spellings — `<Msg> TO <Out>`,
`<Msg> RES <Res> TO <Out>`, and resource-only `RES <Res> TO <Out>` (with `Msg`
defaulting to `Nothing`) — are documented by `./mfb spec language threads`. The
compiler parses them with `split_thread_types` in `builtins/thread.rs`, producing
the internal structural view `(kind, message, resource, output)` where `resource`
is an `Option`. `thread::start` derives the parent `Thread` type from the worker's
`ThreadWorker` first parameter, preserving the `RES` clause. [[src/builtins/thread.rs:split_thread_types]]

## Sendability enforcement

The `In`/`Msg`/`Out`/`Res` sendability rules (which scalar, collection, record,
union, and resource types may cross a boundary) are owned by
`./mfb spec language threads`. The compiler implements them as follows:

- Thread sendability is a type property decided by `is_thread_sendable_type` in
  `syntaxcheck.rs`; it is not stored as a per-value flag in every value's memory
  block. Opaque resource handles opt in through resource metadata
  (`resource_registry.is_sendable`). [[src/syntaxcheck/resources.rs:is_thread_sendable_type]]
- Statically known non-sendable `In`, `Msg`, `Out`, or `Res` types are rejected
  before lowering by `check_thread_boundary_sendability`, error code
  `TYPE_THREAD_NOT_SENDABLE`. The data plane is additionally resource-free: a
  `Msg` that is itself a resource type is rejected at `thread::send` with guidance
  to use `thread::transfer`. [[src/syntaxcheck/resources.rs:check_thread_boundary_sendability]]
- Runtime helpers and the verifier still consult type metadata so queued values
  can be moved, dropped, or closed correctly.

## See Also

* ./mfb man thread — the source-level thread API (`send`/`receive`/`transfer`/`accept`)
* ./mfb spec language threads — the source-level thread model, signatures, and sendability table
* ./mfb spec threading queue-semantics — move, timeout, and cancellation behavior
