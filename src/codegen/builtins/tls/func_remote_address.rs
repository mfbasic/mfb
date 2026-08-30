//! `tls::remoteAddress` — descriptor entry (native OS-seam, plan-110-D).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Report the peer's address on a TLS connection."#;

const DESC: &str = r#"`tls::remoteAddress` returns the `net::Address` of the peer at the far end of a
TLS socket — the address and port the encrypted connection actually reached.

For a socket from `tls::accept` this is how a server learns who connected. For
one from `tls::connect` it reports the address the host name resolved to, which
can differ from the string that was passed in.

The address describes the *transport* underneath TLS. It is **not** an identity:
a peer's address says nothing about whether its certificate validated, and must
not be used as an authorisation check. `tls::connect` verifies the certificate
against the requested name; that verification, not this address, is what
establishes who the peer is.

The socket is borrowed, not consumed.

**A file that uses the returned address must `IMPORT net` as well as `tls`.**
Imports are not transitive and packages cannot re-export types."#;

const EX: &str = r#"Log which peer a TLS server accepted:

```
IMPORT net
IMPORT tls
IMPORT io

FUNC main AS Integer
  RES server = tls::listen("127.0.0.1", 0, "cert.pem", "key.pem")
  RES conn = tls::accept(server)
  io::print("client from " & tls::remoteAddress(conn).host)
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `tls::remoteAddress` (`remote = true`).
pub(crate) fn lower_remote_address(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_shared::lower_tls_address_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        true,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "remoteAddress",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "sock",
                desc: "An open TLS socket whose peer to report. Borrowed, not consumed.",
                aliases: &[],
                ty: ParameterType::named(super::TLS_SOCKET_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(crate::codegen::builtins::net::ADDRESS_TYPE),
            errors: vec![],
            body: Body::abi_function(lower_remote_address),
        }],
    });
}
