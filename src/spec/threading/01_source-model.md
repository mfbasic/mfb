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

- Lambdas and closures as thread entry points.
- `SUB` thread entry points.
- Non-isolated functions.
- Current-package functions.
- Functions that are not exported from an imported package.
- Functions whose first parameter is not `ThreadWorker OF Msg TO Out`.
- Functions whose return type does not match `Out`.

`Msg` is the message type used by `thread::send`, `thread::receive`, and
`thread::poll`. `Out` is the worker success type. `In` is the input value type
passed to `thread::start`.

`In`, `Msg`, and `Out` must be thread-sendable. Thread sendability is a type
metadata property carried in package type metadata and available to the compiler,
verifier, and runtime. It is not stored as a per-value flag in every value's
memory block.

Thread sendability is derived by type:

- Primitive owned values, `String`, and `Nothing` are sendable.
- `List OF T` is sendable when `T` is sendable.
- `Map OF K TO V` is sendable when `K` and `V` are sendable.
- Records are sendable when every field type is sendable.
- Unions are sendable when every payload type is sendable; a worker outcome (internally a fallible result) is sendable when its success type is.
- Opaque handles are not sendable by default. Each concrete handle type opts in.
- Standard `File`, `Socket`, and `UdpSocket` handles are sendable.
- `Listener`, `Thread`, and `ThreadWorker` handles are not sendable.

The compiler rejects statically known non-sendable `In`, `Msg`, or `Out` types
before lowering. Runtime helpers and the verifier still consult type metadata so
queued values can be moved, dropped, or closed correctly.
