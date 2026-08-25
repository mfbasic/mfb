//! `net::accept` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/accept.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Accept the next pending connection on a TCP listener."#;

const DESC: &str = r#"`net::accept` removes the next pending connection from a `Listener`'s queue and
returns a connected `Socket` for talking to that client. The listener must have
been placed in the listening state by `net::listenTcp` and must still be open.
Each call accepts a single connection, so a server loops over `accept` to serve
clients as they arrive. The listener is *borrowed*, not consumed: it stays open
and usable for further accepts.

The optional `timeoutMs` follows the language timeout convention (see
`mfb spec language builtin-functions` → "Timeout convention"). When it is
**omitted the call blocks** indefinitely until a client connects. `0` is one
immediate attempt: it returns a pending connection if one is already queued,
otherwise it raises `ErrTimeout` without waiting. A positive `timeoutMs` polls
the listener against that deadline (clamped to `2147483647`) and raises
`ErrTimeout` if no client arrives first. A negative `timeoutMs` raises
`ErrInvalidArgument`.

On the bounded path the listener is temporarily switched into non-blocking mode
for the duration of the call and its original file-status flags are restored
before the call returns, on every exit path. This matters when a connection that
the readiness poll saw is aborted by the peer, or is taken by another thread,
between the poll and the accept: the accept then reports `EAGAIN` and the call
re-enters the poll rather than blocking for the *next* client and overrunning
`timeoutMs`. A signal that interrupts either the poll or the accept re-issues the
same call instead of surfacing a spurious failure.

The returned `Socket` is a fully independent resource: it stays usable after the
listener is closed, and closing it does not affect the listener. Like every
`net` handle it is closed by lexical drop when its binding leaves scope, or
earlier with `net::close`. Read and write it with `net::read`, `net::readText`,
`net::write`, and `net::writeText`, and inspect its endpoints with
`net::localAddress` and `net::remoteAddress`."#;

const EX: &str = r#"Accept a single client and read a request:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  net::writeText(client, "hello")
  io::print(net::readText(conn, 16))
  RETURN 0
END FUNC
```

Bound how long a server waits for a client:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  RES conn = net::accept(server, 500)
  io::print("accepted")
  RETURN 0
  TRAP(e)
    io::print(toString(e.code))
    RETURN 0
  END TRAP
END FUNC
```"#;

/// `abi_function` body for `net::accept` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_accept(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_io::lower_net_accept_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "accept",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Listener, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("listener", "An open listener in the listening state, as returned by `net::listenTcp`. It is borrowed, not consumed, and remains available for further `accept` calls.", &[], super::listener()),
                super::opt("timeoutMs", "Optional. The maximum time to wait for a pending connection, in milliseconds. Omit to block indefinitely; `0` is one immediate attempt (`ErrTimeout` if none pending); a positive value that elapses raises `ErrTimeout` (clamped to `2147483647`); a negative value raises `ErrInvalidArgument`.", ParameterType::Integer),
            ],
            return_type: ParameterType::named(super::SOCKET_TYPE_ID),
            errors: vec![],
            body: super::native_body(lower_accept, &[]),
        }],
    });
}
