//! `tls::writeText` — descriptor entry (native OS-seam).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Send a `String` as UTF-8 text over a connected TLS socket."#;
const DESC: &str = r#"`writeText` sends the bytes of `value` as application data over a connected
`TlsSocket`, encrypting them through the negotiated TLS session. An mfb `String`
is already UTF-8, so the bytes are sent exactly as they are stored, with no
re-encoding and no trailing newline added. It writes the whole string: the call
loops over the underlying TLS write until every byte has been accepted, so a
successful return means all of `value` was handed to the TLS layer, not merely
the first chunk. The socket must still be open.

The bytes are taken from the string in order, starting at its first byte. An
empty `value` is a no-op: nothing is sent and the call succeeds without touching
the TLS layer. The function reads from the existing string buffer and allocates
nothing of its own; it has no side effects beyond the bytes it sends and does
not close the socket.

`writeText` returns `Nothing`; there is no short-write result to inspect, because
a partial write that cannot be completed is reported as an error rather than a
count. Use `tls::write` to send a `List OF Byte` when you have raw binary data
rather than text, and `tls::read` or `tls::readText` to receive the peer's reply."#;
const EX: &str = r#"Send an HTTP request as text over a connected TLS socket:

```
IMPORT tls

SUB main()
  RES conn = tls::connect("example.com", 443)
  tls::writeText(conn, "GET / HTTP/1.0\r\n\r\n")
  LET reply = tls::readText(conn, 4096)
  ' conn is closed by lexical drop when this scope ends
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "writeText",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("TlsSocket, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "sock",
                    desc: "A connected TLS socket to send on, as returned by `tls::connect`. It must still be open; writing to a closed socket is an error.",
                    aliases: &[],
                    ty: ParameterType::Named(super::TLS_SOCKET_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "value",
                    desc: "The text to send, transmitted as its UTF-8 bytes in order. The entire string is written before the call returns. An empty string sends nothing and succeeds.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(super::gen_os_seam::lower_tls_os_seam),
        }],
    });
}
