//! `udp::receive` — descriptor entry (native OS-seam, plan-110-C). Bytes only;
//! `net::receiveTextFrom` and its `DatagramText` shape deliberately do not
//! survive.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::gen_io;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::shared as gen_shared;
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Receive the next datagram, with the address it came from."#;

const DESC: &str = r#"`udp::receive` returns the next datagram queued on a bound socket as a
`Datagram`: the payload `bytes` exactly as sent, and the `from` address that sent
them. Because a bound socket has no peer, `from` is the only way to know who is
talking, and it is an ordinary `net::Address` that can be passed straight back to
`udp::send` to reply.

**One `receive` returns exactly one datagram** — never a partial one, never two
run together. `bytes` is empty when the sender sent an empty datagram, which is
ordinary traffic and not an end-of-stream: UDP has no such concept.

`maxBytes` must be positive and bounds the payload accepted. A datagram larger
than `maxBytes` **raises `ErrMessageTooLarge`** rather than being truncated,
because a truncated datagram is a corrupted message with no signal that anything
was lost — the caller would have no way to tell a short message from a clipped
one. Size the buffer for the largest datagram the protocol permits.

How long a receive waits is governed by `udp::setReadTimeout`; without one the
call blocks until a datagram arrives. Use `udp::poll` to ask whether one is
waiting without committing to a receive.

**There is no text form.** The network reports nothing about a payload's
encoding, so decoding on receipt would either guess or raise on perfectly valid
binary traffic. Decode with `encoding::utf8Decode` when the protocol says the
payload is text. Sending is not symmetric — `udp::send` takes a `String` —
because the sender always knows what it is sending."#;

const EX: &str = r#"Serve one request and reply to its sender:

```
IMPORT encoding
IMPORT net
IMPORT udp
IMPORT io

FUNC main AS Integer
  RES server = udp::bind("127.0.0.1", 0)
  RES client = udp::bind("127.0.0.1", 0)
  udp::send(client, udp::localAddress(server), "ping")

  LET request = udp::receive(server, 1024)
  io::print("from port " & toString(request.from.port))
  io::print("said " & encoding::utf8Decode(request.bytes))

  udp::send(server, request.from, "pong")
  io::print("reply " & encoding::utf8Decode(udp::receive(client, 1024).bytes))
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `udp::receive` — the byte form only (`text = false`).
pub(crate) fn lower_receive(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        gen_io::lower_net_receive_from_helper(&symbol, ctx.platform_imports, ctx.platform, false)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "receive",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req(
                    "sock",
                    "An open bound socket to receive on. Borrowed, not consumed.",
                    &[],
                    super::socket(),
                ),
                super::req(
                    "maxBytes",
                    "The largest payload to accept. Must be positive; a larger datagram raises `ErrMessageTooLarge` rather than being truncated.",
                    &[],
                    ParameterType::Integer,
                ),
            ],
            return_type: ParameterType::named(super::DATAGRAM_TYPE),
            errors: vec![],
            body: super::native_body(lower_receive, &[]),
        }],
    });
}
