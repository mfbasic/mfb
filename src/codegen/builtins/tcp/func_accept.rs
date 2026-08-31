//! `tcp::accept` — descriptor entry (native OS-seam, plan-110-B).

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::gen_io;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::shared as gen_shared;
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Accept the next pending connection on a TCP listener."#;

const DESC: &str = r#"`tcp::accept` removes the next pending connection from a `Listener`'s queue and
returns a connected `Socket` for talking to that client. Each call accepts a
single connection, so a server loops over `accept` to serve clients as they
arrive. The listener is *borrowed*, not consumed: it stays open and usable.

`timeoutMs` follows the language timeout convention (see `mfb spec language
builtin-functions` → "Timeout convention"). **Omitted, the call blocks**
indefinitely until a client connects. `0` is one immediate attempt: it returns a
pending connection if one is already queued, otherwise it raises `ErrTimeout`
without waiting. A positive value bounds the wait (clamped to `2147483647`) and
raises `ErrTimeout` if no client arrives. A negative value raises
`ErrInvalidArgument`.

On the bounded path the listener is temporarily switched into non-blocking mode
and its original file-status flags are restored before returning, on every exit
path. This matters when a connection the readiness poll saw is aborted by the
peer, or taken by another thread, between the poll and the accept: the accept
then reports "would block" and the call re-enters the poll rather than blocking
for the *next* client and overrunning `timeoutMs`. A signal that interrupts
either the poll or the accept re-issues it instead of surfacing a spurious
failure.

The returned `Socket` is a fully independent resource: it stays usable after the
listener is closed, and closing it does not affect the listener."#;

const EX: &str = r#"Accept a single client and read its request:

```
IMPORT tcp
IMPORT encoding
IMPORT io

FUNC main AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  RES client = tcp::connect("127.0.0.1", bound.port)
  RES conn = tcp::accept(server)
  tcp::write(client, "hello")
  io::print(encoding::utf8Decode(tcp::read(conn, 16)))
  RETURN 0
END FUNC
```

Bound how long a server waits for a client:

```
IMPORT tcp
IMPORT io

FUNC main AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  RES conn = tcp::accept(server, 500)
  io::print("accepted")
  RETURN 0
  TRAP(e)
    io::print("no client within 500ms")
    RETURN 0
  END TRAP
END FUNC
```"#;

/// `abi_function` body for `tcp::accept`.
pub(crate) fn lower_accept(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        gen_io::lower_net_accept_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
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
                super::req(
                    "listener",
                    "An open listener from `tcp::listen`. Borrowed, not consumed, and available for further `accept` calls.",
                    &[],
                    super::listener(),
                ),
                super::opt(
                    "timeoutMs",
                    "Optional. The maximum time to wait for a pending connection, in milliseconds. Omit to block indefinitely; `0` is one immediate attempt (`ErrTimeout` if none pending); a positive value that elapses raises `ErrTimeout` (clamped to `2147483647`); a negative value raises `ErrInvalidArgument`.",
                    ParameterType::Integer,
                ),
            ],
            return_type: super::socket(),
            errors: vec![],
            body: super::native_body(lower_accept, &[]),
        }],
    });
}
