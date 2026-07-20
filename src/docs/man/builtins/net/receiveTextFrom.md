# receiveTextFrom

Receive a single UDP datagram as UTF-8 text together with its sender address.

## Synopsis

```
net::receiveTextFrom(sock AS UdpSocket, maxBytes AS Integer) AS DatagramText
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

`net::receiveTextFrom` receives exactly one datagram from a bound `UdpSocket` and
returns it as a `DatagramText` record with two fields: `from`, the sender's
`Address`, and `value`, the payload decoded as a UTF-8 `String`. Note the field
name — the text payload is `value`, not `text`, and its byte-oriented counterpart
in `Datagram` is `bytes`. Because UDP is connectionless, one bound socket can
receive from many peers, and each call reports who sent the datagram it returned.
[[src/builtins/net.rs:builtin_type_fields]]

A datagram is delivered whole or not at all. `maxBytes` bounds the payload the
call will accept and must be positive. The receive buffer is deliberately
allocated one byte larger than `maxBytes`, so an oversized datagram is detected
and rejected with `ErrMessageTooLarge` rather than silently truncated. The
returned string holds the entire payload and is frequently shorter than
`maxBytes` bytes; a zero-length datagram is a valid UDP message and yields an
empty string rather than an error.
[[src/target/shared/code/net/io.rs:lower_net_receive_from_helper]]

The payload bytes are validated as UTF-8 before the string is returned, and
invalid bytes raise `ErrEncoding`. Unlike `net::readText` on a TCP stream this is
not a framing hazard: a datagram is received whole, so a multi-byte UTF-8
sequence is never split across two calls. Use `net::receiveFrom` when the payload
is raw binary and a `List OF Byte` is the right shape.
[[src/target/shared/code/net/io.rs:lower_net_receive_from_helper]]

The call blocks until a datagram arrives or the socket's read timeout elapses;
use `net::setReadTimeout` to bound the wait, after which `ErrReadTimeout` is
raised. A signal that interrupts the receive before any byte moved re-issues the
identical call. The sender's address and the decoded payload are each freshly
allocated; the socket is borrowed, stays open, and is otherwise untouched. Reply
to `from` with `net::sendTextTo`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `UdpSocket` | A bound UDP socket to receive on, as returned by `net::bindUdp`. It must still be open; the handle is borrowed, not consumed. [[src/builtins/net.rs:call_param_names]] |
| `maxBytes` | `Integer` | The largest payload the call will accept, in bytes. Must be positive. A datagram exceeding it is rejected with `ErrMessageTooLarge`, never truncated. [[src/target/shared/code/net/io.rs:lower_net_receive_from_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `DatagramText` | A record whose `from` field is the sender's `Address` and whose `value` field is the whole payload decoded as a UTF-8 `String`, built from between `0` and `maxBytes` bytes inclusive. [[src/builtins/net.rs:builtin_type_fields]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `maxBytes` is not positive. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77070007` | `ErrMessageTooLarge` | The received datagram's payload is larger than `maxBytes`. The datagram is not truncated and not returned. [[src/target/shared/code/error_constants.rs:ERR_MESSAGE_TOO_LARGE_CODE]] |
| `77070005` | `ErrReadTimeout` | The socket's read timeout elapsed before a datagram arrived. [[src/target/shared/code/error_constants.rs:ERR_READ_TIMEOUT_CODE]] |
| `77020004` | `ErrEncoding` | The received payload is not valid UTF-8. [[src/target/shared/code/error_constants.rs:ERR_ENCODING_CODE]] |
| `77070003` | `ErrNetworkFailed` | The receive fails for a host reason other than a timeout or an interruption. [[src/target/shared/code/error_constants.rs:ERR_NETWORK_FAILED_CODE]] |
| `77070001` | `ErrAddressInvalid` | The sender address reported by the host could not be converted to its textual form. [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_INVALID_CODE]] |
| `77010001` | `ErrOutOfMemory` | The receive buffer, the decoded string, the sender `Address`, or the `DatagramText` record could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Receive one text datagram and print its payload:

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
  net::sendTextTo(client, dest, "ping")
  LET dg = net::receiveTextFrom(server, 64)
  io::print(dg.value)
  RETURN 0
END FUNC
```

Bound the wait and report the error code on a timeout:

```
IMPORT net

FUNC recvOrCode(RES s AS UdpSocket) AS String
  LET dg = net::receiveTextFrom(s, 512)
  RETURN dg.value
  TRAP(e)
    RETURN toString(e.code)
  END TRAP
END FUNC

SUB main()
  ' Returns the datagram text, or the error code — 77070005 (ErrReadTimeout) when
  ' nothing arrived before the read timeout elapsed.
END SUB
```

## See also

- `mfb man net receiveFrom`
- `mfb man net sendTextTo`
- `mfb man net sendTo`
- `mfb man net bindUdp`
- `mfb man net setReadTimeout`
- `mfb man net localAddress`
- `mfb man net close`
