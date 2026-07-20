# writeText

Write a String to a connected socket as UTF-8 text.

## Synopsis

```
net::writeText(sock AS Socket, value AS String) AS Nothing
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

`net::writeText` sends the UTF-8 bytes of `value` over a connected `Socket`. The
string's packed byte data is written directly from its buffer: a `String` already
holds well-formed UTF-8, so the bytes go out exactly as held, with no
re-encoding, decoding, or newline translation.
[[src/target/shared/code/net/io.rs:lower_net_write_helper]]

The call writes the *entire* string before returning. It loops, advancing a
cursor past whatever each underlying host write accepted and re-issuing the write
for the remainder, so a short host write is resumed rather than mistaken for
completion. When it returns successfully, every byte of `value` has been handed
to the socket's send buffer — which is not a guarantee that the peer has received
or read them. An empty string writes nothing and returns immediately. The socket
is borrowed and stays open. [[src/target/shared/code/net/io.rs:lower_net_write_helper]]

Otherwise the call blocks while the send buffer is full, waiting for space or for
the socket's write timeout to elapse. Use `net::setWriteTimeout` to bound that
wait; when it elapses the call raises `ErrWriteTimeout`, and because the loop may
already have handed over part of the text, a timeout can leave the stream
partially written and unresumable. A signal that interrupts a blocking write
re-issues it from the unchanged cursor rather than reporting a closed connection.

Use `net::write` instead to send raw binary data from a `List OF Byte` rather
than UTF-8 text from a `String`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `Socket` | A connected socket to send on, as returned by `net::connectTcp` or `net::accept`. It must still be open; the handle is borrowed, not consumed. [[src/builtins/net.rs:call_param_names]] |
| `value` | `String` | The text to send, written as the string's UTF-8 bytes in order. An empty string sends nothing. [[src/builtins/net.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `writeText` returns no value. On a successful return every byte of `value` has been handed to the socket's send buffer. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77070004` | `ErrConnectionClosed` | The peer has closed the connection, or the write fails for a host reason other than a timeout or an interruption. [[src/target/shared/code/error_constants.rs:ERR_CONNECTION_CLOSED_CODE]] |
| `77070006` | `ErrWriteTimeout` | The socket's write timeout elapsed before the whole string could be handed over. Part of it may already have been sent. [[src/target/shared/code/error_constants.rs:ERR_WRITE_TIMEOUT_CODE]] |

## Examples

Send text over a connected socket and read the reply:

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

Echo one chunk of text back to the peer that sent it:

```
IMPORT net

FUNC echoOnce(RES peer AS Socket) AS Integer
  LET chunk = net::readText(peer, 4096)
  net::writeText(peer, chunk)
  RETURN len(chunk)
  TRAP(e)
    RETURN 0
  END TRAP
END FUNC

SUB main()
  ' A single read/echo exchange; the function-level TRAP catches the
  ' ErrConnectionClosed raised once the peer has gone away.
END SUB
```

## See also

- `mfb man net write`
- `mfb man net readText`
- `mfb man net setWriteTimeout`
- `mfb man net connectTcp`
- `mfb man net accept`
- `mfb man net close`
