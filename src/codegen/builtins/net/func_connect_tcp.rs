//! `net::connectTcp` — descriptor entry (native OS-seam). Four argument-shape
//! overloads (host/port, host/port/timeout, address, address/timeout); the two
//! `Address` forms lower to the `net.connectTcpAddr` code form (an `os_alias`), the
//! others to `net.connectTcp`. The overload split + timeout padding lives in
//! `builder_values`. Docs in `src/docs/man/builtins/net/connectTcp.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Open a TCP connection to a host and port or to a resolved address."#;

const DESC: &str = r#"`net::connectTcp` establishes an outbound TCP connection and returns a connected
`Socket`. The peer is named either by a host string plus a port, or by an
`Address` record whose `host` and `port` fields supply both. When `host` is a
name rather than a textual IP address it is resolved with the host resolver
before connecting, and the first resolved result is used; the requested port is
written into that address rather than being resolved as a service name.

Every connect takes the same non-blocking-connect plus readiness-poll path. The
socket is switched to non-blocking mode, `connect` is issued, and the call then
polls for writability against a deadline; on success the original blocking mode
is restored and the socket's `SO_ERROR` is checked before the handle is built, so
a connection that failed asynchronously is reported as a failure rather than
handed back as connected. A signal that interrupts the poll re-issues it instead
of surfacing a spurious error.

`timeoutMs` selects that deadline, following the language timeout convention (see
`mfb spec language builtin-functions` → "Timeout convention"). When it is
**omitted the connect blocks** until the connection completes or the OS refuses it
(there is no built-in bounded default any more). `0` is one immediate,
non-blocking attempt: it succeeds if the connect completes at once, otherwise it
raises `ErrTimeout` without waiting. A positive value bounds the attempt and
raises `ErrTimeout` on the deadline. A negative `timeoutMs` raises
`ErrInvalidArgument`. On any failure the pending descriptor and the resolver
results are released first. Because `poll` takes a C `int`, a deadline above
2147483647 milliseconds is clamped to that value. **A caller that must not wedge
on a black-holed peer must pass a positive `timeoutMs`** — the former 120000 ms
safety default is gone.

The four overloads do not share a positional layout: `timeoutMs` is parameter 2
of the host/port forms but parameter 1 of the `Address` forms. Named arguments
therefore bind per-overload, against the parameter list of whichever overload the
argument types select.

The returned `Socket` is an owned, non-copyable resource handle, closed by
lexical drop when its binding leaves scope or earlier with `net::close`. Read
and write it with `net::read`, `net::readText`, `net::write`, and
`net::writeText`, bound its blocking with `net::setReadTimeout` and
`net::setWriteTimeout`, and inspect its endpoints with `net::localAddress` and
`net::remoteAddress`."#;

const EX: &str = r#"Connect to a local listener by host and port:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  io::print(toString(net::remoteAddress(client).port))
  RETURN 0
END FUNC
```

Connect to a resolved `Address` with an explicit deadline:

```
IMPORT collections
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  LET dest = collections::get(net::lookup("127.0.0.1", bound.port), 0)
  RES client = net::connectTcp(dest, 5000)
  io::print("connected")
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `net::connect_tcp` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_connect_tcp(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = if ctx.call == "net.connectTcpAddr" {
        super::gen_shared::lower_net_connect_tcp_addr_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?
    } else {
        super::gen_shared::lower_net_connect_tcp_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let ret = || ParameterType::named(super::SOCKET_TYPE_ID);
    pkg.add_function(RegistryFunction {
        name: "connectTcp",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, Integer, Integer or Address, Integer"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    super::req("host", "The peer's host name or textual IP address. Passed to the host resolver; a name with no address record raises an error.", &[], ParameterType::String),
                    super::req("port", "The TCP port to connect to on the peer. Written directly into the resolved address.", &[], ParameterType::Integer),
                ],
                return_type: ret(),
                errors: vec![],
                body: super::native_body(lower_connect_tcp, &[]),
            },
            Implementation {
                params: vec![
                    super::req("host", "The peer's host name or textual IP address. Passed to the host resolver; a name with no address record raises an error.", &[], ParameterType::String),
                    super::req("port", "The TCP port to connect to on the peer. Written directly into the resolved address.", &[], ParameterType::Integer),
                    super::req("timeoutMs", "Optional. The maximum time the connection attempt may take, in milliseconds. Omit to block until the connection resolves; `0` is one immediate attempt (`ErrTimeout` unless it completes at once); a positive value bounds the attempt and raises `ErrTimeout` when it elapses (clamped to `2147483647`); a negative value raises `ErrInvalidArgument`.", &[], ParameterType::Integer),
                ],
                return_type: ret(),
                errors: vec![],
                body: super::native_body(lower_connect_tcp, &[]),
            },
            Implementation {
                params: vec![super::req("address", "A destination record supplying both the peer host and the peer port, typically from `net::lookup`. Replaces the separate `host` and `port` arguments.",
                    &[],
                    ParameterType::named(super::ADDRESS_TYPE),
                )],
                return_type: ret(),
                errors: vec![],
                body: super::native_body(lower_connect_tcp, &["connectTcpAddr"]),
            },
            Implementation {
                params: vec![
                    super::req("address", "A destination record supplying both the peer host and the peer port, typically from `net::lookup`. Replaces the separate `host` and `port` arguments.", &[], ParameterType::named(super::ADDRESS_TYPE)),
                    super::req("timeoutMs", "Optional. The maximum time the connection attempt may take, in milliseconds. Omit to block until the connection resolves; `0` is one immediate attempt (`ErrTimeout` unless it completes at once); a positive value bounds the attempt and raises `ErrTimeout` when it elapses (clamped to `2147483647`); a negative value raises `ErrInvalidArgument`.", &[], ParameterType::Integer),
                ],
                return_type: ret(),
                errors: vec![],
                body: super::native_body(lower_connect_tcp, &[]),
            },
        ],
    });
}
