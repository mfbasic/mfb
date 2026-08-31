//! `tcp::setReadTimeout` — descriptor entry (native OS-seam, plan-110-B).

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::{poll as gen_poll, shared as gen_shared};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Bound how long reads on a socket may block."#;

const DESC: &str = r#"`tcp::setReadTimeout` sets the socket's read deadline, applying to every
subsequent `tcp::read` on it until changed. A read that reaches the deadline with
no data raises `ErrTimeout` rather than blocking further.

This is a property of the socket, not of one call, which is why `tcp::read` takes
no timeout argument: a stream is read repeatedly and the policy belongs to the
connection.

`timeoutMs` must not be negative — a negative value raises `ErrInvalidArgument`.
`0` means non-blocking: a read with nothing available raises `ErrTimeout`
immediately instead of waiting.

Use `tcp::poll` instead when the question is "is there data?" rather than "read,
but not forever": poll answers with a `Boolean` and raises nothing."#;

const EX: &str = r#"Do not let a silent peer stall a read forever:

```
IMPORT tcp
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  RES client = tcp::connect("127.0.0.1", bound.port)
  RES conn = tcp::accept(server)
  tcp::setReadTimeout(conn, 250)
  LET chunk = tcp::read(conn, 64) TRAP(e)
    io::print("peer said nothing within 250ms")
    RETURN 0
  END TRAP
  io::print(toString(len(chunk)))
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `tcp::setReadTimeout` (`write = false` → `SO_RCVTIMEO`).
pub(crate) fn lower_set_read_timeout(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        gen_poll::lower_net_set_timeout_helper(&symbol, ctx.platform_imports, ctx.platform, false)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
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
                super::req(
                    "sock",
                    "An open connected socket. The handle stays open — you still close it.",
                    &[],
                    super::socket(),
                ),
                super::req(
                    "timeoutMs",
                    "The read deadline in milliseconds, applied to every later read on this socket. `0` makes reads non-blocking; a negative value raises `ErrInvalidArgument`.",
                    &[],
                    ParameterType::Integer,
                ),
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: super::native_body(lower_set_read_timeout, &[]),
        }],
    });
}
