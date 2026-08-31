//! `udp::close` — descriptor entry (native OS-seam, plan-110-C). The registered
//! close op for `udp.Socket`, which is what makes it consume its argument and
//! what lexical drop routes through.

use crate::codegen::registry::{Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::shared as gen_shared;
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Close a UDP socket and release its OS handle."#;

const DESC: &str = r#"`udp::close` releases the operating-system socket behind a handle and marks it
closed, so any later `udp::` call on the same value raises rather than touching a
stale descriptor. It also frees the bound port for reuse.

`udp::close` is the only `udp` call that **consumes** its argument. Every other
function borrows the socket and leaves it open; `close` moves the value into the
call, after which it cannot be referenced again.

Because UDP is connectionless, closing tells no peer anything — there is no
shutdown handshake and no way for a sender to learn the socket is gone. Datagrams
addressed to a closed port are simply discarded by the OS.

Closing is otherwise automatic: every `udp` socket is closed by lexical drop when
its binding leaves scope, so `udp::close` is needed only to release earlier.
Closing and then letting the binding drop is safe — the drop sees the closed flag
and does nothing.

An already-closed handle is an error rather than a no-op, and a handle that
`thread::transfer` moved is refused with `ErrResourceMoved`, which names the real
reason, rather than `ErrResourceClosed`."#;

const EX: &str = r#"Release a bound port as soon as the exchange is done:

```
IMPORT net
IMPORT udp

FUNC main AS Integer
  RES server = udp::bind("127.0.0.1", 0)
  RES client = udp::bind("127.0.0.1", 0)
  udp::send(client, udp::localAddress(server), "bye")
  LET got = udp::receive(server, 16)
  udp::close(server)
  udp::close(client)
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `udp::close` — the shared handle-close emitter.
pub(crate) fn lower_close(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        crate::codegen::builtins::fs::gen_handle::lower_fs_close_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
            false,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "close",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "sock",
                desc: "The socket to close. Consumed by the call and unusable afterwards.",
                aliases: &[],
                ty: super::socket(),
                default: crate::codegen::registry::DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: super::native_body(lower_close, &[]),
        }],
    });
}
