# localAddress

Report the local endpoint bound to a network resource.

## Synopsis

```
net::localAddress(sock AS Socket) AS Address
net::localAddress(listener AS Listener) AS Address
net::localAddress(sock AS UdpSocket) AS Address
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

`net::localAddress` asks the host for the address bound to this side of a network
resource and returns it as an `Address` record. It spans all three `net` handle
types: a connected TCP `Socket`, a TCP `Listener`, and a bound UDP `UdpSocket`.
The handle is borrowed, not consumed, and stays open.
[[src/builtins/net.rs:resolve_call]] [[src/builtins/net.rs:consumes_argument]]

The call reads the endpoint with `getsockname` into a `sockaddr_storage`, then
converts it into an `Address` whose `host` field is the textual form of the
address and whose `port` field is the port. The `Address` record is freshly
allocated on each call; the socket itself is untouched.
[[src/target/shared/code/net/io.rs:lower_net_address_helper]]

The most common use is discovering the concrete port behind an ephemeral bind.
After `net::listenTcp(host, 0)` or `net::bindUdp(host, 0)` the host has chosen a
port that the program never named, and `net::localAddress(...).port` is how it is
read back. For a resource bound to a wildcard host the reported host is that
wildcard address, while the port is always the concrete one the host assigned.

Use `net::remoteAddress` for the *peer* endpoint of a connected `Socket`; it is
the only address query that does not accept a `Listener` or a `UdpSocket`,
because only a connected socket has a peer.

## Overloads

**`net::localAddress(sock AS Socket) AS Address`**

Reports the local endpoint of a connected TCP socket, including the local port
the host assigned to an outbound connection.

**`net::localAddress(listener AS Listener) AS Address`**

Reports the endpoint a TCP listener is accepting on.

**`net::localAddress(sock AS UdpSocket) AS Address`**

Reports the endpoint a UDP socket is bound to.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `Socket` or `UdpSocket` | The connected TCP socket or bound UDP socket whose local endpoint is wanted. It must still be open; the handle is borrowed, not consumed. [[src/builtins/net.rs:call_param_names]] |
| `listener` | `Listener` | Alternatively, the listener whose accepting endpoint is wanted. `sock` and `listener` are alternate named-argument spellings of the same position 0, so `net::localAddress(sock := s)` and `net::localAddress(listener := l)` both bind it. [[src/builtins/net.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Address` | A record whose `host` field (`String`) holds the textual local address and whose `port` field (`Integer`) holds the local port. After binding or listening on port `0`, `port` is the concrete port the host chose. [[src/builtins/net.rs:builtin_type_fields]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | The resource has already been closed, or the host's `getsockname` fails — which it does when the descriptor is no longer a usable socket. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77070001` | `ErrAddressInvalid` | The address the host reported could not be converted to its textual form, so it cannot be represented as an `Address`. [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_INVALID_CODE]] |
| `77010001` | `ErrOutOfMemory` | The host string or the `Address` record could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Discover the port assigned when listening on port `0`:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  io::print(toString(bound.port))
  RETURN 0
END FUNC
```

Inspect the local endpoint of an outbound connection:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  LET local = net::localAddress(client)
  io::print(local.host & " " & toString(local.port))
  RETURN 0
END FUNC
```

## See also

- `mfb man net remoteAddress`
- `mfb man net listenTcp`
- `mfb man net connectTcp`
- `mfb man net accept`
- `mfb man net bindUdp`
- `mfb man net close`
