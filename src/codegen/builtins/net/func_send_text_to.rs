//! `net::sendTextTo` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/sendTextTo.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Send a single UDP datagram of UTF-8 text to a destination address."#;

const DESC: &str = r#"`net::sendTextTo` transmits the UTF-8 bytes of `value` as one UDP datagram from a
bound `UdpSocket` to the peer named by `address`. It is the text counterpart of
`net::sendTo`: instead of building a `List OF Byte`, the string's packed byte
data is sent directly from its buffer. A `String` already holds well-formed
UTF-8, so the bytes go out exactly as held, with no re-encoding, decoding, or
newline translation. The socket is borrowed and stays open.

`address` supplies both the destination host and the destination port. The host
is resolved with the host resolver on **every** call — it may be a numeric IP
literal or a name — and the `port` field is then written directly into the
resolved address rather than being resolved as a service name. The resolver's
answer chain is released before the call returns, on both the success and the
failure paths. In a tight send loop, resolve once with `net::lookup` and reuse the
resulting `Address`.

The whole string is sent as the payload of a single datagram in byte order. UDP
preserves message boundaries: the payload arrives whole or not at all, and is
never split across datagrams or merged with another. An empty string sends a
zero-length datagram, which is a valid UDP message rather than a no-op.

A successful return means the datagram was accepted by the host for best-effort
delivery, not that any peer received it. The call may block while the send buffer
is full; use `net::setWriteTimeout` to bound that wait, after which
`ErrTimeout` is raised. A payload larger than the path allows is rejected
with `ErrMessageTooLarge` rather than truncated. A signal that interrupts the
send before any byte left re-issues the identical call — a datagram send is
all-or-nothing, so a send that already completed is never retried.

To reply to a sender, pass the `from` field of the `DatagramText` returned by
`net::receiveTextFrom` (or of the `Datagram` from `net::receiveFrom`; both carry
the same `Address`). The text payload of a `DatagramText` is its `value` field."#;

const EX: &str = r#"Send a line of text to a resolved destination and receive it:

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
  net::sendTextTo(client, dest, "hello")
  LET dg = net::receiveTextFrom(server, 64)
  io::print(dg.value)
  RETURN 0
END FUNC
```

Echo received text back to its sender:

```
IMPORT net

FUNC echoOne(RES sock AS net::UdpSocket) AS Integer
  LET dg = net::receiveTextFrom(sock, 1024)
  net::sendTextTo(sock, dg.from, dg.value)
  RETURN len(dg.value)
  TRAP(e)
    RETURN e.code
  END TRAP
END FUNC

SUB main()
  ' Echoes one text datagram back to its sender; `from` is the sender Address and
  ' `value` is the decoded payload.
END SUB
```"#;

/// `abi_function` body for `net::send_text_to` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_send_text_to(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_io::lower_net_send_to_helper(&symbol, ctx.platform_imports, ctx.platform, true)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sendTextTo",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("UdpSocket, Address, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", "A bound UDP socket to send from, as returned by `net::bindUdp`. It must still be open; the handle is borrowed, not consumed.", &[], super::udp()),
                super::req("address", "The destination. Its `host` field is resolved on each call and may be a numeric IP literal or a name; its `port` field selects the destination port. Obtain one from `net::lookup`, or from the `from` field of a received `Datagram` or `DatagramText`.", &[], ParameterType::named(super::ADDRESS_TYPE)),
                super::req("value", "The text to send, transmitted as its UTF-8 bytes in order as one datagram. An empty string sends a valid zero-length datagram.", &[], ParameterType::String),
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: super::native_body(lower_send_text_to, &[]),
        }],
    });
}
