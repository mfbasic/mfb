# listenTcp

Open a TCP listening socket bound to a local address.

## Synopsis

```
net::listenTcp(host AS String, port AS Integer) AS Listener
net::listenTcp(host AS String, port AS Integer, backlog AS Integer) AS Listener
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

`net::listenTcp` binds a local TCP socket to `host` and `port` and places it in
the listening state, returning a `Listener` ready for `net::accept`. The host is
resolved for a passive `SOCK_STREAM` endpoint, a socket is created from the first
result, `SO_REUSEADDR` is set on it as a best-effort option, the requested port
is patched into the resolved address, and the socket is bound and switched into
the listening state. [[src/target/shared/code/net/mod.rs:lower_net_endpoint_helper]]

`host` names the local interface to bind. An empty `host` binds every interface:
the resolver is called with a null node and the passive flag, and — because a
null node requires a non-null service — with the service string `"0"`, whose port
the requested `port` then overwrites. `"0.0.0.0"` and `"::"` are ordinary
textual wildcard addresses that reach the same result through normal resolution.
When `port` is `0` the host assigns an ephemeral port, which `net::localAddress`
reads back — the usual way to run a server on an unpredictable free port.
[[src/target/shared/code/net/mod.rs:lower_net_endpoint_helper]]

`backlog` hints how many pending connections the host may queue before refusing
new ones. It is not a host default when omitted: the compiler fills the missing
third argument with the literal `128`, so the two-argument form is exactly
`net::listenTcp(host, port, 128)`. Because `listen` takes a C `int`, a `backlog`
above 2147483647 is clamped to that value before the call. Beyond that the value
is advisory — the host may cap it at its own limit.
[[src/target/shared/code/builder_values.rs:net_connect_is_address_form]]
[[src/target/shared/code/net/mod.rs:lower_net_endpoint_helper]]

The returned `Listener` is an owned, non-copyable resource handle, closed by
lexical drop when its binding leaves scope or earlier with `net::close`. Each
`net::accept` on it returns an independent `Socket` that outlives the listener.
If binding or listening fails, the partially created descriptor and the resolver
results are released before the error is raised.
[[src/builtins/net.rs:resource_close_function]]

## Overloads

**`net::listenTcp(host AS String, port AS Integer) AS Listener`**

Binds `host` on `port` and listens with the default backlog of `128`.

**`net::listenTcp(host AS String, port AS Integer, backlog AS Integer) AS Listener`**

Binds `host` on `port` and listens with the given backlog hint.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `host` | `String` | The local interface to bind, as a textual IP address or a name passed to the host resolver. `"0.0.0.0"`, `"::"`, or an empty string bind every interface. [[src/builtins/net.rs:call_param_names]] |
| `port` | `Integer` | The local TCP port to bind. `0` requests an ephemeral port assigned by the host, readable afterwards with `net::localAddress`. [[src/target/shared/code/net/mod.rs:lower_net_endpoint_helper]] |
| `backlog` | `Integer` | Optional. A hint for how many pending connections the host may queue. Defaults to `128` when omitted, is clamped to `2147483647`, and may be further capped by the host. [[src/target/shared/code/builder_values.rs:net_connect_is_address_form]] |

## Return value

| Type | Description |
| --- | --- |
| `Listener` | A socket in the listening state, ready for `net::accept`. Closed by lexical drop at scope exit unless closed earlier with `net::close`. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77070001` | `ErrAddressInvalid` | `host` could not be resolved into a local endpoint — the resolver rejected it as malformed or unknown. [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_INVALID_CODE]] |
| `77070003` | `ErrNetworkFailed` | The socket could not be created, bound, or placed in the listening state — for example the address and port are already in use, or the port requires privileges the process does not hold. [[src/target/shared/code/error_constants.rs:ERR_NETWORK_FAILED_CODE]] |
| `77010001` | `ErrOutOfMemory` | The NUL-terminated copy of `host` or the `Listener` handle record could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Listen on an ephemeral port and read back the assigned port:

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

Listen with an explicit backlog and serve one client:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0, 16)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  net::writeText(conn, "hello")
  io::print(net::readText(client, 16))
  RETURN 0
END FUNC
```

## See also

- `mfb man net accept`
- `mfb man net connectTcp`
- `mfb man net localAddress`
- `mfb man net bindUdp`
- `mfb man net close`
