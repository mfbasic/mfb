# bindUdp

Open a UDP datagram socket bound to a local address.

## Synopsis

```
net::bindUdp(host AS String, port AS Integer) AS UdpSocket
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

`net::bindUdp` creates a connectionless UDP datagram socket bound to a local
endpoint and returns a `UdpSocket` resource ready to send and receive datagrams.
The call resolves `host` with the host resolver requesting a `SOCK_DGRAM`
endpoint, creates a socket from the first resolved result, patches the requested
`port` into the resolved address, and binds it.
[[src/target/shared/code/net/io.rs:lower_net_bind_udp_helper]]

`host` names the local interface to bind, given as a textual IP address or a name
handed to the resolver. An empty `host` binds every interface: the resolver is
called with a null node and the passive flag, and — because a null node requires
a non-null service — with the service string `"0"`, whose port is then overwritten
by the requested `port`. `"0.0.0.0"` and `"::"` are ordinary textual wildcard
addresses that reach the same result through normal resolution. When `port` is
`0` the host assigns an ephemeral port, which `net::localAddress` reads back.
[[src/target/shared/code/net/io.rs:lower_net_bind_udp_helper]]

Unlike TCP there is no listen or accept step: a UDP socket is not tied to a
single peer. Send datagrams with `net::sendTo` or `net::sendTextTo`, each naming
its own destination, and receive them with `net::receiveFrom` or
`net::receiveTextFrom`, which report the sender's `Address` alongside the
payload. Bound how long a receive or send may block with `net::setReadTimeout`
and `net::setWriteTimeout`.

The returned `UdpSocket` is an owned, non-copyable resource handle. It is closed
by lexical drop when its binding leaves scope, or earlier with `net::close`; it
cannot be stored in a collection or a record. If the socket cannot be created or
bound, the partially created descriptor and the resolver results are released
before the error is raised, so a failed `bindUdp` leaks neither.
[[src/builtins/net.rs:resource_close_function]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `host` | `String` | The local interface to bind, as a textual IP address or a name passed to the host resolver. `"0.0.0.0"`, `"::"`, or an empty string bind every interface. [[src/builtins/net.rs:call_param_names]] |
| `port` | `Integer` | The local UDP port to bind. `0` requests an ephemeral port assigned by the host, readable afterwards with `net::localAddress`. [[src/target/shared/code/net/io.rs:lower_net_bind_udp_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `UdpSocket` | A bound datagram socket, ready for `net::sendTo`, `net::sendTextTo`, `net::receiveFrom`, and `net::receiveTextFrom`. Closed by lexical drop at scope exit unless closed earlier with `net::close`. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77070001` | `ErrAddressInvalid` | `host` could not be resolved into a local endpoint — the resolver rejected it as malformed or unknown. [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_INVALID_CODE]] |
| `77070003` | `ErrNetworkFailed` | The socket could not be created, or `bind` failed — for example the address and port are already in use, or the port requires privileges the process does not hold. [[src/target/shared/code/error_constants.rs:ERR_NETWORK_FAILED_CODE]] |
| `77010001` | `ErrOutOfMemory` | The NUL-terminated copy of `host` or the `UdpSocket` handle record could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Bind an ephemeral port and read back the assigned address:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES sock = net::bindUdp("127.0.0.1", 0)
  LET bound = net::localAddress(sock)
  io::print(toString(bound.port))
  RETURN 0
END FUNC
```

Send a datagram between two bound sockets:

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

## See also

- `mfb man net sendTo`
- `mfb man net sendTextTo`
- `mfb man net receiveFrom`
- `mfb man net receiveTextFrom`
- `mfb man net localAddress`
- `mfb man net setReadTimeout`
- `mfb man net close`
