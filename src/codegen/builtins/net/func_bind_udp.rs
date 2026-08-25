//! `net::bindUdp` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/bindUdp.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Open a UDP datagram socket bound to a local address."#;

const DESC: &str = r#"`net::bindUdp` creates a connectionless UDP datagram socket bound to a local
endpoint and returns a `UdpSocket` resource ready to send and receive datagrams.
The call resolves `host` with the host resolver requesting a `SOCK_DGRAM`
endpoint, creates a socket from the first resolved result, patches the requested
`port` into the resolved address, and binds it.

`host` names the local interface to bind, given as a textual IP address or a name
handed to the resolver. An empty `host` binds every interface: the resolver is
called with a null node and the passive flag, and — because a null node requires
a non-null service — with the service string `"0"`, whose port is then overwritten
by the requested `port`. `"0.0.0.0"` and `"::"` are ordinary textual wildcard
addresses that reach the same result through normal resolution. When `port` is
`0` the host assigns an ephemeral port, which `net::localAddress` reads back.

Unlike TCP there is no listen or accept step: a UDP socket is not tied to a
single peer. Send datagrams with `net::sendTo` or `net::sendTextTo`, each naming
its own destination, and receive them with `net::receiveFrom` or
`net::receiveTextFrom`, which report the sender's `Address` alongside the
payload. Bound how long a receive or send may block with `net::setReadTimeout`
and `net::setWriteTimeout`.

The returned `UdpSocket` is an owned, non-copyable resource handle. It is closed
by lexical drop when its binding leaves scope, or earlier with `net::close`; it
cannot be stored in a collection or a record. If the socket cannot be created or
bound, the partially created descriptor and the resolver results are released
before the error is raised, so a failed `bindUdp` leaks neither."#;

const EX: &str = r#"Bind an ephemeral port and read back the assigned address:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES sock = net::bindUdp("127.0.0.1", 0)
  LET bound = net::localAddress(sock)
  io::print(toString(bound.port))
  RETURN 0
END FUNC
```

Send a datagram between two bound sockets:

```
IMPORT collections
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::bindUdp("127.0.0.1", 0)
  net::setReadTimeout(server, 2000)
  LET bound = net::localAddress(server)
  RES client = net::bindUdp("127.0.0.1", 0)
  LET dest = collections::get(net::lookup("127.0.0.1", bound.port), 0)
  net::sendTextTo(client, dest, "ping")
  LET dg = net::receiveTextFrom(server, 64)
  io::print(dg.value)
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `net::bind_udp` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_bind_udp(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_io::lower_net_bind_udp_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "bindUdp",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("host", "The local interface to bind, as a textual IP address or a name passed to the host resolver. `\"0.0.0.0\"`, `\"::\"`, or an empty string bind every interface.", &[], ParameterType::String),
                super::req("port", "The local UDP port to bind. `0` requests an ephemeral port assigned by the host, readable afterwards with `net::localAddress`.", &[], ParameterType::Integer),
            ],
            return_type: ParameterType::named(super::UDP_SOCKET_TYPE_ID),
            errors: vec![],
            body: super::native_body(lower_bind_udp, &[]),
        }],
    });
}
