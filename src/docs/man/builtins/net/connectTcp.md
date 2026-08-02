# connectTcp

Open a TCP connection to a host and port or to a resolved address.

## Synopsis

```
net::connectTcp(host AS String, port AS Integer) AS Socket
net::connectTcp(host AS String, port AS Integer, timeoutMs AS Integer) AS Socket
net::connectTcp(address AS Address) AS Socket
net::connectTcp(address AS Address, timeoutMs AS Integer) AS Socket
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

`net::connectTcp` establishes an outbound TCP connection and returns a connected
`Socket`. The peer is named either by a host string plus a port, or by an
`Address` record whose `host` and `port` fields supply both. When `host` is a
name rather than a textual IP address it is resolved with the host resolver
before connecting, and the first resolved result is used; the requested port is
written into that address rather than being resolved as a service name.
[[src/target/shared/code/net/mod.rs:lower_net_endpoint_helper]]

Every connect takes the same non-blocking-connect plus readiness-poll path. The
socket is switched to non-blocking mode, `connect` is issued, and the call then
polls for writability against a deadline; on success the original blocking mode
is restored and the socket's `SO_ERROR` is checked before the handle is built, so
a connection that failed asynchronously is reported as a failure rather than
handed back as connected. A signal that interrupts the poll re-issues it instead
of surfacing a spurious error.
[[src/target/shared/code/net/mod.rs:lower_net_endpoint_helper]]

`timeoutMs` selects that deadline, following the language timeout convention (see
`mfb spec language builtin-functions` → "Timeout convention"). When it is
**omitted the connect blocks** until the connection completes or the OS refuses it
(there is no built-in bounded default any more). `0` is one immediate,
non-blocking attempt: it succeeds if the connect completes at once, otherwise it
raises `ErrTimeout` without waiting. A positive value bounds the attempt and
raises `ErrTimeout` on the deadline. A negative `timeoutMs` raises
`ErrInvalidArgument`. On any failure the pending descriptor and the resolver
results are released first. Because `poll` takes a C `int`, a deadline above
2147483647 milliseconds is clamped to that value. **A caller that must not wedge
on a black-holed peer must pass a positive `timeoutMs`** — the former 120000 ms
safety default is gone.
[[src/target/shared/code/net/mod.rs:lower_net_endpoint_helper]]
[[src/target/shared/code/net/mod.rs:lower_net_endpoint_helper]]

The four overloads do not share a positional layout: `timeoutMs` is parameter 2
of the host/port forms but parameter 1 of the `Address` forms. Named arguments
therefore bind per-overload, against the parameter list of whichever overload the
argument types select. [[src/builtins/net.rs:call_param_name_overloads]]

The returned `Socket` is an owned, non-copyable resource handle, closed by
lexical drop when its binding leaves scope or earlier with `net::close`. Read
and write it with `net::read`, `net::readText`, `net::write`, and
`net::writeText`, bound its blocking with `net::setReadTimeout` and
`net::setWriteTimeout`, and inspect its endpoints with `net::localAddress` and
`net::remoteAddress`. [[src/builtins/net.rs:resource_close_function]]

## Overloads

**`net::connectTcp(host AS String, port AS Integer) AS Socket`**

Resolves `host` and connects on `port`, blocking until the connection completes
or the OS refuses it (omitted `timeoutMs` = unbounded).

**`net::connectTcp(host AS String, port AS Integer, timeoutMs AS Integer) AS Socket`**

Resolves `host` and connects on `port`, failing with `ErrTimeout` if the attempt
does not complete within `timeoutMs`.

**`net::connectTcp(address AS Address) AS Socket`**

Connects to the `host` and `port` carried by `address`, blocking until the
connection completes or the OS refuses it (omitted `timeoutMs` = unbounded). This is the form for an `Address` obtained from `net::lookup`.

**`net::connectTcp(address AS Address, timeoutMs AS Integer) AS Socket`**

Connects to the `host` and `port` carried by `address`, failing with `ErrTimeout`
if the attempt exceeds `timeoutMs`. Here `timeoutMs` is parameter 1, not 2.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `host` | `String` | The peer's host name or textual IP address. Passed to the host resolver; a name with no address record raises an error. [[src/builtins/net.rs:call_param_name_overloads]] |
| `port` | `Integer` | The TCP port to connect to on the peer. Written directly into the resolved address. [[src/target/shared/code/net/mod.rs:lower_net_endpoint_helper]] |
| `address` | `Address` | A destination record supplying both the peer host and the peer port, typically from `net::lookup`. Replaces the separate `host` and `port` arguments. [[src/builtins/net.rs:NET]] |
| `timeoutMs` | `Integer` | Optional. The maximum time the connection attempt may take, in milliseconds. Omit to block until the connection resolves; `0` is one immediate attempt (`ErrTimeout` unless it completes at once); a positive value bounds the attempt and raises `ErrTimeout` when it elapses (clamped to `2147483647`); a negative value raises `ErrInvalidArgument`. [[src/target/shared/code/net/mod.rs:lower_net_endpoint_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `Socket` | A connected socket ready for reading and writing. Closed by lexical drop at scope exit unless closed earlier with `net::close`. [[src/builtins/net.rs:NET]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77070002` | `ErrAddressNotFound` | The host could not be resolved — it is malformed, or it has no address record. [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_NOT_FOUND_CODE]] |
| `77050008` | `ErrTimeout` | The connection did not complete before its deadline: immediately when `timeoutMs` is `0` and the connect is not instant, or after a positive `timeoutMs` elapsed. The omitted (unbounded) form never raises this. [[src/target/shared/code/error_constants.rs:ERR_TIMEOUT_CODE]] |
| `77050002` | `ErrInvalidArgument` | `timeoutMs` is negative. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77070003` | `ErrNetworkFailed` | The socket could not be created, or the connection failed outright — the peer refused it, the network is unreachable, or the readiness poll failed for a reason other than an interruption. [[src/target/shared/code/error_constants.rs:ERR_NETWORK_FAILED_CODE]] |
| `77010001` | `ErrOutOfMemory` | The NUL-terminated copy of the host or the `Socket` handle record could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Connect to a local listener by host and port:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  io::print(toString(net::remoteAddress(client).port))
  RETURN 0
END FUNC
```

Connect to a resolved `Address` with an explicit deadline:

```
IMPORT collections
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  LET dest = collections::get(net::lookup("127.0.0.1", bound.port), 0)
  RES client = net::connectTcp(dest, 5000)
  io::print("connected")
  RETURN 0
END FUNC
```

## See also

- `mfb man net lookup`
- `mfb man net listenTcp`
- `mfb man net accept`
- `mfb man net read`
- `mfb man net write`
- `mfb man net remoteAddress`
- `mfb man net close`
- `mfb spec language builtin-functions` — the timeout convention
