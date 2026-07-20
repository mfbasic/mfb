# sendTextTo

Send a single UDP datagram of UTF-8 text to a destination address.

## Synopsis

```
net::sendTextTo(sock AS UdpSocket, address AS Address, value AS String) AS Nothing
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

`net::sendTextTo` transmits the UTF-8 bytes of `value` as one UDP datagram from a
bound `UdpSocket` to the peer named by `address`. It is the text counterpart of
`net::sendTo`: instead of building a `List OF Byte`, the string's packed byte
data is sent directly from its buffer. A `String` already holds well-formed
UTF-8, so the bytes go out exactly as held, with no re-encoding, decoding, or
newline translation. The socket is borrowed and stays open.
[[src/target/shared/code/net/io.rs:lower_net_send_to_helper]]

`address` supplies both the destination host and the destination port. The host
is resolved with the host resolver on **every** call — it may be a numeric IP
literal or a name — and the `port` field is then written directly into the
resolved address rather than being resolved as a service name. The resolver's
answer chain is released before the call returns, on both the success and the
failure paths. In a tight send loop, resolve once with `net::lookup` and reuse the
resulting `Address`. [[src/target/shared/code/net/io.rs:lower_net_send_to_helper]]

The whole string is sent as the payload of a single datagram in byte order. UDP
preserves message boundaries: the payload arrives whole or not at all, and is
never split across datagrams or merged with another. An empty string sends a
zero-length datagram, which is a valid UDP message rather than a no-op.

A successful return means the datagram was accepted by the host for best-effort
delivery, not that any peer received it. The call may block while the send buffer
is full; use `net::setWriteTimeout` to bound that wait, after which
`ErrWriteTimeout` is raised. A payload larger than the path allows is rejected
with `ErrMessageTooLarge` rather than truncated. A signal that interrupts the
send before any byte left re-issues the identical call — a datagram send is
all-or-nothing, so a send that already completed is never retried.

To reply to a sender, pass the `from` field of the `DatagramText` returned by
`net::receiveTextFrom` (or of the `Datagram` from `net::receiveFrom`; both carry
the same `Address`). The text payload of a `DatagramText` is its `value` field.
[[src/builtins/net.rs:builtin_type_fields]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `UdpSocket` | A bound UDP socket to send from, as returned by `net::bindUdp`. It must still be open; the handle is borrowed, not consumed. [[src/builtins/net.rs:call_param_names]] |
| `address` | `Address` | The destination. Its `host` field is resolved on each call and may be a numeric IP literal or a name; its `port` field selects the destination port. Obtain one from `net::lookup`, or from the `from` field of a received `Datagram` or `DatagramText`. [[src/builtins/net.rs:builtin_type_fields]] |
| `value` | `String` | The text to send, transmitted as its UTF-8 bytes in order as one datagram. An empty string sends a valid zero-length datagram. [[src/builtins/net.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `sendTextTo` returns no value. A successful call has handed the datagram to the host for best-effort delivery; it does not guarantee receipt. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77070002` | `ErrAddressNotFound` | The destination host in `address` could not be resolved. [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_NOT_FOUND_CODE]] |
| `77070007` | `ErrMessageTooLarge` | The payload is too large to be sent as a single datagram on this path (the host reports `EMSGSIZE`). [[src/target/shared/code/error_constants.rs:ERR_MESSAGE_TOO_LARGE_CODE]] |
| `77070006` | `ErrWriteTimeout` | The socket's write timeout elapsed before the datagram could be handed over. [[src/target/shared/code/error_constants.rs:ERR_WRITE_TIMEOUT_CODE]] |
| `77070003` | `ErrNetworkFailed` | The send fails for a host reason other than a timeout, an oversized payload, or an interruption. [[src/target/shared/code/error_constants.rs:ERR_NETWORK_FAILED_CODE]] |
| `77010001` | `ErrOutOfMemory` | The NUL-terminated copy of the destination host could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Send a line of text to a resolved destination and receive it:

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
  net::sendTextTo(client, dest, "hello")
  LET dg = net::receiveTextFrom(server, 64)
  io::print(dg.value)
  RETURN 0
END FUNC
```

Echo received text back to its sender:

```
IMPORT net

FUNC echoOne(RES sock AS UdpSocket) AS Integer
  LET dg = net::receiveTextFrom(sock, 1024)
  net::sendTextTo(sock, dg.from, dg.value)
  RETURN len(dg.value)
  TRAP(e)
    RETURN e.code
  END TRAP
END FUNC

SUB main()
  ' Echoes one text datagram back to its sender; `from` is the sender Address and
  ' `value` is the decoded payload.
END SUB
```

## See also

- `mfb man net sendTo`
- `mfb man net receiveTextFrom`
- `mfb man net receiveFrom`
- `mfb man net bindUdp`
- `mfb man net lookup`
- `mfb man net setWriteTimeout`
- `mfb man net close`
