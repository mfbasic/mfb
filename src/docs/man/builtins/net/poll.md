# poll

Test whether a socket has data ready to read, or wait for the first ready socket
among many.

## Synopsis

```
net::poll(sock AS Socket) AS Boolean
net::poll(sock AS Socket, timeoutMs AS Integer) AS Boolean
net::poll(socks AS List OF RES Socket) AS Socket
net::poll(socks AS List OF RES Socket, timeoutMs AS Integer) AS Socket
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

`timeoutMs` bounds the wait, in milliseconds, following the language timeout
convention (see `mfb spec language builtin-functions` → "Timeout convention").
When it is **omitted, `poll` blocks** until the socket becomes readable and then
returns `TRUE` (omit = unbounded). `0` is a non-blocking check that returns
immediately with the socket's current readiness (the old omitted behavior — pass
`, 0` for it). A positive value waits up to that long. A negative `timeoutMs` is
rejected with `ErrInvalidArgument`. Because the host `poll` takes a C `int`, a
value above 2147483647 is clamped to that, which is roughly 24 days.
[[src/target/shared/code/builder_values.rs:net_poll_is_list_form]]
[[src/target/shared/code/net/poll.rs:lower_net_poll_helper]]

Given a `List OF RES Socket`, `net::poll` becomes a **readiness multiplex**: it
blocks until at least one socket in the list is readable, then returns the first
ready one (lowest list index). The returned `Socket` is a **borrowed** pointer —
an alias of a list element, exactly like `collections::get` — so the list retains
ownership and closes every socket exactly once on scope exit; you may read,
`return`, or `thread::transfer` through the returned handle, but you do not close
it. An empty list is rejected with `ErrInvalidArgument`. Because the multiplex
yields a resource and has no not-ready value to return, expiry raises `ErrTimeout`
rather than returning a sentinel (it is a producing call). The elements must be
marked `RES` (`List OF RES Socket`); a bare `List OF Socket` is a compile error,
as resource elements always require the `RES` marker.
[[src/target/shared/code/net/poll.rs:lower_net_poll_list_helper]]

A signal that interrupts the underlying wait re-issues it rather than surfacing a
failure. `net::poll` complements `net::setReadTimeout`: `poll` asks whether a read
would block right now, while `setReadTimeout` bounds how long a read that does
block may wait. [[src/target/shared/code/net/poll.rs:lower_net_poll_helper]]

## Overloads

**`net::poll(sock AS Socket) AS Boolean`**

Blocks until the socket becomes readable, then returns `TRUE` (omitted
`timeoutMs` = unbounded wait). For the old immediate check, pass `, 0`.

**`net::poll(sock AS Socket, timeoutMs AS Integer) AS Boolean`**

Waits at most `timeoutMs` milliseconds for the socket to become readable; `0` is
a non-blocking check.

**`net::poll(socks AS List OF RES Socket) AS Socket`**

Blocks until one socket in `socks` is readable, then returns a borrowed pointer to
the first ready one (omitted `timeoutMs` = unbounded wait). The list still owns and
closes every socket.

**`net::poll(socks AS List OF RES Socket, timeoutMs AS Integer) AS Socket`**

Waits at most `timeoutMs` milliseconds for one socket to become readable; `0` is a
single immediate scan. Expiry with none ready raises `ErrTimeout`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `Socket` | An open connected socket, as returned by `net::connectTcp` or `net::accept`. It is borrowed and inspected for readiness only; no data is read and the handle is not consumed. [[src/builtins/net.rs:call_param_name_overloads]] |
| `socks` | `List OF RES Socket` | A non-empty list of open connected sockets. Each is borrowed and inspected for readiness; the list keeps ownership. An empty list raises `ErrInvalidArgument`. [[src/builtins/net.rs:call_param_name_overloads]] |
| `timeoutMs` | `Integer` | Optional. Omit to block until a socket is readable; `0` is an immediate non-blocking check/scan; a positive value waits up to that many milliseconds, clamped to `2147483647`. Must not be negative. [[src/target/shared/code/net/poll.rs:lower_net_poll_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | (scalar overload) `TRUE` when the socket is readable — a following `net::read` or `net::readText` will not block, including when that read would report end of stream. `FALSE` when nothing became readable before the deadline. [[src/builtins/net.rs:NET]] |
| `Socket` | (list overload) A **borrowed** pointer to the first ready socket (lowest list index). The list retains ownership and closes it; do not close the returned handle. [[src/target/shared/code/net/poll.rs:lower_net_poll_list_helper]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `timeoutMs` is negative, or (list overload) `socks` is empty. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77050008` | `ErrTimeout` | (list overload) the timeout expires with no socket ready. The scalar overload returns `FALSE` instead. [[src/target/shared/code/error_constants.rs:ERR_TIMEOUT_CODE]] |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed, or the underlying readiness check fails for a reason other than an interruption. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |

## Examples

Check whether data is waiting, without blocking (pass `0` for the immediate
check — omitting the timeout would instead block until the socket is readable):

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  io::print(toString(net::poll(conn, 0)))
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

Wait for the first ready socket among several (the readiness multiplex). The
returned socket is borrowed — the list still owns and closes both:

```
IMPORT net
IMPORT io
IMPORT collections

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES clientA = net::connectTcp("127.0.0.1", bound.port)
  RES connA = net::accept(server)
  RES clientB = net::connectTcp("127.0.0.1", bound.port)
  RES connB = net::accept(server)
  MUT socks AS List OF RES Socket = []
  socks = collections::append(socks, connA)
  socks = collections::append(socks, connB)
  net::writeText(clientB, "hi")
  RES ready AS Socket = net::poll(socks)
  io::print(net::readText(ready, 16))
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
- `mfb spec language builtin-functions` — the timeout convention
