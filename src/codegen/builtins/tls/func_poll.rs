//! `tls::poll` — descriptor entry (native OS-seam).
//!
//! `poll` is **return-type-overloaded on argument shape**: a scalar `TlsSocket`
//! yields `Boolean` (a readiness query), a `List OF RES tls.TlsSocket` yields a
//! borrowed `TlsSocket` (the readiness multiplex). That is two distinct
//! `Implementation`s — the datetime/net idiom — so the registry's generic
//! overload/return resolution answers it with no custom resolver. The list
//! overload declares the code-form alias `pollList`: `builder_values` rewrites a
//! `tls.poll(List …)` NIR call to `tls.pollList`, and the generic OS dispatch
//! routes that alias to this member's lowering (which branches on the call name to
//! the portable list driver).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Test whether a TLS socket has application data ready to read, or wait for the first ready socket among many."#;
const DESC: &str = r#"`tls::poll` reports whether a connected `TlsSocket` is readable — that is, whether
a following `tls::read` or `tls::readText` can proceed without blocking. It returns
`TRUE` when application bytes are available (or the connection has reached a
terminal readable state — peer close or error — where a read returns promptly), and
`FALSE` when nothing became readable before the deadline. The socket is borrowed and
inspected only; no application data is consumed, so a `TRUE` result leaves the bytes
in place for the next read.

**Readiness includes bytes buffered inside the TLS layer, not just raw transport
state.** A single TLS record can carry many application bytes: one decrypt drains a
record and buffers the remainder, so a `TlsSocket` may hold decrypted bytes ready to
read while the underlying transport is idle. `tls::poll` accounts for this — it is
`TRUE` whenever the next read would return bytes, whether they are already buffered
or still on the wire.

`timeoutMs` bounds the wait, in milliseconds, following the language timeout
convention. When it is **omitted, `poll` blocks** until the socket becomes readable
and then returns `TRUE` (omit = unbounded). `0` is a non-blocking check that returns
immediately with the socket's current readiness. A positive value waits up to that
long. A negative `timeoutMs` is rejected with `ErrInvalidArgument`.

Given a `List OF RES tls::TlsSocket`, `tls::poll` becomes a **readiness multiplex**: it
blocks until at least one socket in the list is readable, then returns the first
ready one (lowest list index). The returned `TlsSocket` is a **borrowed** pointer —
an alias of a list element — so the list retains ownership and closes every socket
exactly once on scope exit. An empty list is rejected with `ErrInvalidArgument`;
because the multiplex yields a resource with no not-ready value, expiry raises
`ErrTimeout` (a producing call). The elements must be marked `RES`."#;
const EX: &str = r#"Check whether encrypted data is waiting without blocking (pass `0` for the immediate
check — omitting the timeout would instead block until the socket is readable):

```
IMPORT tls
IMPORT io

FUNC main AS Integer
  RES sock = tls::connect("example.com", 443)
  tls::writeText(sock, "GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n")
  IF tls::poll(sock, 0) THEN
    io::print(tls::readText(sock, 4096))
  END IF
  tls::close(sock)
  RETURN 0
END FUNC
```

Wait for the first ready socket among several (the readiness multiplex). The
returned socket is borrowed — the list still owns and closes both:

```
IMPORT tls
IMPORT io
IMPORT collections

FUNC main AS Integer
  RES a = tls::connect("example.com", 443)
  RES b = tls::connect("example.com", 443)
  MUT socks AS List OF RES tls::TlsSocket = []
  socks = collections::append(socks, a)
  socks = collections::append(socks, b)
  tls::writeText(b, "GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n")
  RES ready AS tls::TlsSocket = tls::poll(socks)
  io::print(tls::readText(ready, 64))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let timeout = |desc: &'static str| Parameter {
        name: "timeoutMs",
        desc,
        aliases: &[],
        ty: ParameterType::Integer,
        default: DefaultValue::Fill {
            type_name: ParameterType::Integer,
            expr: super::SENTINEL,
        },
    };
    pkg.add_function(RegistryFunction {
        name: "poll",
        intro: INTRO,
        desc: DESC,
        example: EX,
        // Return-type-overloaded (Boolean vs TlsSocket): the per-position render
        // yields None, so the union phrasing rides on this hand-authored hint.
        expected_arguments: Some("TlsSocket, Integer or List OF RES TlsSocket, Integer"),
        internal_only: false,
        implementations: vec![
            // Scalar readiness query: `poll(TlsSocket[, timeoutMs]) AS Boolean`.
            Implementation {
                params: vec![
                    Parameter {
                        name: "sock",
                        desc: "An open TLS socket, as returned by `tls::connect` or `tls::accept`. It is borrowed and inspected for readiness only; no data is read and the handle is not consumed.",
                        aliases: &[],
                        ty: ParameterType::Named(super::TLS_SOCKET_TYPE_ID),
                        default: DefaultValue::None,
                    },
                    timeout(
                        "Optional. Omit to block until the socket is readable; `0` is an immediate non-blocking check; a positive value waits up to that many milliseconds. Must not be negative.",
                    ),
                ],
                return_type: ParameterType::Boolean,
                errors: vec![],
                body: Body::native_os_seam(
                    Some(super::native::lower_tls_helper),
                    Some(super::native::lower_tls_helper),
                    &[],
                ),
            },
            // Readiness multiplex: `poll(List OF RES tls.TlsSocket[, timeoutMs]) AS
            // TlsSocket` (borrowed). Emits the `tls.pollList` code form.
            Implementation {
                params: vec![
                    Parameter {
                        name: "socks",
                        desc: "A non-empty list of open TLS sockets. Each is borrowed and inspected for readiness; the list keeps ownership. An empty list raises `ErrInvalidArgument`.",
                        aliases: &[],
                        // The element is the bare resource id: `ParameterType::parse`
                        // strips the `RES ` ownership marker off a list element, so the
                        // concrete `List OF RES tls.TlsSocket` argument unifies as
                        // `ListOf(Named("tls.TlsSocket"))`. (The `RES` requirement itself
                        // is enforced separately by the resource/type checker.)
                        ty: ParameterType::ListOf(Box::new(ParameterType::Named(
                            super::TLS_SOCKET_TYPE_ID,
                        ))),
                        default: DefaultValue::None,
                    },
                    timeout(
                        "Optional. Omit to block until a socket is readable; `0` is an immediate non-blocking scan; a positive value waits up to that many milliseconds. Must not be negative.",
                    ),
                ],
                return_type: ParameterType::Named(super::TLS_SOCKET_TYPE_ID),
                errors: vec![],
                body: Body::native_os_seam(
                    Some(super::native::lower_tls_helper),
                    Some(super::native::lower_tls_helper),
                    &["pollList"],
                ),
            },
        ],
    });
}
