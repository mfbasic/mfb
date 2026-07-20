# setWriteTimeout

Bound how long a send on a socket may block.

## Synopsis

```
net::setWriteTimeout(sock AS Socket, timeoutMs AS Integer) AS Nothing
net::setWriteTimeout(sock AS UdpSocket, timeoutMs AS Integer) AS Nothing
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

`net::setWriteTimeout` sets the maximum time, in milliseconds, that a send on
`sock` may block waiting for the host's send buffer to accept data. It applies to
a connected TCP `Socket` or a bound UDP `UdpSocket` and takes effect on every
subsequent send: `net::write` and `net::writeText` for a `Socket`, and
`net::sendTo` and `net::sendTextTo` for a `UdpSocket`. The socket is borrowed and
stays open. [[src/builtins/net.rs:resolve_call]]

The millisecond value is converted into a whole-seconds and microseconds pair and
installed as the socket's send-timeout option; the conversion is exact integer
division, so the value is used as given.
[[src/target/shared/code/net/poll.rs:lower_net_set_timeout_helper]]

When the timeout elapses before the send can make progress, the pending send
fails with `ErrWriteTimeout` rather than blocking further. It bounds a single
underlying send. That distinction matters for `net::write` and `net::writeText`,
which loop until the whole payload has been handed over: each iteration is
separately bounded, and a timeout in the middle of that loop raises
`ErrWriteTimeout` after part of the payload has already been sent. A partially
written stream cannot be resumed from the error, so treat it as fatal to that
connection.

A `timeoutMs` of `0` disables the timeout, so sends block indefinitely until the
data is accepted, the peer closes, or an error occurs. That is also the state of
a freshly opened socket, so `net::setWriteTimeout(sock, 0)` restores the default.
A negative `timeoutMs` is rejected with `ErrInvalidArgument` rather than being
treated as "no timeout".
[[src/target/shared/code/net/poll.rs:lower_net_set_timeout_helper]]

## Overloads

**`net::setWriteTimeout(sock AS Socket, timeoutMs AS Integer) AS Nothing`**

Bounds `net::write` and `net::writeText` on a connected TCP socket.

**`net::setWriteTimeout(sock AS UdpSocket, timeoutMs AS Integer) AS Nothing`**

Bounds `net::sendTo` and `net::sendTextTo` on a bound UDP socket.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `Socket` or `UdpSocket` | The open connected TCP socket or bound UDP socket whose subsequent sends are to be bounded. The handle is borrowed, not consumed. [[src/builtins/net.rs:call_param_names]] |
| `timeoutMs` | `Integer` | The maximum time a subsequent send may block waiting for buffer space, in milliseconds. `0` disables the timeout, which is the default state of a freshly opened socket. Must not be negative. [[src/target/shared/code/net/poll.rs:lower_net_set_timeout_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `setWriteTimeout` returns no value. On a successful return the timeout has been installed on `sock` and applies to every subsequent send. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `timeoutMs` is negative. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed, or the host refuses to install the option — which it does when the descriptor is no longer a usable socket. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |

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
