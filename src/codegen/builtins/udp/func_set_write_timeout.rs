//! `udp::setWriteTimeout` — descriptor entry (native OS-seam, plan-110-C).

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::{poll as gen_poll, shared as gen_shared};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Bound how long sends on a socket may block."#;

const DESC: &str = r#"`udp::setWriteTimeout` sets the socket's send deadline, applying to every
subsequent `udp::send` until changed. A send that reaches the deadline without the
OS accepting the datagram raises `ErrTimeout`.

A UDP send rarely blocks: it hands the datagram to the OS and returns, with no
peer to wait for. It can block when the send buffer is full — a program sending
far faster than the interface drains, or one hitting a kernel rate limit — so a
deadline is a guard against that case rather than an everyday concern.

`timeoutMs` must not be negative — a negative value raises `ErrInvalidArgument`.
`0` makes sends non-blocking: a datagram that cannot be accepted immediately
raises `ErrTimeout`.

A timeout here says the datagram was **not** handed to the OS. That is a stronger
statement than the usual UDP silence: an ordinary successful send still carries no
promise of delivery, but a raised send means it was never even attempted."#;

const EX: &str = r#"Do not let a saturated send buffer stall a sender:

```
IMPORT net
IMPORT udp
IMPORT io

FUNC main AS Integer
  RES sock = udp::bind("127.0.0.1", 0)
  RES peer = udp::bind("127.0.0.1", 0)
  udp::setWriteTimeout(sock, 500)
  udp::send(sock, udp::localAddress(peer), "hello")
  io::print("sent")
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `udp::setWriteTimeout` (`write = true` → `SO_SNDTIMEO`).
pub(crate) fn lower_set_write_timeout(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        gen_poll::lower_net_set_timeout_helper(&symbol, ctx.platform_imports, ctx.platform, true)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
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
                super::req(
                    "sock",
                    "An open bound socket. Borrowed, not consumed.",
                    &[],
                    super::socket(),
                ),
                super::req(
                    "timeoutMs",
                    "The send deadline in milliseconds, applied to every later send on this socket. `0` makes sends non-blocking; a negative value raises `ErrInvalidArgument`.",
                    &[],
                    ParameterType::Integer,
                ),
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: super::native_body(lower_set_write_timeout, &[]),
        }],
    });
}
