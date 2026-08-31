//! `tls::localAddress` — descriptor entry (native OS-seam, plan-110-D).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Report the local end of a TLS connection."#;

const DESC: &str = r#"`tls::localAddress` returns the `net::Address` the OS has assigned to this end of
a TLS socket — the local address and port the encrypted connection runs over.

The address describes the *transport* underneath TLS, not the TLS session: it is
the same value the equivalent plaintext connection would report, and it says
nothing about the cipher suite or the peer's certificate.

The socket is borrowed, not consumed, and the result is an ordinary
`net::Address` with no tie to the socket's lifetime.

**A file that uses the returned address must `IMPORT net` as well as `tls`.**
Imports are not transitive and packages cannot re-export types, so `Address` is
nameable only where `net` is imported."#;

const EX: &str = r#"Report which local port a TLS connection went out on:

```
IMPORT net
IMPORT tls
IMPORT io

FUNC main AS Integer
  RES conn = tls::connect("example.com", 443, 5000)
  LET at = tls::localAddress(conn)
  io::print("local port " & toString(at.port))
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `tls::localAddress` (`remote = false`).
pub(crate) fn lower_local_address(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_shared::lower_tls_address_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        false,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "localAddress",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "sock",
                desc: "An open TLS socket whose local end to report. Borrowed, not consumed.",
                aliases: &[],
                ty: ParameterType::named(super::TLS_SOCKET_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(crate::codegen::builtins::net::ADDRESS_TYPE),
            errors: vec![],
            body: Body::abi_function(lower_local_address),
        }],
    });
}
