//! `net::remoteAddress` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/remoteAddress.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Report the peer endpoint of a connected TCP socket."#;

const DESC: &str = r#"`net::remoteAddress` asks the host for the address of the peer a connected
`Socket` is talking to and returns it as an `Address` record. It is the peer-side
counterpart of `net::localAddress`, and unlike that function it accepts only a
`Socket`: a `Listener` and a `UdpSocket` have no single peer, so passing either
is a compile-time type error rather than a runtime one.

The call reads the endpoint with `getpeername` into a `sockaddr_storage` and
converts it into a freshly allocated `Address` whose `host` field is the textual
form of the address and whose `port` field is the peer port. The socket is
borrowed, stays open, and is otherwise untouched.

The reported host is the concrete address the host stack is actually connected
to, which is not necessarily the string passed to `net::connectTcp`: a name is
resolved before connecting, so a connection opened to `"example.com"` reports the
resolved IP address here. For a socket from `net::accept` this is how a server
identifies the client that connected."#;

const EX: &str = r#"Inspect the peer endpoint of an outbound connection:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  LET remote = net::remoteAddress(client)
  io::print(remote.host & " " & toString(remote.port))
  RETURN 0
END FUNC
```

Identify the client behind an accepted connection:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  io::print(net::remoteAddress(conn).host)
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `net::remote_address` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_remote_address(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_io::lower_net_address_helper(&symbol, ctx.platform_imports, ctx.platform, true)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
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
            params: vec![super::req("sock", "A connected TCP socket, as returned by `net::connectTcp` or `net::accept`, whose peer endpoint is wanted. It must still be open; the handle is borrowed, not consumed.", &[], super::socket())],
            return_type: ParameterType::named(super::ADDRESS_TYPE),
            errors: vec![],
            body: super::native_body(lower_remote_address, &[]),
        }],
    });
}
