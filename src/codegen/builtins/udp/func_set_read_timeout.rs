//! `udp::setReadTimeout` — descriptor entry (native OS-seam, plan-110-C).

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::builtins::net::{gen_poll, gen_shared};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Bound how long receives on a socket may block."#;

const DESC: &str = r#"`udp::setReadTimeout` sets the socket's receive deadline, applying to every
subsequent `udp::receive` until changed. A receive that reaches the deadline with
no datagram queued raises `ErrTimeout`.

This matters more for UDP than for a stream. A datagram that is lost in transit is
never retransmitted and never arrives, so a `receive` with no deadline waits
forever for something that will never come. Any protocol expecting a reply should
set a timeout and treat its expiry as loss.

`timeoutMs` must not be negative — a negative value raises `ErrInvalidArgument`.
`0` makes receives non-blocking: with nothing queued the call raises `ErrTimeout`
immediately.

Use `udp::poll` instead when the question is "has anything arrived?" rather than
"receive, but not forever": poll answers with a `Boolean` and raises nothing."#;

const EX: &str = r#"Treat a missing reply as loss rather than waiting forever:

```
IMPORT net
IMPORT udp
IMPORT io

FUNC main AS Integer
  RES sock = udp::bind("127.0.0.1", 0)
  udp::setReadTimeout(sock, 250)
  LET reply = udp::receive(sock, 1024) TRAP(e)
    io::print("no reply within 250ms -- treating as lost")
    RETURN 0
  END TRAP
  io::print("got " & toString(len(reply.bytes)) & " bytes")
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `udp::setReadTimeout` (`write = false` → `SO_RCVTIMEO`).
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
                    "An open bound socket. Borrowed, not consumed.",
                    &[],
                    super::socket(),
                ),
                super::req(
                    "timeoutMs",
                    "The receive deadline in milliseconds, applied to every later receive on this socket. `0` makes receives non-blocking; a negative value raises `ErrInvalidArgument`.",
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
