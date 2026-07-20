# write

Write bytes to a connected socket.

## Synopsis

```
net::write(sock AS Socket, bytes AS List OF Byte) AS Nothing
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

`net::write` sends the raw bytes of `bytes` over a connected `Socket`. It writes
the *entire* list before returning: the call loops, advancing a cursor past
whatever each underlying host write accepted and re-issuing the write for the
remainder, so a short host write is resumed rather than mistaken for completion.
When the call returns successfully, every byte has been handed to the socket's
send buffer — which is not a guarantee that the peer has received or read them.
[[src/target/shared/code/net/io.rs:lower_net_write_helper]]

The bytes are read directly out of the list's inline data region and sent in list
order, with no copy, re-encoding, or newline translation. An empty list writes
nothing and returns immediately, because the loop's remaining-byte count starts
at zero. The socket is borrowed and stays open.
[[src/target/shared/code/net/io.rs:lower_net_write_helper]]

Otherwise the call blocks while the send buffer is full, waiting for space or for
the socket's write timeout to elapse. Use `net::setWriteTimeout` to bound that
wait; when it elapses the call raises `ErrWriteTimeout`, and because the write
loop may already have handed over part of the payload, a timeout can leave the
stream partially written and unresumable. A signal that interrupts a blocking
write re-issues it from the unchanged cursor rather than reporting a closed
connection. [[src/target/shared/code/net/io.rs:lower_net_write_helper]]

Use `net::writeText` instead when sending UTF-8 text from a `String` is more
convenient than building a `List OF Byte`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `Socket` | A connected socket to send on, as returned by `net::connectTcp` or `net::accept`. It must still be open; the handle is borrowed, not consumed. [[src/builtins/net.rs:call_param_names]] |
| `bytes` | `List OF Byte` | The payload, sent in list order. An empty list writes nothing and returns immediately. [[src/builtins/net.rs:argument_types]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `write` returns no value. On a successful return every byte of `bytes` has been handed to the socket's send buffer. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77070004` | `ErrConnectionClosed` | The peer has closed the connection, or the write fails for a host reason other than a timeout or an interruption. [[src/target/shared/code/error_constants.rs:ERR_CONNECTION_CLOSED_CODE]] |
| `77070006` | `ErrWriteTimeout` | The socket's write timeout elapsed before the payload could be handed over. Part of it may already have been sent. [[src/target/shared/code/error_constants.rs:ERR_WRITE_TIMEOUT_CODE]] |

## Examples

Send a payload as raw bytes and read the reply:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  LET payload AS List OF Byte = [104, 101, 108, 108, 111]
  net::write(client, payload)
  io::print(toString(len(net::read(conn, 16))))
  RETURN 0
END FUNC
```

Echo one chunk back to the peer that sent it:

```
IMPORT net

FUNC echoOnce(RES peer AS Socket) AS Integer
  LET chunk = net::read(peer, 4096)
  net::write(peer, chunk)
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

- `mfb man net writeText`
- `mfb man net read`
- `mfb man net setWriteTimeout`
- `mfb man net connectTcp`
- `mfb man net accept`
- `mfb man net close`
