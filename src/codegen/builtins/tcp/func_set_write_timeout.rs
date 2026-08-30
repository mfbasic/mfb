//! `tcp::setWriteTimeout` — descriptor entry (native OS-seam, plan-110-B).

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::{poll as gen_poll, shared as gen_shared};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Bound how long writes on a socket may block."#;

const DESC: &str = r#"`tcp::setWriteTimeout` sets the socket's write deadline, applying to every
subsequent `tcp::write` on it until changed. A write that reaches the deadline
without the OS accepting the remaining bytes raises `ErrTimeout`.

A write blocks when the OS send buffer is full, which happens when the peer is
not reading fast enough. Without a deadline a single slow or stalled peer can
hold a server thread indefinitely, so a server that writes to untrusted clients
should set one.

`timeoutMs` must not be negative — a negative value raises `ErrInvalidArgument`.
`0` means non-blocking: a write that cannot be accepted immediately raises
`ErrTimeout`.

Because `tcp::write` is a full write, a timeout can fire after *some* bytes have
already been handed to the OS. The call raises without reporting how many, so a
socket whose write timed out should be treated as being in an unknown state and
closed rather than reused."#;

const EX: &str = r#"Do not let a stalled reader hold the server:

```
IMPORT tcp
IMPORT io

FUNC main AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  RES client = tcp::connect("127.0.0.1", bound.port)
  RES conn = tcp::accept(server)
  tcp::setWriteTimeout(conn, 1000)
  tcp::write(conn, "hello")
  io::print("sent")
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `tcp::setWriteTimeout` (`write = true` → `SO_SNDTIMEO`).
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
                    "An open connected socket. Borrowed, not consumed.",
                    &[],
                    super::socket(),
                ),
                super::req(
                    "timeoutMs",
                    "The write deadline in milliseconds, applied to every later write on this socket. `0` makes writes non-blocking; a negative value raises `ErrInvalidArgument`.",
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
