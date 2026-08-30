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
- Bare current-package function references (the signature must be an
  `imported_package_export`).
- Functions that are not exported through an import.
- Functions whose first parameter is not `ThreadWorker OF Msg TO Out`.
- Functions whose return type does not match `Out`.

The first four are enforced together by the thread-builtin call checker, which
requires the entry argument to resolve to a visible
signature that is simultaneously an imported-package export, a `FUNC`,
and isolated; failure reports `TYPE_CALL_ARGUMENT_MISMATCH` with the message
`thread.start entry point must be an exported ISOLATED FUNC from an imported
package.`. The parameter-shape and return-type checks are the ordinary
function-reference signature match. [[src/ir/shape.rs:check_builtin_call]] [[src/codegen/registry/mod.rs:resolve_call]]

The `imported_package_export` requirement is not weakened by the reserved
`IMPORT self` specifier. In a `kind: "package"` project, `IMPORT self` registers
the current package's own `EXPORT` declarations as imported-package signatures
(`imported_package_export == true`), keyed under the `self`/alias binding, so a
`self::worker` entry satisfies the *same* checker with no `self`-specific branch:
the checker never learns about `self`. A bare, unqualified current-package
reference still carries `imported_package_export == false` and is still rejected;
only the `self::`-qualified path to an `EXPORT ISOLATED FUNC` newly resolves.
`self` sees only `EXPORT` symbols, exactly as an external importer does.
[[src/resolver/packages.rs]] [[src/resolver/packages.rs:resolve_imported_package]]

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
  block. Opaque resource handles opt in through resource metadata. [[src/ir/verify/resources.rs:is_thread_sendable]]
- Statically known non-sendable `In`, `Msg`, `Out`, or `Res` types are rejected
  before lowering, error code
  `TYPE_THREAD_NOT_SENDABLE`. The data plane is additionally resource-free: a
  `Msg` that is itself a resource type is rejected at `thread::send` with guidance
  to use `thread::transfer`. [[src/ir/verify/resources.rs:check_thread_boundary_sendability]]
- Runtime helpers and the verifier still consult type metadata so queued values
  can be moved, dropped, or closed correctly.

## See Also

* ./mfb man thread — the source-level thread API (`send`/`receive`/`transfer`/`accept`)
* ./mfb spec language threads — the source-level thread model, signatures, and sendability table
* ./mfb spec threading queue-semantics — move, timeout, and cancellation behavior
