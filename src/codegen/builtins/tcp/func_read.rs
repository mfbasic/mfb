//! `tcp::read` — descriptor entry (native OS-seam, plan-110-B). Bytes only; the
//! `net::readText` decode form is deliberately not carried over.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::gen_io;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::shared as gen_shared;
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Read up to a number of bytes from a connected TCP socket."#;

const DESC: &str = r#"`tcp::read` reads from a connected `Socket` and returns what arrived as a
`List OF Byte`. It is a **short read**: it returns as soon as any data is
available, so the result may be shorter than `maxBytes` and a caller that needs a
whole message loops until it has assembled one. A successful read always returns
at least one byte.

**The end of the stream is a raise, not an empty list.** When the peer closes its
end, `tcp::read` raises `ErrConnectionClosed`; there is no return value that
means "nothing more is coming". So a drain loop ends in a `TRAP`, not on a
zero-length chunk — see the second example. `tls::read` ends a stream the same
way, so a protocol written against one transport reads the same on the other.

`maxBytes` bounds the read and must be positive; `0` or negative raises
`ErrInvalidArgument`.

How long a read waits is governed by `tcp::setReadTimeout`, not by an argument
here. With no read timeout set the call blocks until data arrives or the peer
closes.

**There is no `readText`.** A stream read stops wherever the network happened to
divide the data, which need not be a character boundary, so a decode at that
point can split a multi-byte character in half. Assemble the whole message first,
then decode it with `encoding::utf8Decode`. `tcp::write` does accept a `String`
directly, because sending is not subject to the same hazard."#;

const EX: &str = r#"Read one chunk and decode it once it is whole:

```
IMPORT tcp
IMPORT encoding
IMPORT io

FUNC main AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  RES client = tcp::connect("127.0.0.1", bound.port)
  RES conn = tcp::accept(server)
  tcp::write(client, "hello")
  LET bytes = tcp::read(conn, 64)
  io::print(encoding::utf8Decode(bytes))
  RETURN 0
END FUNC
```

Read until the peer closes. The `TRAP` is what ends the loop — `tcp::read` never
returns an empty list, so a `len(chunk) = 0` test would loop forever. Checking
the code keeps a genuine read failure from being counted as a clean end:

```
IMPORT errorCode
IMPORT tcp

FUNC drain(sock AS tcp::Socket) AS Integer
  MUT total = 0
  WHILE TRUE
    LET chunk = tcp::read(sock, 4096)
    total = total + len(chunk)
  END WHILE
  RETURN total
  TRAP(err)
    IF err.code = errorCode::ErrConnectionClosed THEN
      RETURN total
    END IF
    RETURN -1
  END TRAP
END FUNC
```"#;

/// `abi_function` body for `tcp::read` — the byte form only (`text = false`).
pub(crate) fn lower_read(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        gen_io::lower_net_read_helper(&symbol, ctx.platform_imports, ctx.platform, false)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
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
                super::req(
                    "sock",
                    "An open connected socket. Borrowed, not consumed.",
                    &[],
                    super::socket(),
                ),
                super::req(
                    "maxBytes",
                    "The maximum number of bytes to read. Must be positive; the result may be shorter, but is never empty — a closed peer raises `ErrConnectionClosed`.",
                    &[],
                    ParameterType::Integer,
                ),
            ],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            body: super::native_body(lower_read, &[]),
        }],
    });
}
