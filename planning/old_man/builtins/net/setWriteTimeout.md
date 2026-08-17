# setWriteTimeout

Bound how long a send on a socket may block.

## Synopsis

```
net::setWriteTimeout(sock AS net::Socket, timeoutMs AS Integer) AS Nothing
net::setWriteTimeout(sock AS net::UdpSocket, timeoutMs AS Integer) AS Nothing
```

## Package

`net`

## Imports

```
IMPORT net
```

`net` is a built-in package, so no manifest dependency is required.
[[src/codegen/registry/mod.rs:owning_package]]

## Description

`net::setWriteTimeout` sets the maximum time, in milliseconds, that a send on
`sock` may block waiting for the host's send buffer to accept data. It applies to
a connected TCP `Socket` or a bound UDP `UdpSocket` and takes effect on every
subsequent send: `net::write` and `net::writeText` for a `Socket`, and
`net::sendTo` and `net::sendTextTo` for a `UdpSocket`. The socket is borrowed and
stays open. [[src/codegen/builtins/net/mod.rs:register]]

The millisecond value is converted into a whole-seconds and microseconds pair and
installed as the socket's send-timeout option; the conversion is exact integer
division, so the value is used as given.
[[src/codegen/builtins/net/native/poll.rs:lower_net_set_timeout_helper]]

When the timeout elapses before the send can make progress, the pending send
fails with `ErrTimeout` rather than blocking further. It bounds a single
underlying send. That distinction matters for `net::write` and `net::writeText`,
which loop until the whole payload has been handed over: each iteration is
separately bounded, and a timeout in the middle of that loop raises
`ErrTimeout` after part of the payload has already been sent. A partially
written stream cannot be resumed from the error, so treat it as fatal to that
connection.

Per the language timeout convention (see `mfb spec language builtin-functions` →
"Timeout convention"), a `timeoutMs` of `0` makes subsequent sends
**non-blocking**: a send that cannot make progress fails at once with `ErrTimeout`
rather than waiting for buffer space. A positive value bounds the wait. A negative
`timeoutMs` is rejected with `ErrInvalidArgument`. The socket's *initial* state is
unbounded (a send blocks until buffer space frees); the setter can only bound, so
unbounded cannot be re-established through it once a bound is set.
[[src/codegen/builtins/net/native/poll.rs:lower_net_set_timeout_helper]]

## Overloads

**`net::setWriteTimeout(sock AS net::Socket, timeoutMs AS Integer) AS Nothing`**

Bounds `net::write` and `net::writeText` on a connected TCP socket.

**`net::setWriteTimeout(sock AS net::UdpSocket, timeoutMs AS Integer) AS Nothing`**

Bounds `net::sendTo` and `net::sendTextTo` on a bound UDP socket.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `Socket` or `UdpSocket` | The open connected TCP socket or bound UDP socket whose subsequent sends are to be bounded. The handle is borrowed, not consumed. [[src/codegen/builtins/net/mod.rs:aliases]] |
| `timeoutMs` | `Integer` | The maximum time a subsequent send may block waiting for buffer space, in milliseconds. `0` makes sends non-blocking (immediate `ErrTimeout` when no progress can be made); a positive value bounds the wait. Must not be negative. [[src/codegen/builtins/net/native/poll.rs:lower_net_set_timeout_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `setWriteTimeout` returns no value. On a successful return the timeout has been installed on `sock` and applies to every subsequent send. [[src/codegen/builtins/net/mod.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `timeoutMs` is negative. [[src/codegen/builtins/errorcode/mod.rs:ErrInvalidArgument]] |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed, or the host refuses to install the option — which it does when the descriptor is no longer a usable socket. [[src/codegen/builtins/errorcode/mod.rs:ErrResourceClosed]] |

## Examples

Fail a TCP write that stalls for more than two seconds:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  net::setWriteTimeout(client, 2000)
  net::writeText(client, "hello")
  io::print("sent")
  RETURN 0
END FUNC
```

Bound a UDP send so a full buffer does not block forever:

```
IMPORT collections
IMPORT net
IMPORT io

FUNC main AS Integer
  RES sock = net::bindUdp("127.0.0.1", 0)
  net::setWriteTimeout(sock, 1000)
  LET dest = collections::get(net::lookup("127.0.0.1", 9000), 0)
  net::sendTextTo(sock, dest, "ping")
  io::print("sent")
  RETURN 0
END FUNC
```

## See also

- `mfb man net setReadTimeout`
- `mfb man net write`
- `mfb man net writeText`
- `mfb man net sendTo`
- `mfb man net sendTextTo`
- `mfb man net connectTcp`
- `mfb man net bindUdp`
- `mfb spec language builtin-functions` — the timeout convention
