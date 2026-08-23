//! `tls::write` — descriptor entry (native OS-seam).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Send raw bytes over a connected TLS socket."#;
const DESC: &str = r#"`write` sends the contents of `bytes` as application data over a connected
`TlsSocket`, encrypting it through the negotiated TLS session. It writes the
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
count. Use `tls::writeText` to send a `String` as UTF-8 without first converting
it to a `List OF Byte`, and `tls::read` or `tls::readText` to receive the peer's
reply."#;
const EX: &str = r#"Send a raw request over a connected TLS socket:

```
IMPORT tls
IMPORT strings

SUB main()
  RES conn = tls::connect("example.com", 443)
  LET request = strings::toBytes("GET / HTTP/1.0\r\n\r\n")
  tls::write(conn, request)
  LET reply = tls::readText(conn, 4096)
  ' conn is closed by lexical drop when this scope ends
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "write",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("TlsSocket, List OF Byte"),
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
                    name: "bytes",
                    desc: "The application data to send, in order. The entire list is written before the call returns. An empty list sends nothing and succeeds.",
                    aliases: &[],
                    ty: ParameterType::ListOf(Box::new(ParameterType::Byte)),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(super::gen_os_seam::lower_tls_os_seam),
        }],
    });
}
