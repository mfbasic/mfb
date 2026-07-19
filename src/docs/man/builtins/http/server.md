# server

Bind a plaintext HTTP/1.1 listening socket and return the `net::Listener` that drives the accept loop.

## Synopsis

```
http::server(port AS Integer) AS net::Listener
http::server(port AS Integer, host AS String) AS net::Listener
http::server(port AS Integer, host AS String, backlog AS Integer) AS net::Listener
```

## Package

`http`

## Imports

```
IMPORT net
IMPORT http
```

`IMPORT net` is required because the returned value is a `net::Listener` and the
binding must name that type.

## Description

`server` binds a listening TCP socket for a plaintext HTTP/1.1 server and returns
the `net::Listener` **directly** — the `http` package adds no wrapper resource of
its own. The call is a pass-through to `net::listenTcp(host, port, backlog)`, so
the listener behaves in every respect like one opened by `net` itself.
[[src/builtins/http_package.mfb:__http_server]]

`host` defaults to `"0.0.0.0"` and `backlog` defaults to `128`; both defaults are
injected at IR lowering, so the one- and two-argument forms are exactly the
three-argument form with those literals supplied.
[[src/builtins/http.rs:default_argument_padding]]

The socket is created with `SO_REUSEADDR` set, bound, and placed in the listening
state. Address resolution uses `AF_INET` hints, so **only IPv4 is bound** — an
IPv6 host such as `"::"` does not resolve and fails rather than binding. An empty
`host` (`""`) is passed to the resolver as a passive (NULL) node and binds every
IPv4 interface, which is equivalent to the `"0.0.0.0"` default.
[[src/target/shared/code/net/mod.rs:emit_hints]] [[src/target/shared/code/net/mod.rs:lower_net_endpoint_helper]]

Only the low 16 bits of `port` reach the socket: the value is written into the
two `sin_port` bytes of the resolved address, so a `port` outside `0..65535` is
truncated modulo 65536 rather than rejected. A `port` of `0` requests an ephemeral
port from the host, which can be read back with `net::localAddress`.
[[src/target/shared/code/net/mod.rs:lower_net_endpoint_helper]]

`backlog` is the pending-connection queue hint passed to `listen()`. Because
`listen()` takes a C `int`, a value above `2147483647` is clamped to that maximum
before the call, so a large 64-bit backlog cannot be reinterpreted as negative.
The value is advisory in any case; the host may clamp it further.
[[src/target/shared/code/net/mod.rs:lower_net_endpoint_helper]]

The returned listener is a resource: bind it with `RES`, and it is closed by
lexical drop at scope exit (or earlier with `net::close`). Drive it with a
user-owned `DO`/`LOOP` over `http::handleRequest`, which accepts one connection
per call, parses the request, matches its path against an ordered
`List OF http::Route`, invokes the matched handler, writes the response, and
closes the connection. The server is single-threaded and blocking: one request is
served at a time, in the caller's loop. For HTTPS use `http::serverSSL`, which
returns a `tls::TlsListener` that `handleRequest` also accepts.
[[src/builtins/http_package.mfb:__http_handleRequest]] [[src/builtins/http.rs:resolve_call]]

## Overloads

**`http::server(port AS Integer) AS net::Listener`**

Binds `port` on all IPv4 interfaces (`"0.0.0.0"`) with a backlog of `128`.

**`http::server(port AS Integer, host AS String) AS net::Listener`**

Binds `port` on the given interface with a backlog of `128`.

**`http::server(port AS Integer, host AS String, backlog AS Integer) AS net::Listener`**

The full form: binds `port` on `host` with the given backlog hint.
[[src/builtins/http.rs:arity]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `port` | `Integer` | The local TCP port to bind. Only the low 16 bits are used (values outside `0..65535` truncate modulo 65536). `0` requests a host-assigned ephemeral port, readable with `net::localAddress`. |
| `host` | `String` | Optional local IPv4 interface to bind, as a textual address or a resolvable name. `"0.0.0.0"` or `""` bind every IPv4 interface. Defaults to `"0.0.0.0"`. |
| `backlog` | `Integer` | Optional pending-connection queue hint for `listen()`. Values above `2147483647` are clamped to that maximum; the host may clamp further. Defaults to `128`. |

## Return value

| Type | Description |
| --- | --- |
| `net::Listener` | A listening socket resource ready for `http::handleRequest` (or `net::accept`). It must be bound with `RES` and is closed by lexical drop at scope exit unless closed earlier with `net::close`. [[src/builtins/http.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | Memory for the host C string or the `Listener` handle could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77070001` | `ErrAddressInvalid` | `host` could not be resolved into a local IPv4 endpoint — a malformed address, an unresolvable name, or an IPv6-only host such as `"::"`. [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_INVALID_CODE]] |
| `77070003` | `ErrNetworkFailed` | The socket could not be created, bound, or placed in the listening state — for example the port is already in use or binding it requires privileges the process lacks. [[src/target/shared/code/error_constants.rs:ERR_NETWORK_FAILED_CODE]] |

## Examples

A minimal server on port 8080:

```
IMPORT http
IMPORT net
IMPORT collections

FUNC home(req AS http::Request) AS http::Response
  RETURN http::ok("welcome")
END FUNC

SUB serverMain()
  MUT routes AS List OF http::Route = []
  routes = collections::append(routes, http::route("/", home))
  RES s AS net::Listener = http::server(8080)
  DO
    http::handleRequest(s, routes)
  LOOP UNTIL FALSE
END SUB
```

Bind loopback only, with an explicit backlog:

```
IMPORT http
IMPORT net

SUB localOnly()
  RES s AS net::Listener = http::server(8080, "127.0.0.1", 16)
  LET bound = net::localAddress(s)
  io::print("listening on port " & toString(bound.port))
END SUB
```

## See also

- `mfb man http serverSSL`
- `mfb man http handleRequest`
- `mfb man http route`
- `mfb man net listenTcp`
- `mfb man net localAddress`
