//! `tls::accept` — descriptor entry (native OS-seam).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Accept one inbound connection and complete the server-side TLS handshake."#;
const DESC: &str = r#"`accept` takes the next inbound TCP connection on a `TlsListener`, runs the
**server side** of the TLS handshake using the listener's loaded certificate and
key, and returns a connected `TlsSocket`. The returned socket is
indistinguishable from a client `TlsSocket`: read and write it with `tls::read`,
`tls::readText`, `tls::write`, and `tls::writeText`, and close it with
`tls::close` or by lexical drop.

The `listener` is **borrowed**, not consumed: it stays open for the next
`accept`, so a server loops on one listener to serve many connections. The
accepted socket borrows the listener's shared server TLS context; closing the
socket never frees that context (the listener owns it and frees it once, when the
listener closes), so accepted sockets may be closed in any order while the
listener and its siblings stay live.

The optional `timeoutMs` bounds how long `accept` waits for both an inbound
connection and the handshake to complete, following the language timeout
convention. When it is **omitted, `accept` blocks** until a connection is ready
and handshaken. `0` is one immediate attempt. A positive value fails with
`ErrTimeout` if no connection arrives, or the handshake does not finish, within
that many milliseconds. A negative `timeoutMs` raises `ErrInvalidArgument`. A
handshake that fails raises `ErrTlsFailed`, and the accepted connection is closed
before the error is returned; the listener stays open, so the server can continue
accepting.

This version presents the server certificate but does not request or verify a
client certificate (no mutual TLS)."#;
const EX: &str = r#"Serve connections in a loop, one request/response each:

```
IMPORT tls
IMPORT io

SUB main()
  RES server = tls::listen("", 8443, "cert.pem", "key.pem")
  WHILE TRUE
    RES client = tls::accept(server)
    LET request = tls::readText(client, 4096)
    tls::writeText(client, "HTTP/1.0 200 OK\r\n\r\nhi")
    tls::close(client)
  END WHILE
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "accept",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("TlsListener, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "listener",
                    desc: "A listening `TlsListener` from `tls::listen`. Borrowed, not consumed: it remains open for further `accept` calls.",
                    aliases: &[],
                    ty: ParameterType::Named(super::TLS_LISTENER_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "timeoutMs",
                    desc: "Optional. The maximum time to wait for a connection and its handshake, in milliseconds. Omit to block until ready; `0` is one immediate attempt; a positive value bounds the wait; a negative value raises `ErrInvalidArgument`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::Integer,
                        expr: super::SENTINEL,
                    },
                },
            ],
            return_type: ParameterType::Named(super::TLS_SOCKET_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(super::gen_os_seam::lower_tls_os_seam),
        }],
    });
}
