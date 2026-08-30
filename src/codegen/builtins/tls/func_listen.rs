//! `tls::listen` — descriptor entry (native OS-seam).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Bind a local port and load a server certificate to terminate TLS."#;
const DESC: &str = r#"`listen` binds a local TCP endpoint and loads a server TLS identity so a program
can *terminate* TLS: accept encrypted inbound connections, present a server
certificate that clients validate, and exchange application data. It returns a
`Listener` resource that `tls::accept` draws connections from. It is the
server-side counterpart to the client's `tls::connect`.

The endpoint is resolved and bound exactly as `tcp::listen` does. An empty
`host` (or `"0.0.0.0"`) binds all local interfaces; any other value binds the
matching address. The listening socket is created with the address-reuse option
set, so a restarted server can re-bind a recently used port. The optional
`backlog` hints the size of the kernel's pending-connection queue; `0` (the
default when omitted) uses the host default.

`certPath` and `keyPath` are filesystem paths to PEM files: the certificate
chain (leaf certificate first, followed by any intermediates) and the matching
private key. The pair is loaded once, when the listener is created, into a
**server TLS context** that every accepted connection reuses. A cert or key that
cannot be read, does not parse, or does not match its partner raises
`ErrTlsFailed` and the listening socket is closed before the error is returned.

The server TLS context is owned by the `Listener` and *borrowed* by each
accepted `Socket`: closing an accepted socket never frees the shared context,
which is released exactly once when the listener itself closes. The listener
presents its certificate but does not request or verify a client certificate
(no mutual TLS in this version)."#;
const EX: &str = r#"Terminate TLS on port 8443 with a self-signed certificate and echo one line:

```
IMPORT encoding
IMPORT tls
IMPORT io

SUB main()
  RES server = tls::listen("127.0.0.1", 8443, "cert.pem", "key.pem")
  RES client = tls::accept(server)
  LET line = encoding::utf8Decode(tls::read(client, 4096))
  tls::write(client, "you said: " & line)
  tls::close(client)
  ' server is closed by lexical drop when this scope ends
END SUB
```"#;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `tls::listen` — calls the shared `lower_tls_*_helper`
/// family dispatcher and finalizes.
pub(crate) fn lower_listen(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_shared::lower_tls_listen_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "listen",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, Integer, String, String, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "host",
                    desc: "The local address to bind. An empty string or `\"0.0.0.0\"` binds all interfaces; any other value binds the matching local address.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "port",
                    desc: "The local TCP port to bind and listen on.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "certPath",
                    desc: "Filesystem path to a PEM file holding the server certificate chain, leaf certificate first.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "keyPath",
                    desc: "Filesystem path to a PEM file holding the private key matching the leaf certificate.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "backlog",
                    desc: "Optional. A hint for the kernel pending-connection queue length. Defaults to `0`, which uses the host default.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::Integer,
                        expr: "0",
                    },
                },
            ],
            return_type: ParameterType::named(super::TLS_LISTENER_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_listen),
        }],
    });
}
