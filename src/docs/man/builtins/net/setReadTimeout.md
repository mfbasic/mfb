# setReadTimeout

Bound how long a receive on a socket may block.

## Synopsis

```
net::setReadTimeout(sock AS Socket, timeoutMs AS Integer) AS Nothing
net::setReadTimeout(sock AS UdpSocket, timeoutMs AS Integer) AS Nothing
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

`net::setReadTimeout` sets the maximum time, in milliseconds, that a receive on
`sock` may block waiting for data. It applies to a connected TCP `Socket` or a
bound UDP `UdpSocket` and takes effect on every subsequent receive: `net::read`
and `net::readText` for a `Socket`, and `net::receiveFrom` and
`net::receiveTextFrom` for a `UdpSocket`. The socket is borrowed and stays open.
[[src/builtins/net.rs:resolve_call]]

The millisecond value is converted into a whole-seconds and microseconds pair and
installed as the socket's receive-timeout option. Because the conversion is exact
integer division, a `timeoutMs` under one millisecond of resolution is not
rounded up — the value is used as given.
[[src/target/shared/code/net/poll.rs:lower_net_set_timeout_helper]]

When the timeout elapses before any data arrives, the pending receive fails with
`ErrReadTimeout` rather than blocking further. The timeout governs only how long
a *single* receive waits for its first data; it does not cap the total time a
loop of receives may take, and it does not abort a receive that has already
started delivering bytes.

A `timeoutMs` of `0` disables the timeout, so receives block indefinitely until
data arrives, the peer closes, or an error occurs. That is also the state of a
freshly opened socket, so `net::setReadTimeout(sock, 0)` restores the default. A
negative `timeoutMs` is rejected with `ErrInvalidArgument` rather than being
treated as "no timeout". [[src/target/shared/code/net/poll.rs:lower_net_set_timeout_helper]]

`net::setReadTimeout` bounds a blocking receive; `net::poll` instead asks whether
a receive would block at all. They compose: poll for readiness, and keep a
timeout installed as a backstop.

## Overloads

**`net::setReadTimeout(sock AS Socket, timeoutMs AS Integer) AS Nothing`**

Bounds `net::read` and `net::readText` on a connected TCP socket.

**`net::setReadTimeout(sock AS UdpSocket, timeoutMs AS Integer) AS Nothing`**

Bounds `net::receiveFrom` and `net::receiveTextFrom` on a bound UDP socket.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `Socket` or `UdpSocket` | The open connected TCP socket or bound UDP socket whose subsequent receives are to be bounded. The handle is borrowed, not consumed. [[src/builtins/net.rs:call_param_names]] |
| `timeoutMs` | `Integer` | The maximum time a subsequent receive may block waiting for data, in milliseconds. `0` disables the timeout, which is the default state of a freshly opened socket. Must not be negative. [[src/target/shared/code/net/poll.rs:lower_net_set_timeout_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `setReadTimeout` returns no value. On a successful return the timeout has been installed on `sock` and applies to every subsequent receive. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `timeoutMs` is negative. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed, or the host refuses to install the option — which it does when the descriptor is no longer a usable socket. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |

## Examples

Fail a TCP read that stalls for more than two seconds:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  net::setReadTimeout(client, 2000)
  io::print("armed")
  RETURN 0
END FUNC
```

Bound a UDP receive so a missing reply does not block forever:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES sock = net::bindUdp("127.0.0.1", 0)
  net::setReadTimeout(sock, 1000)
  LET dg = net::receiveTextFrom(sock, 512)
  io::print(dg.value)
  RETURN 0
  TRAP(e)
    io::print(toString(e.code))
    RETURN 0
  END TRAP
END FUNC
```

## See also

- `mfb man net setWriteTimeout`
- `mfb man net read`
- `mfb man net readText`
- `mfb man net receiveFrom`
- `mfb man net receiveTextFrom`
- `mfb man net poll`
- `mfb man net connectTcp`
