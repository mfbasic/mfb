//! `tls::setWriteTimeout` — descriptor entry (native OS-seam, plan-110-D).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Bound how long writes on a TLS socket may block."#;

const DESC: &str = r#"`tls::setWriteTimeout` sets the socket's write deadline, applying to every
subsequent `tls::write` on it until changed. A write that reaches the deadline
raises `ErrTimeout` rather than blocking further — the usual cause is a peer
that has stopped reading, so the connection's send buffer has filled.

This is a property of the socket, not of one call, matching
`tls::setReadTimeout`.

`timeoutMs` must not be negative — a negative value raises
`ErrInvalidArgument`. `0` means non-blocking: a write that cannot be handed over
at once raises `ErrTimeout` immediately.

**A timed-out write leaves the connection in an unknown state.** Some, all, or
none of the buffer may already have reached the peer, and TLS gives no way to
find out which. Unlike a timed-out read, this is not a retryable condition: the
only safe response is to close the socket. Re-sending would duplicate whatever
did arrive, and continuing to write would interleave with it."#;

const EX: &str = r#"Give up on a peer that has stopped reading:

```
IMPORT tls
IMPORT io

FUNC main AS Integer
  RES conn = tls::connect("example.com", 443)
  tls::setWriteTimeout(conn, 5000)
  tls::write(conn, "GET / HTTP/1.0\r\n\r\n") TRAP(e)
    io::print("peer stopped reading; closing")
    tls::close(conn)
    RETURN 1
  END TRAP
  io::print("request sent")
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `tls::setWriteTimeout` (`write = true`).
pub(crate) fn lower_set_write_timeout(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_shared::lower_tls_set_timeout_helper(
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
        name: "setWriteTimeout",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "sock",
                    desc: "An open TLS socket whose write deadline to set. The handle stays open — you still close it.",
                    aliases: &[],
                    ty: ParameterType::named(super::TLS_SOCKET_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "timeoutMs",
                    desc: "The maximum time a write may wait, in milliseconds. `0` makes writes non-blocking; a negative value raises `ErrInvalidArgument`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_set_write_timeout),
        }],
    });
}
