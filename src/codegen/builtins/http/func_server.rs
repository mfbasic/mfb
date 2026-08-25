//! `http::server` — descriptor entry (source-backed, body `__http_server`).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Bind a plaintext HTTP/1.1 listening socket and return the `net::Listener` that drives the accept loop."#;

const DESC: &str = r#"`server` binds a listening TCP socket for a plaintext HTTP/1.1 server and returns
the `net::Listener` **directly** — the `http` package adds no wrapper resource of
its own. The call is a pass-through to `net::listenTcp(host, port, backlog)`, so
the listener behaves in every respect like one opened by `net` itself.

`host` defaults to `"0.0.0.0"` and `backlog` defaults to `128`; both defaults are
injected at IR lowering, so the one- and two-argument forms are exactly the
three-argument form with those literals supplied.

The socket is created with `SO_REUSEADDR` set, bound, and placed in the listening
state. Address resolution uses `AF_INET` hints, so **only IPv4 is bound** — an
IPv6 host such as `"::"` does not resolve and fails rather than binding. An empty
`host` (`""`) is passed to the resolver as a passive (NULL) node and binds every
IPv4 interface, which is equivalent to the `"0.0.0.0"` default.

Only the low 16 bits of `port` reach the socket: the value is written into the
two `sin_port` bytes of the resolved address, so a `port` outside `0..65535` is
truncated modulo 65536 rather than rejected. A `port` of `0` requests an ephemeral
port from the host, which can be read back with `net::localAddress`.

`backlog` is the pending-connection queue hint passed to `listen()`. Because
`listen()` takes a C `int`, a value above `2147483647` is clamped to that maximum
before the call, so a large 64-bit backlog cannot be reinterpreted as negative.
The value is advisory in any case; the host may clamp it further.

The returned listener is a resource: bind it with `RES`, and it is closed by
lexical drop at scope exit (or earlier with `net::close`). Drive it with a
user-owned `DO`/`LOOP` over `http::handleRequest`, which accepts one connection
per call, parses the request, matches its path against an ordered
`List OF http::Route`, invokes the matched handler, writes the response, and
closes the connection. The server is single-threaded and blocking: one request is
served at a time, in the caller's loop. For HTTPS use `http::serverSSL`, which
returns a `tls::TlsListener` that `handleRequest` also accepts."#;

const EX: &str = r#"A minimal server on port 8080:

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
IMPORT io

SUB localOnly()
  RES s AS net::Listener = http::server(8080, "127.0.0.1", 16)
  LET bound = net::localAddress(s)
  io::print("listening on port " & toString(bound.port))
END SUB
```"#;

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_server(port AS Integer, host AS String, backlog AS Integer) AS RES net::Listener
  RETURN net::listenTcp(host, port, backlog)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "server",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer[, String[, Integer]]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("port", "The local TCP port to bind. Only the low 16 bits are used (values outside `0..65535` truncate modulo 65536). `0` requests a host-assigned ephemeral port, readable with `net::localAddress`.", &[], ParameterType::Integer),
                super::fill("host", "Optional local IPv4 interface to bind, as a textual address or a resolvable name. `\"0.0.0.0\"` or `\"\"` bind every IPv4 interface. Defaults to `\"0.0.0.0\"`.", ParameterType::String, "0.0.0.0"),
                super::fill("backlog", "Optional pending-connection queue hint for `listen()`. Values above `2147483647` are clamped to that maximum; the host may clamp further. Defaults to `128`.", ParameterType::Integer, "128"),
            ],
            return_type: ParameterType::named(super::LISTENER_TYPE),
            errors: vec![],
            body: Body::mfb(BODY, "__http_server"),
        }],
    });
}
