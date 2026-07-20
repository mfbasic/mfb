# receiveFrom

Receive a single UDP datagram as bytes together with its sender address.

## Synopsis

```
net::receiveFrom(sock AS UdpSocket, maxBytes AS Integer) AS Datagram
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

`net::receiveFrom` receives exactly one datagram from a bound `UdpSocket` and
returns it as a `Datagram` record with two fields: `from`, the sender's
`Address`, and `bytes`, the payload as a `List OF Byte`. Because UDP is
connectionless, one bound socket can receive from many peers, and each call
reports who sent the datagram it returned.
[[src/builtins/net.rs:builtin_type_fields]]

A datagram is delivered whole or not at all. `maxBytes` bounds the payload the
call will accept and must be positive. The receive buffer is deliberately
allocated one byte larger than `maxBytes`, so an oversized datagram is detected
by the host returning more than `maxBytes` bytes and is rejected with
`ErrMessageTooLarge` rather than silently truncated. Size `maxBytes` to the
largest message the protocol expects. The returned list holds the entire payload
and is frequently shorter than `maxBytes`; a zero-length datagram is a valid UDP
message and yields an empty list rather than an error — unlike a TCP read, where
a zero-length result would mean end of stream.
[[src/target/shared/code/net/io.rs:lower_net_receive_from_helper]]

The call blocks until a datagram arrives or the socket's read timeout elapses;
use `net::setReadTimeout` to bound the wait, after which `ErrReadTimeout` is
raised (the host reporting `EAGAIN` is what distinguishes a timeout from a hard
network failure). A signal that interrupts the receive before any byte moved
re-issues the identical call rather than reporting a spurious failure.
[[src/target/shared/code/net/io.rs:lower_net_receive_from_helper]]

The sender's address is captured alongside the payload and converted into a
freshly allocated `Address`; the payload is copied into a freshly allocated
`List OF Byte`. The socket is borrowed, stays open, and is otherwise untouched.
Use `net::receiveTextFrom` when the payload is UTF-8 text and a `String` is more
convenient than raw bytes, and `net::sendTo` to reply to `from`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `UdpSocket` | A bound UDP socket to receive on, as returned by `net::bindUdp`. It must still be open; the handle is borrowed, not consumed. [[src/builtins/net.rs:call_param_names]] |
| `maxBytes` | `Integer` | The largest payload the call will accept, in bytes. Must be positive. A datagram exceeding it is rejected with `ErrMessageTooLarge`, never truncated. [[src/target/shared/code/net/io.rs:lower_net_receive_from_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `Datagram` | A record whose `from` field is the sender's `Address` and whose `bytes` field is the whole payload as a `List OF Byte`, of length between `0` and `maxBytes` inclusive. [[src/builtins/net.rs:builtin_type_fields]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `maxBytes` is not positive. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77070007` | `ErrMessageTooLarge` | The received datagram's payload is larger than `maxBytes`. The datagram is not truncated and not returned. [[src/target/shared/code/error_constants.rs:ERR_MESSAGE_TOO_LARGE_CODE]] |
| `77070005` | `ErrReadTimeout` | The socket's read timeout elapsed before a datagram arrived. [[src/target/shared/code/error_constants.rs:ERR_READ_TIMEOUT_CODE]] |
| `77070003` | `ErrNetworkFailed` | The receive fails for a host reason other than a timeout or an interruption. [[src/target/shared/code/error_constants.rs:ERR_NETWORK_FAILED_CODE]] |
| `77070001` | `ErrAddressInvalid` | The sender address reported by the host could not be converted to its textual form, so it cannot be represented as an `Address`. [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_INVALID_CODE]] |
| `77010001` | `ErrOutOfMemory` | The receive buffer, the payload list, the sender `Address`, or the `Datagram` record could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Receive one datagram and report its size:

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

Report the error code when the datagram does not fit:

```
IMPORT net

FUNC recvCount(RES s AS UdpSocket, maxBytes AS Integer) AS Integer
  LET dg = net::receiveFrom(s, maxBytes)
  RETURN len(dg.bytes)
  TRAP(e)
    RETURN e.code
  END TRAP
END FUNC

SUB main()
  ' Returns the payload size, or the error code when the datagram exceeds maxBytes
  ' (ErrMessageTooLarge, 77070007) or the read timeout elapses.
END SUB
```

## See also

- `mfb man net receiveTextFrom`
- `mfb man net sendTo`
- `mfb man net sendTextTo`
- `mfb man net bindUdp`
- `mfb man net setReadTimeout`
- `mfb man net localAddress`
- `mfb man net close`
