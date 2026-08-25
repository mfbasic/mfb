//! `net::receiveTextFrom` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/receiveTextFrom.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str =
    r#"Receive a single UDP datagram as UTF-8 text together with its sender address."#;

const DESC: &str = r#"`net::receiveTextFrom` receives exactly one datagram from a bound `UdpSocket` and
returns it as a `DatagramText` record with two fields: `from`, the sender's
`Address`, and `value`, the payload decoded as a UTF-8 `String`. Note the field
name — the text payload is `value`, not `text`, and its byte-oriented counterpart
in `Datagram` is `bytes`. Because UDP is connectionless, one bound socket can
receive from many peers, and each call reports who sent the datagram it returned.

A datagram is delivered whole or not at all. `maxBytes` bounds the payload the
call will accept and must be positive. The receive buffer is deliberately
allocated one byte larger than `maxBytes`, so an oversized datagram is detected
and rejected with `ErrMessageTooLarge` rather than silently truncated. The
returned string holds the entire payload and is frequently shorter than
`maxBytes` bytes; a zero-length datagram is a valid UDP message and yields an
empty string rather than an error.

The payload bytes are validated as UTF-8 before the string is returned, and
invalid bytes raise `ErrEncoding`. Unlike `net::readText` on a TCP stream this is
not a framing hazard: a datagram is received whole, so a multi-byte UTF-8
sequence is never split across two calls. Use `net::receiveFrom` when the payload
is raw binary and a `List OF Byte` is the right shape.

The call blocks until a datagram arrives or the socket's read timeout elapses;
use `net::setReadTimeout` to bound the wait, after which `ErrTimeout` is
raised. A signal that interrupts the receive before any byte moved re-issues the
identical call. The sender's address and the decoded payload are each freshly
allocated; the socket is borrowed, stays open, and is otherwise untouched. Reply
to `from` with `net::sendTextTo`."#;

const EX: &str = r#"Receive one text datagram and print its payload:

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
```

Bound the wait and report the error code on a timeout:

```
IMPORT net

FUNC recvOrCode(RES s AS net::UdpSocket) AS String
  LET dg = net::receiveTextFrom(s, 512)
  RETURN dg.value
  TRAP(e)
    RETURN toString(e.code)
  END TRAP
END FUNC

SUB main()
  ' Returns the datagram text, or the error code — 77050008 (ErrTimeout) when
  ' nothing arrived before the read timeout elapsed.
END SUB
```"#;

/// `abi_function` body for `net::receive_text_from` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_receive_text_from(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_io::lower_net_receive_from_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        true,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "receiveTextFrom",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("UdpSocket, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", "A bound UDP socket to receive on, as returned by `net::bindUdp`. It must still be open; the handle is borrowed, not consumed.", &[], super::udp()),
                super::req("maxBytes", "The largest payload the call will accept, in bytes. Must be positive. A datagram exceeding it is rejected with `ErrMessageTooLarge`, never truncated.", &[], ParameterType::Integer),
            ],
            return_type: ParameterType::named(super::DATAGRAM_TEXT_TYPE),
            errors: vec![],
            body: super::native_body(lower_receive_text_from, &[]),
        }],
    });
}
