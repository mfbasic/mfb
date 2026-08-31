//! `tls::connect` — descriptor entry (native OS-seam).
//!
//! Per-member file (planning/migrate.md). This member owns its `Body::abi_function`
//! body ([`lower_connect`]), which calls the shared per-member family dispatcher
//! [`super::gen_shared::lower_tls_connect_helper`] (picking openssl/schannel/macOS by
//! `platform.family()`) and finalizes.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Open a TLS connection to a host and verify its certificate."#;
const DESC: &str = r#"`connect` establishes an outbound TCP connection to `host` on `port`, performs a
TLS client handshake over it, and returns a connected `Socket` resource. The
host is resolved with the system host resolver before connecting; the first
resolved IPv4 address is used. Once the socket is connected the handshake
negotiates TLS 1.2 or later — older protocol versions are refused — against the
system trust store loaded from the default certificate verification paths.

The peer's certificate is always verified: the certificate chain must validate
against the system trust store and the certificate must match the expected
server name. By default the expected name is `host`; supply a non-empty
`serverName` to validate against a different name and to send it as the TLS
Server Name Indication (SNI) extension, which is useful when connecting to a
literal IP address or to a virtual host whose certificate name differs from the
`host` argument. A handshake that fails for any reason — chain validation, name
mismatch, protocol negotiation, or a refused or reset connection during the
handshake — raises `ErrTlsFailed`, and the underlying socket is closed before
the error is returned.

`timeoutMs` follows the language timeout convention (see
`mfb spec language builtin-functions` → "Timeout convention"). When it is
**omitted the connect blocks** until it completes or the OS/TLS layer fails. `0`
is one immediate attempt: it succeeds if the connection and handshake complete at
once, otherwise it raises `ErrTimeout` without waiting. A positive value bounds
the attempt and raises `ErrTimeout` when it elapses. A negative `timeoutMs` raises
`ErrInvalidArgument`. **Host resolution is not bounded** — the resolver call
happens before the deadline starts, so a slow DNS lookup can exceed `timeoutMs`.

The overloads do not share a positional layout: `timeoutMs` and `serverName` are
parameters 2 and 3 of the host/port form but 1 and 2 of the `Address` form, since
one endpoint value replaces two. Named arguments therefore bind per-overload,
against whichever overload the argument types select — the same caveat
`tcp::connect` carries.

TLS is implemented on Linux by driving the system OpenSSL library (`libssl.so.3`,
falling back to `libssl.so.1.1`) so a single binary spans OpenSSL 1.1.1 and 3.x;
the macOS backend drives Network.framework through a synchronous bridge. If the
TLS layer cannot be initialized — neither OpenSSL library can be loaded, or a
required symbol is missing — `connect` raises `ErrTlsFailed`."#;
const EX: &str = r#"Connect to an HTTPS server and validate its certificate:

```
IMPORT encoding
IMPORT tls

SUB main()
  RES conn = tls::connect("example.com", 443)
  tls::write(conn, "GET / HTTP/1.0\r\n\r\n")
  LET response = encoding::utf8Decode(tls::read(conn, 4096))
  ' conn closes itself when this scope ends
END SUB
```

Connect to a literal IP but validate against a named certificate via SNI:

```
IMPORT encoding
IMPORT tls

SUB main()
  RES conn = tls::connect("93.184.216.34", 443, timeoutMs := 5000, serverName := "example.com")
  ' conn closes itself when this scope ends
END SUB
```"#;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `tls::connect` — calls the shared `lower_tls_*_helper`
/// family dispatcher and finalizes. The `net::Address` overloads arrive under the
/// `tls.connectAddr` code form and read the endpoint out of the record.
pub(crate) fn lower_connect(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_shared::lower_tls_connect_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        ctx.call == "tls.connectAddr",
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let ret = || ParameterType::named(super::TLS_SOCKET_TYPE_ID);
    pkg.add_function(RegistryFunction {
        name: "connect",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, Integer, Integer, String or Address, Integer, String"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "host",
                        desc: "The host name or textual IP address of the peer. Resolved with the host resolver; a name that cannot be resolved raises an error. Also used as the certificate validation and SNI name when `serverName` is omitted or empty.",
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "port",
                        desc: "The TCP port to connect to on the peer.",
                        aliases: &[],
                        ty: ParameterType::Integer,
                        default: DefaultValue::None,
                    },
                    timeout_param(),
                    server_name_param(false),
                ],
                return_type: ret(),
                errors: vec![],
                body: Body::abi_function(lower_connect),
            },
            // The `net::Address` form. The endpoint is one value instead of two,
            // so the two optional arguments shift down a position; everything
            // after the argument prologue is the same handshake.
            Implementation {
                params: vec![
                    Parameter {
                        name: "address",
                        desc: ADDRESS_DESC,
                        aliases: &[],
                        ty: ParameterType::named(crate::codegen::builtins::net::ADDRESS_TYPE),
                        default: DefaultValue::None,
                    },
                    timeout_param(),
                    server_name_param(true),
                ],
                return_type: ret(),
                errors: vec![],
                body: Body::abi_function_aliased(lower_connect, &["connectAddr"]),
            },
        ],
    });
}

const ADDRESS_DESC: &str = "A destination supplying both the peer host and the peer port, typically from `net::lookup`. Replaces the separate `host` and `port` arguments. Certificate validation still defaults to this address's `host` field, which for a `net::lookup` result is a numeric IP — pass `serverName` to validate against the name the certificate actually carries.";

/// The shared optional `timeoutMs` parameter. Omitted is the unbounded sentinel.
fn timeout_param() -> Parameter {
    Parameter {
        name: "timeoutMs",
        desc: "Optional. The maximum time the connection and handshake may take, in milliseconds. Omit to block until it completes; `0` is one immediate attempt; a positive value bounds it; a negative value raises `ErrInvalidArgument`. Host resolution happens first and is not counted against it.",
        aliases: &[],
        ty: ParameterType::Integer,
        default: DefaultValue::Fill {
            type_name: ParameterType::Integer,
            expr: super::SENTINEL,
        },
    }
}

/// The shared optional `serverName` parameter; `address` selects the wording for
/// which argument it falls back to.
fn server_name_param(address: bool) -> Parameter {
    Parameter {
        name: "serverName",
        desc: if address {
            "Optional. When non-empty, the name the peer certificate must match and the host name sent in the TLS SNI extension, replacing the address's host for validation. Defaults to the empty string, in which case `address.host` is used."
        } else {
            "Optional. When non-empty, the name the peer certificate must match and the host name sent in the TLS SNI extension, replacing `host` for validation. Defaults to the empty string, in which case `host` is used."
        },
        aliases: &[],
        ty: ParameterType::String,
        default: DefaultValue::Fill {
            type_name: ParameterType::String,
            expr: "",
        },
    }
}
