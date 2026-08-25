//! `net::localAddress` — descriptor entry (native OS-seam). Overloaded over the
//! `Socket` / `Listener` / `UdpSocket` union, all returning `Address`. Docs in
//! `src/docs/man/builtins/net/localAddress.md`.

use crate::codegen::registry::{Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

fn overload(ty: ParameterType) -> Implementation {
    Implementation {
        params: vec![Parameter {
            name: "sock",
            desc: "The connected TCP socket or bound UDP socket whose local endpoint is wanted. It must still be open; the handle is borrowed, not consumed.",
            aliases: &["listener"],
            ty,
            default: crate::codegen::registry::DefaultValue::None,
        }],
        return_type: ParameterType::named(super::ADDRESS_TYPE),
        errors: vec![],
        body: super::native_body(lower_local_address, &[]),
    }
}

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Report the local endpoint bound to a network resource."#;

const DESC: &str = r#"`net::localAddress` asks the host for the address bound to this side of a network
resource and returns it as an `Address` record. It spans all three `net` handle
types: a connected TCP `Socket`, a TCP `Listener`, and a bound UDP `UdpSocket`.
The handle is borrowed, not consumed, and stays open.

The call reads the endpoint with `getsockname` into a `sockaddr_storage`, then
converts it into an `Address` whose `host` field is the textual form of the
address and whose `port` field is the port. The `Address` record is freshly
allocated on each call; the socket itself is untouched.

The most common use is discovering the concrete port behind an ephemeral bind.
After `net::listenTcp(host, 0)` or `net::bindUdp(host, 0)` the host has chosen a
port that the program never named, and `net::localAddress(...).port` is how it is
read back. For a resource bound to a wildcard host the reported host is that
wildcard address, while the port is always the concrete one the host assigned.

Use `net::remoteAddress` for the *peer* endpoint of a connected `Socket`; it is
the only address query that does not accept a `Listener` or a `UdpSocket`,
because only a connected socket has a peer."#;

const EX: &str = r#"Discover the port assigned when listening on port `0`:

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

Inspect the local endpoint of an outbound connection:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  LET local = net::localAddress(client)
  io::print(local.host & " " & toString(local.port))
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `net::local_address` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_local_address(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_io::lower_net_address_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        false,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "localAddress",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket or Listener or UdpSocket"),
        internal_only: false,
        implementations: vec![
            overload(super::socket()),
            overload(super::listener()),
            overload(super::udp()),
        ],
    });
}
