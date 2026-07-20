# readText

Read available bytes from a connected socket as UTF-8 text.

## Synopsis

```
net::readText(sock AS Socket, maxBytes AS Integer) AS String
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

`net::readText` receives data from a connected `Socket` and returns it as a
`String`. A single call performs one underlying receive: it returns as soon as
any data is available rather than waiting to fill the requested size, so the
result is frequently shorter than `maxBytes` bytes, and on success it is built
from at least one byte. The socket is borrowed and stays open.
[[src/target/shared/code/net/io.rs:lower_net_read_helper]]

The received bytes are copied into a freshly allocated string and then validated
as UTF-8 before the string is returned; bytes that are not well-formed UTF-8
raise `ErrEncoding`. This is the one way `readText` differs from `net::read`
beyond the result type, and it is also its main hazard: a single receive may split
a multi-byte UTF-8 sequence across two calls, and the call holding the partial
sequence fails validation. When the peer sends raw binary data, or when bytes
must be reassembled across several receives before decoding, use `net::read` and
convert once the message is complete.
[[src/target/shared/code/net/io.rs:lower_net_read_helper]]

The call blocks until at least one byte arrives, the peer closes its side, or the
socket's read timeout elapses. `maxBytes` bounds the bytes read in this call and
must be positive; internally the temporary receive buffer is capped at 1 MiB even
when `maxBytes` is larger, so a very large `maxBytes` does not pre-commit that
much memory. Like `net::read`, end of stream is *not* an empty result: when the
peer has closed, `ErrConnectionClosed` is raised. Read in a loop and stop on that
error. Use `net::poll` to test for readiness without blocking, and
`net::setReadTimeout` to bound how long a read may wait — an elapsed timeout
raises `ErrReadTimeout`.

A signal that interrupts the blocking receive re-issues the identical read rather
than misreporting it as a closed connection. The call has no side effects beyond
receiving and does not close the socket.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `Socket` | A connected socket to receive from, as returned by `net::connectTcp` or `net::accept`. It must still be open; the handle is borrowed, not consumed. [[src/builtins/net.rs:call_param_names]] |
| `maxBytes` | `Integer` | The maximum number of bytes to receive in this call. Must be positive. It caps the bytes received before decoding but does not guarantee that many arrive. [[src/target/shared/code/net/io.rs:lower_net_read_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The bytes received in this read, decoded as UTF-8, in arrival order — built from between `1` and `maxBytes` bytes inclusive. End of stream is reported as `ErrConnectionClosed`, never as an empty string. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `maxBytes` is not positive. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77070004` | `ErrConnectionClosed` | The peer has closed the connection (a zero-length receive), or the receive fails for a host reason other than a timeout or an interruption. [[src/target/shared/code/error_constants.rs:ERR_CONNECTION_CLOSED_CODE]] |
| `77070005` | `ErrReadTimeout` | The socket's read timeout elapsed before any data arrived. [[src/target/shared/code/error_constants.rs:ERR_READ_TIMEOUT_CODE]] |
| `77020004` | `ErrEncoding` | The received bytes are not valid UTF-8 — including the case where a multi-byte sequence was split across two receives. [[src/target/shared/code/error_constants.rs:ERR_ENCODING_CODE]] |
| `77010001` | `ErrOutOfMemory` | The temporary read buffer or the returned `String` could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Read a line of text from a connected socket:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  net::writeText(client, "hello")
  io::print(net::readText(conn, 64))
  RETURN 0
END FUNC
```

Report the error code when a read times out:

```
IMPORT net
IMPORT io

FUNC readOrCode(RES sock AS Socket) AS String
  RETURN net::readText(sock, 64)
  TRAP(e)
    RETURN toString(e.code)
  END TRAP
END FUNC
```

## See also

- `mfb man net read`
- `mfb man net writeText`
- `mfb man net poll`
- `mfb man net setReadTimeout`
- `mfb man net connectTcp`
- `mfb man net accept`
- `mfb man net close`
