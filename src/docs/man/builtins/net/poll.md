# poll

Test whether a socket has data ready to read.

## Synopsis

```
net::poll(sock AS Socket) AS Boolean
net::poll(sock AS Socket, timeoutMs AS Integer) AS Boolean
```

## Package

`net`

## Imports

```
IMPORT net
```

`net` is a built-in package, so no manifest dependency is required.
[[src/builtins/net.rs:is_net_call]]

## Description

`net::poll` reports whether a connected `Socket` is readable. It returns `TRUE`
when a following `net::read` or `net::readText` can proceed without blocking —
including the case where the peer has closed and that read would report end of
stream — and `FALSE` when nothing became readable before the deadline. The
socket is borrowed and inspected only: no data is consumed, so a `TRUE` result
leaves the bytes in place for the next read.
[[src/target/shared/code/net/poll.rs:lower_net_poll_helper]]

`timeoutMs` bounds the wait, in milliseconds. When it is omitted the compiler
supplies `0`, which makes `poll` a non-blocking check that returns immediately
with the socket's current readiness. A positive value waits up to that long. A
negative `timeoutMs` is rejected with `ErrInvalidArgument` rather than being
treated as "wait forever". Because the host `poll` takes a C `int`, a value above
2147483647 is clamped to that, which is roughly 24 days.
[[src/target/shared/code/builder_values.rs:net_connect_is_address_form]]
[[src/target/shared/code/net/poll.rs:lower_net_poll_helper]]

`net::poll` is a single-socket readiness test. The `poll(List OF Socket)` overload
described in the language specification is deliberately **not** implemented: the
ownership model forbids resource handles as collection elements, so a
`List OF Socket` value cannot be constructed and the overload would be
unreachable. Poll each socket individually.
[[src/builtins/net.rs:resolve_call]]

A signal that interrupts the underlying wait re-issues it rather than surfacing a
failure. `net::poll` complements `net::setReadTimeout`: `poll` asks whether a read
would block right now, while `setReadTimeout` bounds how long a read that does
block may wait. [[src/target/shared/code/net/poll.rs:lower_net_poll_helper]]

## Overloads

**`net::poll(sock AS Socket) AS Boolean`**

Checks readiness immediately and returns without waiting — the omitted
`timeoutMs` is filled with `0`.

**`net::poll(sock AS Socket, timeoutMs AS Integer) AS Boolean`**

Waits at most `timeoutMs` milliseconds for the socket to become readable.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `Socket` | An open connected socket, as returned by `net::connectTcp` or `net::accept`. It is borrowed and inspected for readiness only; no data is read and the handle is not consumed. [[src/builtins/net.rs:call_param_names]] |
| `timeoutMs` | `Integer` | Optional, defaulting to `0` for an immediate non-blocking check. A positive value waits up to that many milliseconds, clamped to `2147483647`. Must not be negative. [[src/target/shared/code/net/poll.rs:lower_net_poll_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when the socket is readable — a following `net::read` or `net::readText` will not block, including when that read would report end of stream. `FALSE` when nothing became readable before the deadline. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `timeoutMs` is negative. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed, or the underlying readiness check fails for a reason other than an interruption. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |

## Examples

Check whether data is waiting, without blocking:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  io::print(toString(net::poll(conn)))
  RETURN 0
END FUNC
```

Wait up to a second for a peer to send something:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  net::writeText(client, "hi")
  IF net::poll(conn, 1000) THEN
    io::print(net::readText(conn, 16))
  END IF
  RETURN 0
END FUNC
```

## See also

- `mfb man net read`
- `mfb man net readText`
- `mfb man net setReadTimeout`
- `mfb man net accept`
- `mfb man net connectTcp`
- `mfb man net close`
