//! `net::write` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/write.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Write bytes to a connected socket."#;

const DESC: &str = r#"`net::write` sends the raw bytes of `bytes` over a connected `Socket`. It writes
the *entire* list before returning: the call loops, advancing a cursor past
whatever each underlying host write accepted and re-issuing the write for the
remainder, so a short host write is resumed rather than mistaken for completion.
When the call returns successfully, every byte has been handed to the socket's
send buffer — which is not a guarantee that the peer has received or read them.

The bytes are read directly out of the list's inline data region and sent in list
order, with no copy, re-encoding, or newline translation. An empty list writes
nothing and returns immediately, because the loop's remaining-byte count starts
at zero. The socket is borrowed and stays open.

Otherwise the call blocks while the send buffer is full, waiting for space or for
the socket's write timeout to elapse. Use `net::setWriteTimeout` to bound that
wait; when it elapses the call raises `ErrTimeout`, and because the write
loop may already have handed over part of the payload, a timeout can leave the
stream partially written and unresumable. A signal that interrupts a blocking
write re-issues it from the unchanged cursor rather than reporting a closed
connection.

Use `net::writeText` instead when sending UTF-8 text from a `String` is more
convenient than building a `List OF Byte`."#;

const EX: &str = r#"Send a payload as raw bytes and read the reply:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  LET payload AS List OF Byte = [104, 101, 108, 108, 111]
  net::write(client, payload)
  io::print(toString(len(net::read(conn, 16))))
  RETURN 0
END FUNC
```

Echo one chunk back to the peer that sent it:

```
IMPORT net

FUNC echoOnce(RES peer AS net::Socket) AS Integer
  LET chunk = net::read(peer, 4096)
  net::write(peer, chunk)
  RETURN len(chunk)
  TRAP(e)
    RETURN 0
  END TRAP
END FUNC

SUB main()
  ' A single read/echo exchange; the function-level TRAP catches the
  ' ErrConnectionClosed raised once the peer has gone away.
END SUB
```"#;

/// `abi_function` body for `net::write` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_write(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_io::lower_net_write_helper(&symbol, ctx.platform_imports, ctx.platform, false)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "write",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket, List OF Byte"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", "A connected socket to send on, as returned by `net::connectTcp` or `net::accept`. It must still be open; the handle is borrowed, not consumed.", &[], super::socket()),
                super::req("bytes", "The payload, sent in list order. An empty list writes nothing and returns immediately.", &[], ParameterType::list_of(ParameterType::Byte)),
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: super::native_body(lower_write, &[]),
        }],
    });
}
