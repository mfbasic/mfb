//! `net::writeText` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/writeText.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Write a String to a connected socket as UTF-8 text."#;

const DESC: &str = r#"`net::writeText` sends the UTF-8 bytes of `value` over a connected `Socket`. The
string's packed byte data is written directly from its buffer: a `String` already
holds well-formed UTF-8, so the bytes go out exactly as held, with no
re-encoding, decoding, or newline translation.

The call writes the *entire* string before returning. It loops, advancing a
cursor past whatever each underlying host write accepted and re-issuing the write
for the remainder, so a short host write is resumed rather than mistaken for
completion. When it returns successfully, every byte of `value` has been handed
to the socket's send buffer — which is not a guarantee that the peer has received
or read them. An empty string writes nothing and returns immediately. The socket
is borrowed and stays open.

Otherwise the call blocks while the send buffer is full, waiting for space or for
the socket's write timeout to elapse. Use `net::setWriteTimeout` to bound that
wait; when it elapses the call raises `ErrTimeout`, and because the loop may
already have handed over part of the text, a timeout can leave the stream
partially written and unresumable. A signal that interrupts a blocking write
re-issues it from the unchanged cursor rather than reporting a closed connection.

Use `net::write` instead to send raw binary data from a `List OF Byte` rather
than UTF-8 text from a `String`."#;

const EX: &str = r#"Send text over a connected socket and read the reply:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  net::writeText(client, "hello")
  io::print(net::readText(conn, 64))
  RETURN 0
END FUNC
```

Echo one chunk of text back to the peer that sent it:

```
IMPORT net

FUNC echoOnce(RES peer AS net::Socket) AS Integer
  LET chunk = net::readText(peer, 4096)
  net::writeText(peer, chunk)
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

/// `abi_function` body for `net::write_text` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_write_text(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_io::lower_net_write_helper(&symbol, ctx.platform_imports, ctx.platform, true)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "writeText",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", "A connected socket to send on, as returned by `net::connectTcp` or `net::accept`. It must still be open; the handle is borrowed, not consumed.", &[], super::socket()),
                super::req("value", "The text to send, written as the string's UTF-8 bytes in order. An empty string sends nothing.", &[], ParameterType::String),
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: super::native_body(lower_write_text, &[]),
        }],
    });
}
