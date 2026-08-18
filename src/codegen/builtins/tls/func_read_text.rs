//! `tls::readText` — descriptor entry (native OS-seam).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Read available bytes from a connected TLS socket as UTF-8 text."#;
const DESC: &str = r#"`readText` receives decrypted application data from a connected `TlsSocket` and
returns it decoded as a UTF-8 `String`. A single call performs one underlying TLS
read: it returns as soon as any plaintext is available rather than waiting to
fill the requested size, so the returned `String` is frequently built from fewer
than `maxBytes` bytes. The socket must still be open.

The call blocks until at least one byte of application data has been decrypted,
the peer closes its side of the TLS session, or the underlying read fails.
`maxBytes` bounds the number of bytes read in this call and the number of bytes
decoded into the result; it does not request that exactly that many bytes be
read. On success the returned `String` is built from at least one byte.

Unlike a plain stream read that signals end of stream with a zero-length result,
`readText` raises an error when the peer has closed the connection: there is no
empty-`String` sentinel. To consume a whole response, call `readText` in a loop,
appending each result, and stop when an `ErrConnectionClosed` error is raised.

The decrypted bytes are validated as UTF-8 before being returned; invalid UTF-8
raises an `ErrEncoding` error. Because a single TLS read may split a multi-byte
UTF-8 sequence across calls, use `tls::read` instead when the peer sends raw
binary data, or when you need to reassemble bytes spanning multiple reads before
decoding."#;
const EX: &str = r#"Read up to 4096 bytes of text from a connected TLS socket:

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
        name: "readText",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("TlsSocket, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "sock",
                    desc: "A connected TLS socket to receive from, as returned by `tls::connect`. It must still be open; reading from a closed socket is an error.",
                    aliases: &[],
                    ty: ParameterType::Named(super::TLS_SOCKET_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "maxBytes",
                    desc: "The maximum number of bytes to read in this call. Must be positive. It caps the number of bytes received before decoding but does not guarantee that many bytes are returned.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::native_os_seam(
                Some(super::native::lower_tls_helper),
                Some(super::native::lower_tls_helper),
                &[],
            ),
        }],
    });
}
