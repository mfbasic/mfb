# listen

Bind a local port and load a server certificate to terminate TLS.

## Synopsis

```
tls::listen(host AS String, port AS Integer, certPath AS String, keyPath AS String) AS TlsListener
tls::listen(host AS String, port AS Integer, certPath AS String, keyPath AS String, backlog AS Integer) AS TlsListener
```

## Package

`tls`

## Imports

```
IMPORT tls
```

## Description

`listen` binds a local TCP endpoint and loads a server TLS identity so a program
can *terminate* TLS: accept encrypted inbound connections, present a server
certificate that clients validate, and exchange application data. It returns a
`TlsListener` resource that `tls::accept` draws connections from. It is the
server-side counterpart to the client's `tls::connect`.

The endpoint is resolved and bound exactly as `net::listenTcp` does. An empty
`host` (or `"0.0.0.0"`) binds all local interfaces; any other value binds the
matching address. The listening socket is created with the address-reuse option
set, so a restarted server can re-bind a recently used port. The optional
`backlog` hints the size of the kernel's pending-connection queue; `0` (the
default when omitted) uses the host default.

`certPath` and `keyPath` are filesystem paths to PEM files: the certificate
chain (leaf certificate first, followed by any intermediates) and the matching
private key. The pair is loaded once, when the listener is created, into a
**server TLS context** that every accepted connection reuses. On Linux the
context is an OpenSSL `SSL_CTX` built from `TLS_server_method` with the chain and
key loaded via `SSL_CTX_use_certificate_chain_file` /
`SSL_CTX_use_PrivateKey_file` and cross-checked with
`SSL_CTX_check_private_key`; the minimum protocol is TLS 1.2. On macOS the PEM
pair is imported through Security.framework into a `sec_identity` installed on a
Network.framework listener. A cert or key that cannot be read, does not parse, or
does not match its partner raises `ErrTlsFailed` and the listening socket is
closed before the error is returned.

The server TLS context is owned by the `TlsListener` and *borrowed* by each
accepted `TlsSocket`: closing an accepted socket never frees the shared context,
which is released exactly once when the listener itself closes. The listener
presents its certificate but does not request or verify a client certificate
(no mutual TLS in this version).

`TlsListener` resources are closed by lexical drop at scope exit or explicitly
with `tls::close`. Draw connections from a listener with `tls::accept`.
[[src/builtins/tls.rs:TLS_LISTENER_TYPE]]

## Overloads

**`tls::listen(host AS String, port AS Integer, certPath AS String, keyPath AS String) AS TlsListener`**

Binds `host`/`port` with the host default backlog and loads the identity.

**`tls::listen(host AS String, port AS Integer, certPath AS String, keyPath AS String, backlog AS Integer) AS TlsListener`**

As above, with an explicit pending-connection `backlog` hint (`0` uses the host
default). [[src/builtins/tls.rs:resolve_call]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `host` | `String` | The local address to bind. An empty string or `"0.0.0.0"` binds all interfaces; any other value binds the matching local address. |
| `port` | `Integer` | The local TCP port to bind and listen on. |
| `certPath` | `String` | Filesystem path to a PEM file holding the server certificate chain, leaf certificate first. |
| `keyPath` | `String` | Filesystem path to a PEM file holding the private key matching the leaf certificate. |
| `backlog` | `Integer` | Optional. A hint for the kernel pending-connection queue length. Defaults to `0`, which uses the host default. |

## Return value

| Type | Description |
| --- | --- |
| `TlsListener` | A bound, listening `TlsListener` resource that owns the loaded server TLS context, ready for `tls::accept`. The listener is closed by lexical drop at scope exit unless closed earlier with `tls::close`. [[src/builtins/tls.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | Memory for a path/host C string or the `TlsListener` handle could not be allocated. |
| `77070001` | `ErrAddressInvalid` | `host`/`port` could not be resolved to a bindable local endpoint. |
| `77070003` | `ErrNetworkFailed` | The listening socket could not be created, bound, or set to listen. |
| `77070008` | `ErrTlsFailed` | The TLS layer could not be initialized, or the server identity could not be loaded — the certificate or key file could not be read or parsed, or the key does not match the certificate. |

## Type checking

`certPath` and `keyPath` must be `String`; `port` and the optional `backlog`
must be `Integer`. Other argument types are rejected at compile time.

## Examples

Terminate TLS on port 8443 with a self-signed certificate and echo one line:

```
IMPORT tls
IMPORT io

RES server = tls::listen("127.0.0.1", 8443, "cert.pem", "key.pem")
RES client = tls::accept(server)
LET line = tls::readText(client, 4096)
tls::writeText(client, "you said: " + line)
tls::close(client)
' server is closed by lexical drop when this scope ends
```

## See also

- `mfb man tls accept`
- `mfb man tls connect`
- `mfb man tls close`
- `mfb man tls readText`
- `mfb man tls writeText`
- `mfb man net listenTcp`
