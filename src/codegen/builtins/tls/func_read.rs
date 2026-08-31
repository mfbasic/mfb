//! `tls::read` — descriptor entry (native OS-seam).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Read available bytes from a connected TLS socket."#;
const DESC: &str = r#"`read` receives decrypted application data from a connected `Socket` and
returns it as a `List OF Byte`. A single call performs one underlying TLS read:
it returns as soon as any plaintext is available rather than waiting to fill the
requested size, so the returned list is frequently shorter than `maxBytes`. The
socket must still be open.

The call blocks until at least one byte of application data has been decrypted,
the peer closes its side of the TLS session, or the underlying read fails.
`maxBytes` bounds the size of a single read and the size of the returned list; it
does not request that exactly that many bytes be read. On success the returned
list always holds at least one byte.

`read` raises `ErrConnectionClosed` when the peer has closed the connection:
there is no empty-list sentinel. To consume a whole response, call `read` in a
loop, appending each result, and stop when that error is raised. `tcp::read` ends
a stream exactly the same way, so a protocol written against one transport reads
the same on the other.

**There is no `readText`.** A stream read stops wherever the network happened to
divide the data, which need not be a character boundary, so a decode at that
point can split a multi-byte character in half. Assemble the whole message first,
then decode it with `encoding::utf8Decode`. `tls::write` does accept a `String`
directly, because sending is not subject to the same hazard."#;
const EX: &str = r#"Read up to 4096 bytes from a connected TLS socket:

```
IMPORT tls

SUB main()
  RES conn = tls::connect("example.com", 443)
  tls::write(conn, "GET / HTTP/1.0\r\n\r\n")
  LET chunk = tls::read(conn, 4096)
  ' conn is closed by lexical drop when this scope ends
END SUB
```"#;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `tls::read` — calls the shared `lower_tls_*_helper`
/// family dispatcher and finalizes.
pub(crate) fn lower_read(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_shared::lower_tls_read_helper(&symbol, ctx.platform_imports, ctx.platform)?;
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
                Parameter {
                    name: "sock",
                    desc: "A connected TLS socket to receive from, as returned by `tls::connect`. It must still be open; reading from a closed socket is an error.",
                    aliases: &[],
                    ty: ParameterType::named(super::TLS_SOCKET_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "maxBytes",
                    desc: "The maximum number of bytes to read in this call. Must be positive. It caps the length of the returned list but does not guarantee that many bytes are returned.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::ListOf(Box::new(ParameterType::Byte)),
            errors: vec![],
            body: Body::abi_function(lower_read),
        }],
    });
}
