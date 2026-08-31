//! `tcp::remoteAddress` — descriptor entry (native OS-seam, plan-110-B). Sockets
//! only: a listener has no single peer.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::shared as gen_shared;
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Report the peer's address on a connected socket."#;

const DESC: &str = r#"`tcp::remoteAddress` returns the `net::Address` of the peer at the far end of a
connected `Socket` — the address and port the connection actually reached.

For a socket from `tcp::accept` this is how a server learns who connected, which
is what logging and per-client policy are usually built on. For a socket from
`tcp::connect` it reports the address the name resolved to, which can differ from
the host string that was passed in.

There is no listener overload: a listener has no single peer. Use
`tcp::localAddress` for the address it is bound to.

The socket stays open — you still close it — and the returned value is an ordinary
`net::Address` independent of the socket.

**A file that uses the returned address must `IMPORT net` as well as `tcp`.**
Imports are not transitive and packages cannot re-export types, so `Address` is
nameable only where `net` is imported."#;

const EX: &str = r#"Log who connected:

```
IMPORT tcp
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  RES client = tcp::connect("127.0.0.1", bound.port)
  RES conn = tcp::accept(server)
  LET peer = tcp::remoteAddress(conn)
  io::print("client from " & peer.host)
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `tcp::remoteAddress` (`remote = true` → `getpeername`).
pub(crate) fn lower_remote_address(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        crate::codegen::os::socket::shared::lower_net_address_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
            true,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "remoteAddress",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req(
                "sock",
                "An open connected socket whose peer to report. The handle stays open — you still close it.",
                &[],
                super::socket(),
            )],
            return_type: super::address(),
            errors: vec![],
            body: super::native_body(lower_remote_address, &[]),
        }],
    });
}
