//! `net::listenTcp` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/listenTcp.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Open a TCP listening socket bound to a local address."#;

const DESC: &str = r#"`net::listenTcp` binds a local TCP socket to `host` and `port` and places it in
the listening state, returning a `Listener` ready for `net::accept`. The host is
resolved for a passive `SOCK_STREAM` endpoint, a socket is created from the first
result, `SO_REUSEADDR` is set on it as a best-effort option, the requested port
is patched into the resolved address, and the socket is bound and switched into
the listening state.

`host` names the local interface to bind. An empty `host` binds every interface:
the resolver is called with a null node and the passive flag, and — because a
null node requires a non-null service — with the service string `"0"`, whose port
the requested `port` then overwrites. `"0.0.0.0"` and `"::"` are ordinary
textual wildcard addresses that reach the same result through normal resolution.
When `port` is `0` the host assigns an ephemeral port, which `net::localAddress`
reads back — the usual way to run a server on an unpredictable free port.

`backlog` hints how many pending connections the host may queue before refusing
new ones. It is not a host default when omitted: the compiler fills the missing
third argument with the literal `128`, so the two-argument form is exactly
`net::listenTcp(host, port, 128)`. Because `listen` takes a C `int`, a `backlog`
above 2147483647 is clamped to that value before the call. Beyond that the value
is advisory — the host may cap it at its own limit.

The returned `Listener` is an owned, non-copyable resource handle, closed by
lexical drop when its binding leaves scope or earlier with `net::close`. Each
`net::accept` on it returns an independent `Socket` that outlives the listener.
If binding or listening fails, the partially created descriptor and the resolver
results are released before the error is raised."#;

const EX: &str = r#"Listen on an ephemeral port and read back the assigned port:

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
```"#;

/// `abi_function` body for `net::listen_tcp` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_listen_tcp(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_shared::lower_net_listen_tcp_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "listenTcp",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, Integer, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("host", "The local interface to bind, as a textual IP address or a name passed to the host resolver. `\"0.0.0.0\"`, `\"::\"`, or an empty string bind every interface.", &[], ParameterType::String),
                super::req("port", "The local TCP port to bind. `0` requests an ephemeral port assigned by the host, readable afterwards with `net::localAddress`.", &[], ParameterType::Integer),
                super::opt("backlog", "Optional. A hint for how many pending connections the host may queue. Defaults to `128` when omitted, is clamped to `2147483647`, and may be further capped by the host.", ParameterType::Integer),
            ],
            return_type: ParameterType::named(super::LISTENER_TYPE_ID),
            errors: vec![],
            body: super::native_body(lower_listen_tcp, &[]),
        }],
    });
}
