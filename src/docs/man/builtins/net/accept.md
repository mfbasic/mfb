# accept

Accept the next pending connection on a TCP listener.

## Synopsis

```
net::accept(listener AS net::Listener) AS net::Socket
net::accept(listener AS net::Listener, timeoutMs AS Integer) AS net::Socket
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

`net::accept` removes the next pending connection from a `Listener`'s queue and
returns a connected `Socket` for talking to that client. The listener must have
been placed in the listening state by `net::listenTcp` and must still be open.
Each call accepts a single connection, so a server loops over `accept` to serve
clients as they arrive. The listener is *borrowed*, not consumed: it stays open
and usable for further accepts. [[src/syntaxcheck/builtins.rs:net_consumes_argument]]

The optional `timeoutMs` follows the language timeout convention (see
`mfb spec language builtin-functions` → "Timeout convention"). When it is
**omitted the call blocks** indefinitely until a client connects. `0` is one
immediate attempt: it returns a pending connection if one is already queued,
otherwise it raises `ErrTimeout` without waiting. A positive `timeoutMs` polls
the listener against that deadline (clamped to `2147483647`) and raises
`ErrTimeout` if no client arrives first. A negative `timeoutMs` raises
`ErrInvalidArgument`.
[[src/target/shared/code/builder_values.rs:net_connect_is_address_form]]
[[src/codegen/builtins/net/native/io.rs:lower_net_accept_helper]]

On the bounded path the listener is temporarily switched into non-blocking mode
for the duration of the call and its original file-status flags are restored
before the call returns, on every exit path. This matters when a connection that
the readiness poll saw is aborted by the peer, or is taken by another thread,
between the poll and the accept: the accept then reports `EAGAIN` and the call
re-enters the poll rather than blocking for the *next* client and overrunning
`timeoutMs`. A signal that interrupts either the poll or the accept re-issues the
same call instead of surfacing a spurious failure.
[[src/codegen/builtins/net/native/io.rs:emit_listener_flags_restore]]

The returned `Socket` is a fully independent resource: it stays usable after the
listener is closed, and closing it does not affect the listener. Like every
`net` handle it is closed by lexical drop when its binding leaves scope, or
earlier with `net::close`. Read and write it with `net::read`, `net::readText`,
`net::write`, and `net::writeText`, and inspect its endpoints with
`net::localAddress` and `net::remoteAddress`.
[[src/codegen/builtins/net/mod.rs:close_function]]

## Overloads

**`net::accept(listener AS net::Listener) AS net::Socket`**

Blocks until a client connects and returns the connected `Socket` (omitted
`timeoutMs` = unbounded wait).

**`net::accept(listener AS net::Listener, timeoutMs AS Integer) AS net::Socket`**

Waits at most `timeoutMs` milliseconds for a pending connection and raises
`ErrTimeout` if none arrives. `0` is one immediate attempt (`ErrTimeout` when no
connection is already pending); a negative value raises `ErrInvalidArgument`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `listener` | `Listener` | An open listener in the listening state, as returned by `net::listenTcp`. It is borrowed, not consumed, and remains available for further `accept` calls. [[src/codegen/builtins/net/mod.rs:aliases]] |
| `timeoutMs` | `Integer` | Optional. The maximum time to wait for a pending connection, in milliseconds. Omit to block indefinitely; `0` is one immediate attempt (`ErrTimeout` if none pending); a positive value that elapses raises `ErrTimeout` (clamped to `2147483647`); a negative value raises `ErrInvalidArgument`. [[src/codegen/builtins/net/native/io.rs:lower_net_accept_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `Socket` | A connected socket for communicating with the accepted client. It is independent of the listener and is closed by lexical drop at scope exit unless closed earlier with `net::close`. [[src/codegen/builtins/net/mod.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050008` | `ErrTimeout` | No connection arrived before the deadline: immediately when `timeoutMs` is `0` and none is pending, or after a positive `timeoutMs` elapsed. The omitted (unbounded) form never raises this. [[src/codegen/builtins/errorcode/mod.rs:ErrTimeout]] |
| `77050002` | `ErrInvalidArgument` | `timeoutMs` is negative. [[src/codegen/builtins/errorcode/mod.rs:ErrInvalidArgument]] |
| `77070003` | `ErrNetworkFailed` | The underlying `accept` or readiness `poll` fails for a host reason other than an interruption, an `EAGAIN` re-poll, or the deadline. [[src/codegen/builtins/errorcode/mod.rs:ErrNetworkFailed]] |
| `77030004` | `ErrResourceClosed` | `listener` has already been closed. [[src/codegen/builtins/errorcode/mod.rs:ErrResourceClosed]] |
| `77010001` | `ErrOutOfMemory` | The `Socket` handle record for the accepted connection could not be allocated. [[src/codegen/builtins/errorcode/mod.rs:ErrOutOfMemory]] |

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
- `mfb spec language builtin-functions` — the timeout convention
