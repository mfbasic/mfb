# sendTo

Send a single UDP datagram of bytes to a destination address.

## Synopsis

```
net::sendTo(sock AS net::UdpSocket, address AS Address, bytes AS List OF Byte) AS Nothing
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

`net::sendTo` transmits the contents of `bytes` as one UDP datagram from a bound
`UdpSocket` to the peer named by `address`. Because a UDP socket is not tied to a
single peer, each call names its own destination and the same socket can address
many peers in turn. The socket is borrowed and stays open.
[[src/target/shared/code/net/io.rs:lower_net_send_to_helper]]

`address` supplies both the destination host and the destination port. The host
is resolved with the host resolver on **every** call — it may be a numeric IP
literal or a name — and the `port` field is then written directly into the
resolved address rather than being resolved as a service name. The resolver's
answer chain is released before the call returns, on both the success and the
failure paths. Note the per-call resolution cost: in a tight send loop, resolve
once with `net::lookup` and reuse the resulting `Address`.
[[src/target/shared/code/net/io.rs:lower_net_send_to_helper]]

The whole list is sent as the payload of a single datagram, read directly out of
the list's inline data region in list order. UDP preserves message boundaries:
the payload arrives whole or not at all, and is never split across datagrams or
merged with another. An empty list sends a zero-length datagram, which is a valid
UDP message rather than a no-op.

A successful return means the datagram was accepted by the host for best-effort
delivery, not that any peer received it. The call may block while the send buffer
is full; use `net::setWriteTimeout` to bound that wait, after which
`ErrTimeout` is raised. A payload larger than the path allows is rejected
with `ErrMessageTooLarge` rather than truncated. A signal that interrupts the
send before any byte left re-issues the identical call — a datagram send is
all-or-nothing, so a send that already completed is never retried.
[[src/target/shared/code/net/io.rs:lower_net_send_to_helper]]

Use `net::sendTextTo` instead when sending UTF-8 text from a `String` is more
convenient than building a `List OF Byte`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `UdpSocket` | A bound UDP socket to send from, as returned by `net::bindUdp`. It must still be open; the handle is borrowed, not consumed. [[src/builtins/net.rs:call_param_names]] |
| `address` | `Address` | The destination. Its `host` field is resolved on each call and may be a numeric IP literal or a name; its `port` field selects the destination port. Obtain one from `net::lookup`, or use the `from` field of the `Datagram` returned by `net::receiveFrom` to reply to a sender. [[src/builtins/net.rs:NET]] |
| `bytes` | `List OF Byte` | The payload, sent in list order as one datagram. An empty list sends a valid zero-length datagram. [[src/builtins/net.rs:argument_types]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `sendTo` returns no value. A successful call has handed the datagram to the host for best-effort delivery; it does not guarantee receipt. [[src/builtins/net.rs:NET]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed. [[src/builtins/errorcode.rs:ErrResourceClosed]] |
| `77070002` | `ErrAddressNotFound` | The destination host in `address` could not be resolved. [[src/builtins/errorcode.rs:ErrAddressNotFound]] |
| `77070007` | `ErrMessageTooLarge` | The payload is too large to be sent as a single datagram on this path (the host reports `EMSGSIZE`). [[src/builtins/errorcode.rs:ErrMessageTooLarge]] |
| `77050008` | `ErrTimeout` | The socket's write timeout elapsed before the datagram could be handed over. [[src/builtins/errorcode.rs:ErrTimeout]] |
| `77070003` | `ErrNetworkFailed` | The send fails for a host reason other than a timeout, an oversized payload, or an interruption. [[src/builtins/errorcode.rs:ErrNetworkFailed]] |
| `77010001` | `ErrOutOfMemory` | The NUL-terminated copy of the destination host could not be allocated. [[src/builtins/errorcode.rs:ErrOutOfMemory]] |

## Examples

Send a datagram to a resolved destination and receive it:

```
IMPORT collections
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::bindUdp("127.0.0.1", 0)
  net::setReadTimeout(server, 2000)
  LET bound = net::localAddress(server)
  RES client = net::bindUdp("127.0.0.1", 0)
  LET dest = collections::get(net::lookup("127.0.0.1", bound.port), 0)
  LET payload AS List OF Byte = [10, 20, 30, 40]
  net::sendTo(client, dest, payload)
  LET dg = net::receiveFrom(server, 16)
  io::print(toString(len(dg.bytes)))
  RETURN 0
END FUNC
```

Reply to whoever sent a datagram:

```
IMPORT net

FUNC echoOne(RES sock AS net::UdpSocket) AS Integer
  LET dg = net::receiveFrom(sock, 1024)
  net::sendTo(sock, dg.from, dg.bytes)
  RETURN len(dg.bytes)
  TRAP(e)
    RETURN e.code
  END TRAP
END FUNC

SUB main()
  ' Echoes one datagram back to its sender using the Datagram's `from` address.
END SUB
```

## See also

- `mfb man net sendTextTo`
- `mfb man net receiveFrom`
- `mfb man net receiveTextFrom`
- `mfb man net bindUdp`
- `mfb man net lookup`
- `mfb man net setWriteTimeout`
- `mfb man net close`
