# accept

Accept the next pending connection on a TCP listener.

## Synopsis

```
net::accept(listener AS Listener) AS Socket
net::accept(listener AS Listener, timeoutMs AS Integer) AS Socket
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

`net::accept` removes the next pending connection from a `Listener`'s queue and
returns a connected `Socket` for talking to that client. The listener must have
been placed in the listening state by `net::listenTcp` and must still be open.
Each call accepts a single connection, so a server loops over `accept` to serve
clients as they arrive. The listener is *borrowed*, not consumed: it stays open
and usable for further accepts. [[src/builtins/net.rs:consumes_argument]]

The optional `timeoutMs` bounds how long the call waits for a client, in
milliseconds. When it is omitted the compiler supplies `0`, and a `timeoutMs` of
zero or less takes the plain blocking path: the call waits indefinitely for a
connection. A positive `timeoutMs` instead polls the listener for readiness
against that deadline and raises `ErrTimeout` if no client arrives first.
[[src/target/shared/code/builder_values.rs:net_connect_is_address_form]]
[[src/target/shared/code/net/io.rs:lower_net_accept_helper]]

On the bounded path the listener is temporarily switched into non-blocking mode
for the duration of the call and its original file-status flags are restored
before the call returns, on every exit path. This matters when a connection that
the readiness poll saw is aborted by the peer, or is taken by another thread,
between the poll and the accept: the accept then reports `EAGAIN` and the call
re-enters the poll rather than blocking for the *next* client and overrunning
`timeoutMs`. A signal that interrupts either the poll or the accept re-issues the
same call instead of surfacing a spurious failure.
[[src/target/shared/code/net/io.rs:emit_listener_flags_restore]]

The returned `Socket` is a fully independent resource: it stays usable after the
listener is closed, and closing it does not affect the listener. Like every
`net` handle it is closed by lexical drop when its binding leaves scope, or
earlier with `net::close`. Read and write it with `net::read`, `net::readText`,
`net::write`, and `net::writeText`, and inspect its endpoints with
`net::localAddress` and `net::remoteAddress`.
[[src/builtins/net.rs:resource_close_function]]

## Overloads

**`net::accept(listener AS Listener) AS Socket`**

Blocks until a client connects and returns the connected `Socket`. The omitted
`timeoutMs` is filled with `0`, which selects the unbounded blocking path.

**`net::accept(listener AS Listener, timeoutMs AS Integer) AS Socket`**

Waits at most `timeoutMs` milliseconds for a pending connection and raises
`ErrTimeout` if none arrives. A zero or negative `timeoutMs` is equivalent to the
one-argument form and blocks indefinitely.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `listener` | `Listener` | An open listener in the listening state, as returned by `net::listenTcp`. It is borrowed, not consumed, and remains available for further `accept` calls. [[src/builtins/net.rs:call_param_names]] |
| `timeoutMs` | `Integer` | Optional. The maximum time to wait for a pending connection, in milliseconds. A positive value that elapses with no connection raises `ErrTimeout`; omitted, zero, or negative blocks indefinitely. [[src/target/shared/code/net/io.rs:lower_net_accept_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `Socket` | A connected socket for communicating with the accepted client. It is independent of the listener and is closed by lexical drop at scope exit unless closed earlier with `net::close`. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050008` | `ErrTimeout` | A positive `timeoutMs` was given and no connection arrived before the deadline elapsed. The unbounded form never raises this. [[src/target/shared/code/error_constants.rs:ERR_TIMEOUT_CODE]] |
| `77070003` | `ErrNetworkFailed` | The underlying `accept` or readiness `poll` fails for a host reason other than an interruption, an `EAGAIN` re-poll, or the deadline. [[src/target/shared/code/error_constants.rs:ERR_NETWORK_FAILED_CODE]] |
| `77030004` | `ErrResourceClosed` | `listener` has already been closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77010001` | `ErrOutOfMemory` | The `Socket` handle record for the accepted connection could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Accept a single client and read a request:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  net::writeText(client, "hello")
  io::print(net::readText(conn, 16))
  RETURN 0
END FUNC
```

Bound how long a server waits for a client:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  RES conn = net::accept(server, 500)
  io::print("accepted")
  RETURN 0
  TRAP(e)
    io::print(toString(e.code))
    RETURN 0
  END TRAP
END FUNC
```

## See also

- `mfb man net listenTcp`
- `mfb man net connectTcp`
- `mfb man net read`
- `mfb man net readText`
- `mfb man net close`
- `mfb man net localAddress`
- `mfb man net remoteAddress`
