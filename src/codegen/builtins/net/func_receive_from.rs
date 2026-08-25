//! `net::receiveFrom` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/receiveFrom.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Receive a single UDP datagram as bytes together with its sender address."#;

const DESC: &str = r#"`net::receiveFrom` receives exactly one datagram from a bound `UdpSocket` and
returns it as a `Datagram` record with two fields: `from`, the sender's
`Address`, and `bytes`, the payload as a `List OF Byte`. Because UDP is
connectionless, one bound socket can receive from many peers, and each call
reports who sent the datagram it returned.

A datagram is delivered whole or not at all. `maxBytes` bounds the payload the
call will accept and must be positive. The receive buffer is deliberately
allocated one byte larger than `maxBytes`, so an oversized datagram is detected
by the host returning more than `maxBytes` bytes and is rejected with
`ErrMessageTooLarge` rather than silently truncated. Size `maxBytes` to the
largest message the protocol expects. The returned list holds the entire payload
and is frequently shorter than `maxBytes`; a zero-length datagram is a valid UDP
message and yields an empty list rather than an error — unlike a TCP read, where
a zero-length result would mean end of stream.

The call blocks until a datagram arrives or the socket's read timeout elapses;
use `net::setReadTimeout` to bound the wait, after which `ErrTimeout` is
raised (the host reporting `EAGAIN` is what distinguishes a timeout from a hard
network failure). A signal that interrupts the receive before any byte moved
re-issues the identical call rather than reporting a spurious failure.

The sender's address is captured alongside the payload and converted into a
freshly allocated `Address`; the payload is copied into a freshly allocated
`List OF Byte`. The socket is borrowed, stays open, and is otherwise untouched.
Use `net::receiveTextFrom` when the payload is UTF-8 text and a `String` is more
convenient than raw bytes, and `net::sendTo` to reply to `from`."#;

const EX: &str = r#"Receive one datagram and report its size:

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

Report the error code when the datagram does not fit:

```
IMPORT net

FUNC recvCount(RES s AS net::UdpSocket, maxBytes AS Integer) AS Integer
  LET dg = net::receiveFrom(s, maxBytes)
  RETURN len(dg.bytes)
  TRAP(e)
    RETURN e.code
  END TRAP
END FUNC

SUB main()
  ' Returns the payload size, or the error code when the datagram exceeds maxBytes
  ' (ErrMessageTooLarge, 77070007) or the read timeout elapses.
END SUB
```"#;

/// `abi_function` body for `net::receive_from` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_receive_from(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_io::lower_net_receive_from_helper(
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
        name: "receiveFrom",
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
            return_type: ParameterType::named(super::DATAGRAM_TYPE),
            errors: vec![],
            body: super::native_body(lower_receive_from, &[]),
        }],
    });
}
