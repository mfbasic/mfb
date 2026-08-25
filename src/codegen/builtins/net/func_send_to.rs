//! `net::sendTo` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/sendTo.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Send a single UDP datagram of bytes to a destination address."#;

const DESC: &str = r#"`net::sendTo` transmits the contents of `bytes` as one UDP datagram from a bound
`UdpSocket` to the peer named by `address`. Because a UDP socket is not tied to a
single peer, each call names its own destination and the same socket can address
many peers in turn. The socket is borrowed and stays open.

`address` supplies both the destination host and the destination port. The host
is resolved with the host resolver on **every** call — it may be a numeric IP
literal or a name — and the `port` field is then written directly into the
resolved address rather than being resolved as a service name. The resolver's
answer chain is released before the call returns, on both the success and the
failure paths. Note the per-call resolution cost: in a tight send loop, resolve
once with `net::lookup` and reuse the resulting `Address`.

The whole list is sent as the payload of a single datagram, read directly out of
the list's inline data region in list order. UDP preserves message boundaries:
the payload arrives whole or not at all, and is never split across datagrams or
merged with another. An empty list sends a zero-length datagram, which is a valid
UDP message rather than a no-op.

A successful return means the datagram was accepted by the host for best-effort
delivery, not that any peer received it. The call may block while the send buffer
is full; use `net::setWriteTimeout` to bound that wait, after which
`ErrTimeout` is raised. A payload larger than the path allows is rejected
with `ErrMessageTooLarge` rather than truncated. A signal that interrupts the
send before any byte left re-issues the identical call — a datagram send is
all-or-nothing, so a send that already completed is never retried.

Use `net::sendTextTo` instead when sending UTF-8 text from a `String` is more
convenient than building a `List OF Byte`."#;

const EX: &str = r#"Send a datagram to a resolved destination and receive it:

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
  LET payload AS List OF Byte = [10, 20, 30, 40]
  net::sendTo(client, dest, payload)
  LET dg = net::receiveFrom(server, 16)
  io::print(toString(len(dg.bytes)))
  RETURN 0
END FUNC
```

Reply to whoever sent a datagram:

```
IMPORT net

FUNC echoOne(RES sock AS net::UdpSocket) AS Integer
  LET dg = net::receiveFrom(sock, 1024)
  net::sendTo(sock, dg.from, dg.bytes)
  RETURN len(dg.bytes)
  TRAP(e)
    RETURN e.code
  END TRAP
END FUNC

SUB main()
  ' Echoes one datagram back to its sender using the Datagram's `from` address.
END SUB
```"#;

/// `abi_function` body for `net::send_to` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_send_to(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_io::lower_net_send_to_helper(
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
        name: "sendTo",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("UdpSocket, Address, List OF Byte"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", "A bound UDP socket to send from, as returned by `net::bindUdp`. It must still be open; the handle is borrowed, not consumed.", &[], super::udp()),
                super::req("address", "The destination. Its `host` field is resolved on each call and may be a numeric IP literal or a name; its `port` field selects the destination port. Obtain one from `net::lookup`, or use the `from` field of the `Datagram` returned by `net::receiveFrom` to reply to a sender.", &[], ParameterType::named(super::ADDRESS_TYPE)),
                super::req("bytes", "The payload, sent in list order as one datagram. An empty list sends a valid zero-length datagram.", &[], ParameterType::list_of(ParameterType::Byte)),
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: super::native_body(lower_send_to, &[]),
        }],
    });
}
