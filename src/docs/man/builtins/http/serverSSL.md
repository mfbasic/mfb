# serverSSL

Bind an HTTPS listening socket, load a PEM server identity, and return the `tls::TlsListener` that drives the accept loop.

## Synopsis

```
http::serverSSL(port AS Integer, certPath AS String, keyPath AS String) AS tls::TlsListener
http::serverSSL(port AS Integer, certPath AS String, keyPath AS String, host AS String) AS tls::TlsListener
http::serverSSL(port AS Integer, certPath AS String, keyPath AS String, host AS String, backlog AS Integer) AS tls::TlsListener
```

## Package

`http`

## Imports

```
IMPORT tls
IMPORT http
```

`IMPORT tls` is required because the returned value is a `tls::TlsListener` and
the binding must name that type.

## Description

`serverSSL` is the TLS counterpart of `http::server`. It binds a listening TCP
socket and loads a server certificate chain and private key, returning the
`tls::TlsListener` **directly** — the `http` package adds no wrapper resource of
its own. The call is a pass-through to `tls::listen(host, port, certPath,
keyPath, backlog)`, so the listener behaves in every respect like one opened by
`tls` itself. Note the argument order differs: `serverSSL` leads with `port` to
match `http::server`, while `tls::listen` leads with `host`.
[[src/builtins/http_package.mfb:__http_serverSSL]]

`host` defaults to `"0.0.0.0"` and `backlog` defaults to `128`; both defaults are
injected at IR lowering, so the three- and four-argument forms are exactly the
five-argument form with those literals supplied. The `128` default is supplied by
`http` — calling `tls::listen` directly defaults `backlog` to `0` instead.
[[src/builtins/http.rs:default_argument_padding]]

The socket is created with `SO_REUSEADDR` set, bound, and placed in the listening
state. On Linux, address resolution uses `AF_INET` passive hints, so **only IPv4
is bound** — an IPv6 host such as `"::"` does not resolve and fails rather than
binding. An empty `host` (`""`) is passed to the resolver as a passive (NULL)
node and binds every IPv4 interface, which is equivalent to the `"0.0.0.0"`
default. Only the low 16 bits of `port` reach the socket: the value is written
into the two `sin_port` bytes of the resolved address, so a `port` outside
`0..65535` is truncated modulo 65536 rather than rejected.
[[src/target/shared/code/tls/openssl.rs:lower_tls_listen_helper]]
[[src/target/shared/code/tls/mod.rs:HINTS_FAMILY_WORD_PASSIVE]]

`certPath` and `keyPath` are filesystem paths to PEM files: the certificate
chain (leaf certificate first, then any intermediates) and the matching private
key. The pair is loaded once, when the listener is created, into a server TLS
context that every accepted connection reuses. On Linux the context is an
OpenSSL `SSL_CTX` built from `TLS_server_method`, loaded with
`SSL_CTX_use_certificate_chain_file` and `SSL_CTX_use_PrivateKey_file` and
cross-checked with `SSL_CTX_check_private_key`; the minimum protocol version is
pinned to TLS 1.2, and a failure to pin it is itself an error rather than a
silent downgrade. On macOS the PEM pair is imported through Security.framework
into a `sec_identity` installed on a Network.framework listener, and `backlog` is
accepted but ignored because Network.framework manages its own accept queue. A
certificate or key that cannot be read, does not parse, or does not match its
partner raises `ErrTlsFailed`, and the listening socket is closed before the
error is returned.
[[src/target/shared/code/tls/openssl.rs:lower_tls_listen_helper]]
[[src/target/shared/code/tls/macos/server.rs:lower_tls_listen_macos]]

A single server certificate is presented: there is no SNI multi-certificate
selection, and the listener does not request or verify a client certificate (no
mutual TLS).

The server TLS context is owned by the listener and *borrowed* by each accepted
socket, so closing an accepted connection never frees the shared context; it is
released exactly once when the listener itself closes.
[[src/target/shared/code/tls/mod.rs:TLS_LISTENER_OFFSET_CTX]]

The returned listener is a resource: bind it with `RES`, and it is closed by
lexical drop at scope exit (or earlier with `tls::close`). Drive it with a
user-owned `DO`/`LOOP` over `http::handleRequest`, which is overloaded on the
listener type — the loop body and route list are unchanged between `http://` and
`https://`. Each call accepts one connection, performs the server-side TLS
handshake, parses the request, matches its path against an ordered
`List OF http::Route`, invokes the matched handler, writes the response, and
closes the connection. The server is single-threaded and blocking: one request is
served at a time, in the caller's loop.
[[src/builtins/http_package.mfb:__http_handleRequestSSL]] [[src/builtins/http.rs:resolve_call]]

## Overloads

**`http::serverSSL(port AS Integer, certPath AS String, keyPath AS String) AS tls::TlsListener`**

Binds `port` on all IPv4 interfaces (`"0.0.0.0"`) with a backlog of `128`.

**`http::serverSSL(port AS Integer, certPath AS String, keyPath AS String, host AS String) AS tls::TlsListener`**

Binds `port` on the given interface with a backlog of `128`.

**`http::serverSSL(port AS Integer, certPath AS String, keyPath AS String, host AS String, backlog AS Integer) AS tls::TlsListener`**

The full form: binds `port` on `host` with the given backlog hint.
[[src/builtins/http.rs:resolve_call]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `port` | `Integer` | The local TCP port to bind. Only the low 16 bits are used (values outside `0..65535` truncate modulo 65536). |
| `certPath` | `String` | Filesystem path to a PEM file holding the server certificate chain, leaf certificate first. |
| `keyPath` | `String` | Filesystem path to a PEM file holding the private key matching the leaf certificate. |
| `host` | `String` | Optional local IPv4 interface to bind, as a textual address or a resolvable name. `"0.0.0.0"` or `""` bind every IPv4 interface. Defaults to `"0.0.0.0"`. |
| `backlog` | `Integer` | Optional pending-connection queue hint passed to `listen()`. Defaults to `128`. Ignored on macOS, where Network.framework manages its own queue. |

## Return value

| Type | Description |
| --- | --- |
| `tls::TlsListener` | A bound, listening TLS resource owning the loaded server TLS context, ready for `http::handleRequest` (or `tls::accept`). It must be bound with `RES` and is closed by lexical drop at scope exit unless closed earlier with `tls::close`. [[src/builtins/http.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | Memory for a host/path C string or the `TlsListener` handle could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77070001` | `ErrAddressInvalid` | `host` could not be resolved into a local IPv4 endpoint — a malformed address, an unresolvable name, or an IPv6-only host such as `"::"`. Linux only; macOS reports the same condition as `ErrNetworkFailed`. [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_INVALID_CODE]] |
| `77070003` | `ErrNetworkFailed` | The socket could not be created, bound, or placed in the listening state — for example the port is already in use or binding it requires privileges the process lacks. [[src/target/shared/code/error_constants.rs:ERR_NETWORK_FAILED_CODE]] |
| `77070008` | `ErrTlsFailed` | The TLS layer could not be initialized (on Linux, `libssl` could not be loaded or a required symbol was missing), or the server identity could not be loaded — the certificate or key file could not be read or parsed, or the key does not match the certificate. [[src/target/shared/code/error_constants.rs:ERR_TLS_FAILED_CODE]] |

## Examples

An HTTPS server sharing the same route list and loop shape as a plaintext one:

```
IMPORT http
IMPORT tls
IMPORT collections

FUNC home(req AS http::Request) AS http::Response
  RETURN http::ok("welcome")
END FUNC

SUB secureMain()
  MUT routes AS List OF http::Route = []
  routes = collections::append(routes, http::route("/", home))
  RES s AS tls::TlsListener = http::serverSSL(8443, "cert.pem", "key.pem")
  DO
    http::handleRequest(s, routes)
  LOOP UNTIL FALSE
END SUB
```

Bind loopback only, with an explicit backlog:

```
IMPORT http
IMPORT tls
IMPORT io

SUB localOnly()
  RES s AS tls::TlsListener = http::serverSSL(8443, "cert.pem", "key.pem", "127.0.0.1", 16)
  io::print("listening on 127.0.0.1:8443")
END SUB
```

## See also

- `mfb man http server`
- `mfb man http handleRequest`
- `mfb man http route`
- `mfb man tls listen`
- `mfb man tls accept`
- `mfb man tls close`
