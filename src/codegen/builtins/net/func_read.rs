//! `net::read` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/read.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Read available bytes from a connected socket."#;

const DESC: &str = r#"`net::read` receives data from a connected `Socket` and returns it as a
`List OF Byte`. A single call performs one underlying receive: it returns as soon
as any data is available rather than waiting to fill the requested size, so the
returned list is frequently shorter than `maxBytes`. On success it always holds
at least one byte. The socket is borrowed and stays open.

The call blocks until at least one byte arrives, the peer closes its side, or the
socket's read timeout elapses. `maxBytes` bounds a single read and the length of
the result; it does not request that exactly that many bytes be read, and it must
be positive. Internally the temporary receive buffer is capped at 1 MiB even when
`maxBytes` is larger, so a very large `maxBytes` does not pre-commit that much
memory for a read that delivers far fewer bytes. Because a single host receive
never returns more than the socket's receive buffer, this cap is invisible to the
one-receive semantics above.

Unlike a plain stream read that signals end of stream with a zero-length result,
`net::read` raises an error when the peer has closed: there is no empty-list
sentinel, and a successful result never has length `0`. To consume a whole
message, call `read` in a loop, appending each result, and stop when
`ErrConnectionClosed` is raised. Use `net::poll` to test for readiness without
blocking and `net::setReadTimeout` to bound how long a read may wait; a timeout
that elapses raises `ErrTimeout`, which is distinguished from a closed
connection by the host reporting `EAGAIN`.

A signal that interrupts the blocking receive re-issues the identical read rather
than misreporting it as a closed connection. The bytes are copied into a freshly
allocated `List OF Byte`; the call has no other side effects and does not close
the socket. Use `net::readText` when the peer sends UTF-8 text and a `String` is
more convenient than raw bytes."#;

const EX: &str = r#"Read up to 16 bytes from a connected socket:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  net::writeText(client, "abc")
  LET raw = net::read(conn, 16)
  io::print(toString(len(raw)))
  RETURN 0
END FUNC
```

Drain a connection until the peer closes it:

```
IMPORT net

FUNC drain(RES sock AS net::Socket) AS Integer
  MUT total AS Integer = 0
  MUT reading AS Boolean = TRUE
  WHILE reading
    LET chunk = net::read(sock, 4096)
    total = total + len(chunk)
  END WHILE
  RETURN total
  TRAP(e)
    RETURN total
  END TRAP
END FUNC

SUB main()
  ' Sums the bytes received until the peer closes; the function-level TRAP catches
  ' the ErrConnectionClosed that ends the stream and returns the running total.
END SUB
```"#;

/// `abi_function` body for `net::read` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_read(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_io::lower_net_read_helper(&symbol, ctx.platform_imports, ctx.platform, false)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "read",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("sock", "A connected socket to receive from, as returned by `net::connectTcp` or `net::accept`. It must still be open; the handle is borrowed, not consumed.", &[], super::socket()),
                super::req("maxBytes", "The maximum number of bytes to read in this call. Must be positive. It caps the length of the returned list but does not guarantee that many bytes arrive.", &[], ParameterType::Integer),
            ],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            body: super::native_body(lower_read, &[]),
        }],
    });
}
