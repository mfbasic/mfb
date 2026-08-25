//! `net::readText` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/readText.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Read available bytes from a connected socket as UTF-8 text."#;

const DESC: &str = r#"`net::readText` receives data from a connected `Socket` and returns it as a
`String`. A single call performs one underlying receive: it returns as soon as
any data is available rather than waiting to fill the requested size, so the
result is frequently shorter than `maxBytes` bytes, and on success it is built
from at least one byte. The socket is borrowed and stays open.

The received bytes are copied into a freshly allocated string and then validated
as UTF-8 before the string is returned; bytes that are not well-formed UTF-8
raise `ErrEncoding`. This is the one way `readText` differs from `net::read`
beyond the result type, and it is also its main hazard: a single receive may split
a multi-byte UTF-8 sequence across two calls, and the call holding the partial
sequence fails validation. When the peer sends raw binary data, or when bytes
must be reassembled across several receives before decoding, use `net::read` and
convert once the message is complete.

The call blocks until at least one byte arrives, the peer closes its side, or the
socket's read timeout elapses. `maxBytes` bounds the bytes read in this call and
must be positive; internally the temporary receive buffer is capped at 1 MiB even
when `maxBytes` is larger, so a very large `maxBytes` does not pre-commit that
much memory. Like `net::read`, end of stream is *not* an empty result: when the
peer has closed, `ErrConnectionClosed` is raised. Read in a loop and stop on that
error. Use `net::poll` to test for readiness without blocking, and
`net::setReadTimeout` to bound how long a read may wait — an elapsed timeout
raises `ErrTimeout`.

A signal that interrupts the blocking receive re-issues the identical read rather
than misreporting it as a closed connection. The call has no side effects beyond
receiving and does not close the socket."#;

const EX: &str = r#"Read a line of text from a connected socket:

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

Report the error code when a read times out:

```
IMPORT net
IMPORT io

FUNC readOrCode(RES sock AS net::Socket) AS String
  RETURN net::readText(sock, 64)
  TRAP(e)
    RETURN toString(e.code)
  END TRAP
END FUNC
```"#;

/// `abi_function` body for `net::read_text` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_read_text(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_io::lower_net_read_helper(&symbol, ctx.platform_imports, ctx.platform, true)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readText",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", "A connected socket to receive from, as returned by `net::connectTcp` or `net::accept`. It must still be open; the handle is borrowed, not consumed.", &[], super::socket()),
                super::req("maxBytes", "The maximum number of bytes to receive in this call. Must be positive. It caps the bytes received before decoding but does not guarantee that many arrive.", &[], ParameterType::Integer),
            ],
            return_type: ParameterType::String,
            errors: vec![],
            body: super::native_body(lower_read_text, &[]),
        }],
    });
}
