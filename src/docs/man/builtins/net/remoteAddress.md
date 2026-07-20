# remoteAddress

Report the peer endpoint of a connected TCP socket.

## Synopsis

```
net::remoteAddress(sock AS Socket) AS Address
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

`net::remoteAddress` asks the host for the address of the peer a connected
`Socket` is talking to and returns it as an `Address` record. It is the peer-side
counterpart of `net::localAddress`, and unlike that function it accepts only a
`Socket`: a `Listener` and a `UdpSocket` have no single peer, so passing either
is a compile-time type error rather than a runtime one.
[[src/builtins/net.rs:resolve_call]] [[src/builtins/net.rs:expected_arguments]]

The call reads the endpoint with `getpeername` into a `sockaddr_storage` and
converts it into a freshly allocated `Address` whose `host` field is the textual
form of the address and whose `port` field is the peer port. The socket is
borrowed, stays open, and is otherwise untouched.
[[src/target/shared/code/net/io.rs:lower_net_address_helper]]

The reported host is the concrete address the host stack is actually connected
to, which is not necessarily the string passed to `net::connectTcp`: a name is
resolved before connecting, so a connection opened to `"example.com"` reports the
resolved IP address here. For a socket from `net::accept` this is how a server
identifies the client that connected.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `sock` | `Socket` | A connected TCP socket, as returned by `net::connectTcp` or `net::accept`, whose peer endpoint is wanted. It must still be open; the handle is borrowed, not consumed. [[src/builtins/net.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Address` | A record whose `host` field (`String`) holds the textual peer address the host resolved and whose `port` field (`Integer`) holds the peer port. [[src/builtins/net.rs:builtin_type_fields]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `sock` has already been closed, or the host's `getpeername` fails — which it does when the descriptor is no longer a connected socket. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77070001` | `ErrAddressInvalid` | The peer address the host reported could not be converted to its textual form, so it cannot be represented as an `Address`. [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_INVALID_CODE]] |
| `77010001` | `ErrOutOfMemory` | The host string or the `Address` record could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Inspect the peer endpoint of an outbound connection:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  LET remote = net::remoteAddress(client)
  io::print(remote.host & " " & toString(remote.port))
  RETURN 0
END FUNC
```

Identify the client behind an accepted connection:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  io::print(net::remoteAddress(conn).host)
  RETURN 0
END FUNC
```

## See also

- `mfb man net localAddress`
- `mfb man net connectTcp`
- `mfb man net accept`
- `mfb man net listenTcp`
- `mfb man net close`
