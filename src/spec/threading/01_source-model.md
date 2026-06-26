# Source Model

Thread entry points have this shape:

```text
EXPORT ISOLATED FUNC worker(t AS ThreadWorker OF Msg TO Out, input AS In) AS Out
  ...
END FUNC
```

`thread::start` accepts a function reference to such an exported function:

```text
thread::start OF In, Msg, Out(
  f AS ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out,
  data AS In,
  inboundLimit AS Integer = 64,
  outboundLimit AS Integer = 64
) AS Thread OF Msg TO Out
```

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
`typecheck.rs`, which requires the entry argument to resolve to a visible
signature that is simultaneously `imported_package_export`, `FunctionKind::Func`,
and `isolated`; failure reports `TYPE_CALL_ARGUMENT_MISMATCH` with the message
`thread.start entry point must be an exported ISOLATED FUNC from an imported
package.`. The parameter-shape and return-type checks are the ordinary
function-reference signature match performed by `builtins::thread::resolve_call`.

## Thread type grammar

`Thread` and `ThreadWorker` types carry their channel types in the type spelling.
The body after `Thread OF ` / `ThreadWorker OF ` has three shapes (parsed by
`split_thread_types` in `builtins/thread.rs`):

```text
<Msg> TO <Out>                 ; data-only thread
<Msg> RES <Res> TO <Out>       ; data plane + resource plane
RES <Res> TO <Out>             ; resource-only (Msg defaults to Nothing)
```

- `Msg` is the data-plane message type used by `thread::send`, `thread::receive`,
  and `thread::poll`. For a resource-only thread it defaults to `Nothing`.
- `Res` is the resource-plane type used by `thread::transfer`/`thread::accept`. It
  is present only when the type carries a `RES Res` clause.
- `Out` is the worker success type. `In` is the input value type passed to
  `thread::start`.

The internal structural view is `(kind, message, resource, output)`, with
`resource` an `Option`. `thread::start` derives the parent `Thread` type from the
worker's `ThreadWorker` first parameter, preserving the `RES` clause.

## Resource plane

The resource plane is a second pair of queues for moving **resource handles**
(`File`, `Socket`, …) across the boundary. It mirrors the data plane but is kept
separate so the data channel stays resource-free:

```text
thread::transfer(t AS Thread/ThreadWorker OF [Msg] RES Res TO Out,
                 res AS RES Res, timeoutMs AS Integer = 0) AS Nothing
thread::accept(t AS Thread/ThreadWorker OF [Msg] RES Res TO Out,
               timeoutMs AS Integer = 0) AS RES Res
```

`transfer` mirrors `send` and `accept` mirrors `receive`. `Res` must be a
thread-sendable resource type; a thread typed with resource `Unknown` accepts any
resource. Passing a non-resource value, or operating on a thread that carries no
`RES` clause, fails to type-check (`TYPE_THREAD_NOT_SENDABLE`). See
`queue-semantics` for the move/timeout/cancellation behavior and
`src/man/builtins/thread/{transfer,accept}.txt` for the full source contract.

## Sendability

`Msg` is the data-plane message type. `Out` is the worker success type. `In` is
the input value type passed to `thread::start`. `Res` is the resource-plane type.

`In`, `Msg`, `Out`, and `Res` must be thread-sendable. Thread sendability is a
type property decided by `is_thread_sendable_type` in `typecheck.rs`; it is not
stored as a per-value flag in every value's memory block.

Thread sendability is derived by type:

- `Boolean`, `Byte`, `Integer`, `Float`, `Fixed`, `String`, `Nothing`, `Error`,
  `ErrorLoc`, and `Unknown` are sendable.
- `List OF T` is sendable when `T` is sendable.
- `Map OF K TO V` is sendable when `K` and `V` are sendable.
- `Result OF Success` is sendable when `Success` is — this is how a worker
  outcome (internally a fallible result) is sendable.
- Records are sendable when every field type is sendable.
- Unions are sendable when every payload type in every variant is sendable; bare
  enums are always sendable.
- Opaque resource handles are not sendable by default. Each concrete handle type
  opts in through resource metadata (`resource_registry.is_sendable`). Standard
  `File`, `Socket`, and `UdpSocket` handles are sendable; `Listener` and
  `TlsSocket` are not.
- `Thread`, `ThreadWorker`, `Function` types, and `Res(...)` resource collections
  are not sendable.

The data plane is additionally **resource-free**: a `Msg` that is itself a
resource type is rejected at `thread::send` with the guidance to use
`thread::transfer` instead. Resources cross only on the resource plane.

The compiler rejects statically known non-sendable `In`, `Msg`, `Out`, or `Res`
types before lowering (`check_thread_boundary_sendability`, error code
`TYPE_THREAD_NOT_SENDABLE`). Runtime helpers and the verifier still consult type
metadata so queued values can be moved, dropped, or closed correctly.
