//! `tls::write` — descriptor entry (native OS-seam).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Send raw bytes over a connected TLS socket."#;
const DESC: &str = r#"`write` sends the contents of `bytes` as application data over a connected
`Socket`, encrypting it through the negotiated TLS session. It writes the
whole list: the call loops over the underlying TLS write until every byte has
been accepted, so a successful return means all of `bytes` was handed to the TLS
layer, not merely the first chunk. The socket must still be open.

The bytes are taken from the list in order, starting at its first element. An
empty `bytes` list is a no-op: nothing is sent and the call succeeds without
touching the TLS layer. The function reads from the existing list buffer and
allocates nothing of its own; it has no side effects beyond the bytes it sends
and does not close the socket.

`write` returns `Nothing`; there is no short-write result to inspect, because a
partial write that cannot be completed is reported as an error rather than a
count. Use `tls::write` to send a `String` as UTF-8 without first converting
it to a `List OF Byte`, and `tls::read` or `tls::read` to receive the peer's
reply."#;
const EX: &str = r#"Send a raw request over a connected TLS socket:

```
IMPORT encoding
IMPORT tls
IMPORT strings

SUB main()
  RES conn = tls::connect("example.com", 443)
  LET request = strings::toBytes("GET / HTTP/1.0\r\n\r\n")
  tls::write(conn, request)
  LET reply = encoding::utf8Decode(tls::read(conn, 4096))
  ' conn is closed by lexical drop when this scope ends
END SUB
```"#;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `tls::write` — calls the shared `lower_tls_*_helper`
/// family dispatcher and finalizes. plan-110-D collapsed the former separate
/// `writeText` member into a second overload here, so the byte-vs-text choice
/// moved from the member name to the payload's type: the `String` overload
/// arrives under the `tls.writeText` code form, which is the same emitter in its
/// text mode.
pub(crate) fn lower_write(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let text = ctx.call == "tls.writeText";
    let (instructions, relocations, stack_size) = super::gen_shared::lower_tls_write_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        text,
    )?;
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
        expected_arguments: Some("Socket, List OF Byte or Socket, String"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "sock",
                        desc: "A connected TLS socket to send on, as returned by `tls::connect`. It must still be open; writing to a closed socket is an error.",
                        aliases: &[],
                        ty: ParameterType::named(super::TLS_SOCKET_TYPE_ID),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "bytes",
                        desc: "The application data to send, in order. The entire list is written before the call returns. An empty list sends nothing and succeeds.",
                        aliases: &[],
                        ty: ParameterType::ListOf(Box::new(ParameterType::Byte)),
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::Nothing,
                errors: vec![],
                body: Body::abi_function_aliased(lower_write, &[]),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "sock",
                        desc: "A connected TLS socket to send on, as returned by `tls::connect`. It must still be open; writing to a closed socket is an error.",
                        aliases: &[],
                        ty: ParameterType::named(super::TLS_SOCKET_TYPE_ID),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "text",
                        desc: "The text to send. Written as its UTF-8 bytes, with no terminator or length prefix added. The whole string is written before the call returns.",
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::Nothing,
                errors: vec![],
                body: Body::abi_function_aliased(lower_write, &["writeText"]),
            },
        ],
    });
}
