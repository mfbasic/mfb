//! `tls::setReadTimeout` — descriptor entry (native OS-seam, plan-110-D).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Bound how long reads on a TLS socket may block."#;

const DESC: &str = r#"`tls::setReadTimeout` sets the socket's read deadline, applying to every
subsequent `tls::read` on it until changed. A read that reaches the deadline
with no plaintext available raises `ErrTimeout` rather than blocking further.

This is a property of the socket, not of one call, which is why `tls::read`
takes no timeout argument: a stream is read repeatedly and the policy belongs to
the connection.

`timeoutMs` must not be negative — a negative value raises `ErrInvalidArgument`.
`0` means non-blocking: a read with nothing decrypted and nothing pending raises
`ErrTimeout` immediately instead of waiting.

A timed-out read does not disturb the connection. The bytes the peer sends after
the deadline are not lost — they are delivered to the next `tls::read`, which
resumes the same outstanding receive rather than starting a new one. A read that
timed out can therefore be retried, and a `tls::poll` after one still reports
readiness correctly.

Use `tls::poll` instead when the question is "is there data?" rather than "read,
but not forever": poll answers with a `Boolean` and raises nothing."#;

const EX: &str = r#"Do not let a silent peer stall a read forever:

```
IMPORT encoding
IMPORT tls
IMPORT io

FUNC main AS Integer
  RES conn = tls::connect("example.com", 443)
  tls::setReadTimeout(conn, 2000)
  tls::write(conn, "GET / HTTP/1.0\r\n\r\n")
  LET chunk = tls::read(conn, 4096) TRAP(e)
    io::print("peer said nothing within 2s")
    RETURN 0
  END TRAP
  io::print(encoding::utf8Decode(chunk))
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `tls::setReadTimeout` (`write = false`).
pub(crate) fn lower_set_read_timeout(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_shared::lower_tls_set_timeout_helper(
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
        name: "setReadTimeout",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "sock",
                    desc: "An open TLS socket whose read deadline to set. The handle stays open — you still close it.",
                    aliases: &[],
                    ty: ParameterType::named(super::TLS_SOCKET_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "timeoutMs",
                    desc: "The maximum time a read may wait, in milliseconds. `0` makes reads non-blocking; a negative value raises `ErrInvalidArgument`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_set_read_timeout),
        }],
    });
}
