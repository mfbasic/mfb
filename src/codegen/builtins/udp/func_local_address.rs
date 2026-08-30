//! `udp::localAddress` — descriptor entry (native OS-seam, plan-110-C).

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};

use crate::codegen::builtins::net::{gen_io, gen_shared};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Report the local address and port a UDP socket is bound to."#;

const DESC: &str = r#"`udp::localAddress` returns the `net::Address` the OS assigned to a bound socket.

Its main use is learning the port after binding port `0`. Asking the OS for a free
port and reading back which one it chose is the only race-free way to bind, and a
UDP client that wants replies needs to tell the peer where to send them — this is
where that address comes from.

There is no `remoteAddress`: a bound datagram socket has no peer. The address a
datagram came from is reported per-datagram, in the `from` field of what
`udp::receive` returns.

The socket is borrowed, not consumed, and the result is an ordinary `net::Address`
with no tie to the socket's lifetime.

**A file that uses the returned address must `IMPORT net` as well as `udp`.**
Imports are not transitive and packages cannot re-export types, so `Address` is
nameable only where `net` is imported."#;

const EX: &str = r#"Tell a peer where to reply:

```
IMPORT net
IMPORT udp
IMPORT io

FUNC main AS Integer
  RES sock = udp::bind("127.0.0.1", 0)
  LET at = udp::localAddress(sock)
  io::print(at.host & ":" & toString(at.port))
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `udp::localAddress` (`remote = false` → `getsockname`).
pub(crate) fn lower_local_address(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        gen_io::lower_net_address_helper(&symbol, ctx.platform_imports, ctx.platform, false)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "localAddress",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![super::req(
                "sock",
                "An open bound socket whose local address to report. Borrowed, not consumed.",
                &[],
                super::socket(),
            )],
            return_type: super::address(),
            errors: vec![],
            body: super::native_body(lower_local_address, &[]),
        }],
    });
}
