//! `http::serverSSL` — descriptor entry (source-backed, body `__http_serverSSL`).
//!

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Bind an HTTPS listening socket, load a PEM server identity, and return the `tls::Listener` that drives the accept loop."#;

const DESC: &str = r#"`serverSSL` is the TLS counterpart of `http::server`. It binds a listening TCP
socket and loads a server certificate chain and private key, returning the
`tls::Listener` **directly** — the `http` package adds no wrapper resource of
its own. The call is a pass-through to `tls::listen(host, port, certPath,
keyPath, backlog)`, so the listener behaves in every respect like one opened by
`tls` itself. Note the argument order differs: `serverSSL` leads with `port` to
match `http::server`, while `tls::listen` leads with `host`.

`host` defaults to `"0.0.0.0"` and `backlog` defaults to `128`; both defaults are
injected at IR lowering, so the three- and four-argument forms are exactly the
five-argument form with those literals supplied. The `128` default is supplied by
`http` — calling `tls::listen` directly defaults `backlog` to `0` instead.

The socket is created with `SO_REUSEADDR` set, bound, and placed in the listening
state. On Linux, address resolution uses `AF_INET` passive hints, so **only IPv4
is bound** — an IPv6 host such as `"::"` does not resolve and fails rather than
binding. An empty `host` (`""`) is passed to the resolver as a passive (NULL)
node and binds every IPv4 interface, which is equivalent to the `"0.0.0.0"`
default. Only the low 16 bits of `port` reach the socket: the value is written
into the two `sin_port` bytes of the resolved address, so a `port` outside
`0..65535` is truncated modulo 65536 rather than rejected.

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

A single server certificate is presented: there is no SNI multi-certificate
selection, and the listener does not request or verify a client certificate (no
mutual TLS).

The server TLS context is owned by the listener and *borrowed* by each accepted
socket, so closing an accepted connection never frees the shared context; it is
released exactly once when the listener itself closes.

The returned listener is a resource: bind it with `RES`, and it is closed by
lexical drop at scope exit (or earlier with `tls::close`). Drive it with a
user-owned `DO`/`LOOP` over `http::handleRequest`, which is overloaded on the
listener type — the loop body and route list are unchanged between `http://` and
`https://`. Each call accepts one connection, performs the server-side TLS
handshake, parses the request, matches its path against an ordered
`List OF http::Route`, invokes the matched handler, writes the response, and
closes the connection. The server is single-threaded and blocking: one request is
served at a time, in the caller's loop."#;

const EX: &str = r#"An HTTPS server sharing the same route list and loop shape as a plaintext one:

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
  RES s AS tls::Listener = http::serverSSL(8443, "cert.pem", "key.pem")
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
  RES s AS tls::Listener = http::serverSSL(8443, "cert.pem", "key.pem", "127.0.0.1", 16)
  io::print("listening on 127.0.0.1:8443")
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_serverSSL(port AS Integer, certPath AS String, keyPath AS String, host AS String, backlog AS Integer) AS RES tls::Listener
  RETURN tls::listen(host, port, certPath, keyPath, backlog)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "serverSSL",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer, String, String[, String[, Integer]]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("port", "The local TCP port to bind. Only the low 16 bits are used (values outside `0..65535` truncate modulo 65536).", &[], ParameterType::Integer),
                super::req("certPath", "Filesystem path to a PEM file holding the server certificate chain, leaf certificate first.", &[], ParameterType::String),
                super::req("keyPath", "Filesystem path to a PEM file holding the private key matching the leaf certificate.", &[], ParameterType::String),
                super::fill("host", "Optional local IPv4 interface to bind, as a textual address or a resolvable name. `\"0.0.0.0\"` or `\"\"` bind every IPv4 interface. Defaults to `\"0.0.0.0\"`.", ParameterType::String, "0.0.0.0"),
                super::fill("backlog", "Optional pending-connection queue hint passed to `listen()`. Defaults to `128`. Ignored on macOS, where Network.framework manages its own queue.", ParameterType::Integer, "128"),
            ],
            return_type: ParameterType::named(super::TLS_LISTENER_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__http_serverSSL"),
        }],
    });
}
