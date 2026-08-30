//! `udp::send` — descriptor entry (native OS-seam, plan-110-C). Two overloads
//! sharing a name but not a lowering: bytes go through `udp.send`, a `String`
//! through the `udp.sendText` code form. `builder_values` selects on the third
//! argument's static type.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::gen_io;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::shared as gen_shared;
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Send one datagram to an address."#;

const DESC: &str = r#"`udp::send` addresses a single datagram to a peer. The payload is either a
`List OF Byte` or a `String`, which is sent as its UTF-8 bytes with nothing added
— no length prefix, terminator, or encoding declaration.

**One `send` is exactly one datagram.** It is never split across several, never
merged with the next, and arrives whole or not at all. That boundary is the whole
point of UDP, and it is why a caller does not have to frame messages the way a
`tcp` stream requires.

A zero-length payload is valid and sends a real, empty datagram.

Success means the local OS accepted the datagram for sending — nothing more. UDP
has no acknowledgement, so there is no way to learn from `send` whether the peer
received it, and no retransmission if it did not. A protocol that needs delivery
guarantees builds them on top, or uses `tcp`.

`udp::setWriteTimeout` bounds how long a blocked send may wait; without one the
call blocks until the OS accepts the datagram, which in practice is immediate
unless the send buffer is full.

An oversized payload raises rather than being truncated: silently dropping the
tail would corrupt the message with no signal."#;

const EX: &str = r#"Send to a resolved address, then reply to whoever answers:

```
IMPORT collections
IMPORT net
IMPORT udp
IMPORT io

FUNC main AS Integer
  RES server = udp::bind("127.0.0.1", 0)
  RES client = udp::bind("127.0.0.1", 0)
  LET serverAt = udp::localAddress(server)

  udp::send(client, serverAt, "ping")
  LET request = udp::receive(server, 64)

  ' `request.from` is an ordinary net::Address -- reply straight to it.
  LET payload AS List OF Byte = [80, 79, 78, 71]
  udp::send(server, request.from, payload)
  io::print(toString(len(udp::receive(client, 64).bytes)))
  RETURN 0
END FUNC
```"#;

const SOCK_DESC: &str = "An open bound socket to send from. Borrowed, not consumed.";
const TO_DESC: &str = "The destination address, typically from `net::lookup` or from a received datagram's `from` field.";

/// `abi_function` body for `udp::send`. The `String` overload arrives under the
/// `udp.sendText` code form — the same emitter in its text mode.
pub(crate) fn lower_send(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let text = ctx.call == "udp.sendText";
    let (instructions, relocations, stack_size) =
        gen_io::lower_net_send_to_helper(&symbol, ctx.platform_imports, ctx.platform, text)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "send",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket, Address, List OF Byte or Socket, Address, String"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    super::req("sock", SOCK_DESC, &[], super::socket()),
                    super::req("to", TO_DESC, &[], super::address()),
                    super::req(
                        "bytes",
                        "The payload. Sent as exactly one datagram; a zero-length list sends a real empty datagram.",
                        &[],
                        ParameterType::list_of(ParameterType::Byte),
                    ),
                ],
                return_type: ParameterType::Nothing,
                errors: vec![],
                body: super::native_body(lower_send, &[]),
            },
            Implementation {
                params: vec![
                    super::req("sock", SOCK_DESC, &[], super::socket()),
                    super::req("to", TO_DESC, &[], super::address()),
                    super::req(
                        "text",
                        "The payload as text. Sent as its UTF-8 bytes in exactly one datagram, with no terminator or length prefix added.",
                        &[],
                        ParameterType::String,
                    ),
                ],
                return_type: ParameterType::Nothing,
                errors: vec![],
                body: super::native_body(lower_send, &["sendText"]),
            },
        ],
    });
}
