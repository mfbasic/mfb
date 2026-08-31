//! `tcp::close` — descriptor entry (native OS-seam, plan-110-B). Spans both
//! resources as two overloads. This is the registered close op for `tcp.Socket`
//! and `tcp.Listener`, which is what makes it *consume* its argument and what
//! lexical drop routes through.

use crate::codegen::registry::{Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::shared as gen_shared;
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Close a TCP socket or listener and release its OS handle."#;

const DESC: &str = r#"`tcp::close` releases the operating-system socket behind a handle and marks it
closed, so any later `tcp::` call on the same value raises rather than touching a
stale descriptor.

`tcp::close` is the only `tcp` call that **consumes** its argument. Every other
function borrows the handle and leaves it open; `close` moves the value into the
call, after which it cannot be referenced again.

Closing a `Socket` tears down the connection, so a peer reading from it observes
the end of the stream. Closing a `Listener` stops new connections from being
accepted but does not affect sockets already returned by `tcp::accept`: each of
those is an independent resource with its own lifetime.

Closing is otherwise automatic. Every `tcp` handle is closed by lexical drop when
its binding leaves scope, so `tcp::close` is needed only to release earlier — to
free a listening port for reuse, to let a peer see the end of the stream promptly,
or to bound how many descriptors a long-running program holds open. Closing and
then letting the binding drop is safe: the drop sees the closed flag and does
nothing.

**An already-closed handle is an error rather than a no-op, and `tls::close`
deliberately differs** — there, closing twice succeeds. The two are otherwise
drop-in mirrors, so the split is worth knowing: code moved between the transports
must not assume either answer. Neither package will change under the other
without a decision, because each has callers relying on what it does today
(bug-465). In practice the difference is invisible to the recommended idiom —
close once, or let lexical drop do it — since a drop after an explicit close is a
no-op on both.

The handle's closed word
is checked first, and a non-zero value refuses the call. That word also carries
the *moved* bit that `thread::transfer` sets, so a handle transferred to another
thread is refused too — but with `ErrResourceMoved`, which names the real reason,
rather than `ErrResourceClosed`. The closed flag is set before the host close's
result is examined, so a host failure surfaces `ErrCloseFailed` exactly once and a
second close is refused rather than closing a descriptor number that may by then
name an unrelated file."#;

const EX: &str = r#"Release a listening port as soon as it is no longer needed:

```
IMPORT tcp
IMPORT encoding
IMPORT io

FUNC main AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  RES client = tcp::connect("127.0.0.1", bound.port)
  RES conn = tcp::accept(server)
  tcp::close(server)
  tcp::write(client, "hi")
  io::print(encoding::toUtf8Text(tcp::read(conn, 16)))
  RETURN 0
END FUNC
```"#;

fn overload(ty: ParameterType) -> Implementation {
    Implementation {
        params: vec![Parameter {
            name: "resource",
            desc: "The socket or listener to close. Consumed by the call and unusable afterwards. Also accepts the alternate named-argument spellings `sock` and `listener`.",
            aliases: &["sock", "listener"],
            ty,
            default: crate::codegen::registry::DefaultValue::None,
        }],
        return_type: ParameterType::Nothing,
        errors: vec![],
        body: super::native_body(lower_close, &[]),
    }
}

/// `abi_function` body for `tcp::close` — the shared handle-close emitter, the
/// same one `fs::close` and `udp::close` use.
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
        expected_arguments: Some("Socket or Listener"),
        internal_only: false,
        implementations: vec![overload(super::socket()), overload(super::listener())],
    });
}
