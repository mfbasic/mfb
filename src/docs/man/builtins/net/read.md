# read

Read available bytes from a connected socket.

## Synopsis

```
net::read(sock AS Socket, maxBytes AS Integer) AS List OF Byte
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

`net::read` receives data from a connected `Socket` and returns it as a
`List OF Byte`. A single call performs one underlying receive: it returns as soon
as any data is available rather than waiting to fill the requested size, so the
returned list is frequently shorter than `maxBytes`. On success it always holds
at least one byte. The socket is borrowed and stays open.
[[src/target/shared/code/net/io.rs:lower_net_read_helper]]

The call blocks until at least one byte arrives, the peer closes its side, or the
socket's read timeout elapses. `maxBytes` bounds a single read and the length of
the result; it does not request that exactly that many bytes be read, and it must
be positive. Internally the temporary receive buffer is capped at 1 MiB even when
`maxBytes` is larger, so a very large `maxBytes` does not pre-commit that much
memory for a read that delivers far fewer bytes. Because a single host receive
never returns more than the socket's receive buffer, this cap is invisible to the
one-receive semantics above. [[src/target/shared/code/net/io.rs:lower_net_read_helper]]

Unlike a plain stream read that signals end of stream with a zero-length result,
`net::read` raises an error when the peer has closed: there is no empty-list
sentinel, and a successful result never has length `0`. To consume a whole
message, call `read` in a loop, appending each result, and stop when
`ErrConnectionClosed` is raised. Use `net::poll` to test for readiness without
blocking and `net::setReadTimeout` to bound how long a read may wait; a timeout
that elapses raises `ErrReadTimeout`, which is distinguished from a closed
connection by the host reporting `EAGAIN`.
[[src/target/shared/code/net/io.rs:lower_net_read_helper]]

A signal that interrupts the blocking receive re-issues the identical read rather
than misreporting it as a closed connection. The bytes are copied into a freshly
allocated `List OF Byte`; the call has no other side effects and does not close
the socket. Use `net::readText` when the peer sends UTF-8 text and a `String` is
more convenient than raw bytes.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `Socket` | A connected socket to receive from, as returned by `net::connectTcp` or `net::accept`. It must still be open; the handle is borrowed, not consumed. [[src/builtins/net.rs:call_param_names]] |
| `maxBytes` | `Integer` | The maximum number of bytes to read in this call. Must be positive. It caps the length of the returned list but does not guarantee that many bytes arrive. [[src/target/shared/code/net/io.rs:lower_net_read_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The bytes received in this read, in arrival order, with length between `1` and `maxBytes` inclusive. End of stream is reported as `ErrConnectionClosed`, never as an empty list. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `maxBytes` is not positive. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77070004` | `ErrConnectionClosed` | The peer has closed the connection (a zero-length receive), or the receive fails for a host reason other than a timeout or an interruption. [[src/target/shared/code/error_constants.rs:ERR_CONNECTION_CLOSED_CODE]] |
| `77070005` | `ErrReadTimeout` | The socket's read timeout elapsed before any data arrived. [[src/target/shared/code/error_constants.rs:ERR_READ_TIMEOUT_CODE]] |
| `77010001` | `ErrOutOfMemory` | The temporary read buffer or the returned `List OF Byte` could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Read up to 16 bytes from a connected socket:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  net::writeText(client, "abc")
  LET raw = net::read(conn, 16)
  io::print(toString(len(raw)))
  RETURN 0
END FUNC
```

Drain a connection until the peer closes it:

```
IMPORT net

FUNC drain(RES sock AS Socket) AS Integer
  MUT total AS Integer = 0
  MUT reading AS Boolean = TRUE
  WHILE reading
    LET chunk = net::read(sock, 4096)
    total = total + len(chunk)
  END WHILE
  RETURN total
  TRAP(e)
    RETURN total
  END TRAP
END FUNC

SUB main()
  ' Sums the bytes received until the peer closes; the function-level TRAP catches
  ' the ErrConnectionClosed that ends the stream and returns the running total.
END SUB
```

## See also

- `mfb man net readText`
- `mfb man net write`
- `mfb man net poll`
- `mfb man net setReadTimeout`
- `mfb man net connectTcp`
- `mfb man net accept`
- `mfb man net close`
