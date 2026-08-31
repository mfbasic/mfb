//! `tcp::write` — descriptor entry (native OS-seam, plan-110-B). Two overloads
//! that share one name but not one lowering: bytes go through `tcp.write` and a
//! `String` through the `tcp.writeText` code form, which marshals the UTF-8 bytes.
//! `builder_values` selects between them by the second argument's static type.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::gen_io;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::shared as gen_shared;
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Write bytes or text to a connected TCP socket."#;

const DESC: &str = r#"`tcp::write` sends over a connected `Socket`. It accepts either a `List OF Byte`
or a `String`; a `String` is sent as its UTF-8 bytes, with no length prefix,
terminator, or encoding declaration added.

Unlike `tcp::read`, this is a **full write**: it loops until every byte has been
handed to the OS, so a partial write inside the network stack is not something
the caller has to handle. It returns nothing — success means all of it was
accepted for sending, and a failure raises.

How long a blocked write waits is governed by `tcp::setWriteTimeout`, not by an
argument here. With no write timeout set the call blocks until the OS accepts the
data.

An empty `bytes` list — or an empty `String` — is a no-op: nothing is sent and
the call succeeds without touching the socket. `tls::write` behaves identically.

**A write to a peer that has already gone away is not reported reliably** — see
bug-467. The first such write is accepted by the local OS, and a later one
currently terminates the process with `SIGPIPE` instead of raising, so a `TRAP`
around the write does not protect a server from a client that disconnects. Until
that is fixed, treat a disconnect as detectable on the *read* side, where
`tcp::read` raises `ErrConnectionClosed` promptly and correctly.

Note also that TCP gives no delivery receipt even when everything works: a
successful write means the bytes were accepted by the local OS for sending, not
that the peer received or processed them.

The `String` overload exists because sending has no character-boundary hazard —
the whole string is written at once. Reading deliberately has no text form; see
`tcp::read`."#;

const EX: &str = r#"Write text and bytes over the same socket:

```
IMPORT tcp
IMPORT net

FUNC main AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  RES client = tcp::connect("127.0.0.1", bound.port)
  RES conn = tcp::accept(server)
  tcp::write(client, "GET / HTTP/1.0" & chr(13) & chr(10))
  tcp::write(client, [toByte(72), toByte(105)])
  RETURN 0
END FUNC
```"#;

const SOCK_DESC: &str = "An open connected socket. The handle stays open — you still close it.";

/// `abi_function` body for `tcp::write`. The `String` overload arrives under the
/// `tcp.writeText` code form, which is the same emitter in its text mode.
pub(crate) fn lower_write(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let text = ctx.call == "tcp.writeText";
    let (instructions, relocations, stack_size) =
        gen_io::lower_net_write_helper(&symbol, ctx.platform_imports, ctx.platform, text)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "write",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket, List OF Byte or Socket, String"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    super::req("sock", SOCK_DESC, &[], super::socket()),
                    super::req(
                        "bytes",
                        "The bytes to send. Every one is written before the call returns.",
                        &[],
                        ParameterType::list_of(ParameterType::Byte),
                    ),
                ],
                return_type: ParameterType::Nothing,
                errors: vec![],
                body: super::native_body(lower_write, &[]),
            },
            Implementation {
                params: vec![
                    super::req("sock", SOCK_DESC, &[], super::socket()),
                    super::req(
                        "text",
                        "The text to send. Written as UTF-8 bytes, with no terminator or length prefix added.",
                        &[],
                        ParameterType::String,
                    ),
                ],
                return_type: ParameterType::Nothing,
                errors: vec![],
                body: super::native_body(lower_write, &["writeText"]),
            },
        ],
    });
}
