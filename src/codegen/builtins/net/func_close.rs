//! `net::close` — descriptor entry (native OS-seam). Spans the resource union
//! (`Socket` / `Listener` / `UdpSocket`) as three overloads, all returning
//! `Nothing` and all lowering to `net.close` (the datetime/tls idiom, no custom
//! resolver). `close` consumes the handle it is given (see
//! the former source checker's `builtins::net_consumes_argument`). Docs in
//! `src/docs/man/builtins/net/close.md`.

use crate::codegen::registry::{Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

fn overload(ty: ParameterType) -> Implementation {
    Implementation {
        params: vec![Parameter {
            name: "resource",
            desc: "The network resource to close. Consumed by the call and unusable afterwards. This parameter also accepts the alternate named-argument spellings `sock` and `listener`, so `net::close(resource := s)`, `net::close(sock := s)`, and `net::close(listener := l)` all bind position 0.",
            aliases: &["sock", "listener"],
            ty,
            default: crate::codegen::registry::DefaultValue::None,
        }],
        return_type: ParameterType::Nothing,
        errors: vec![],
        body: super::native_body(lower_close, &[]),
    }
}

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Close a network resource and release its OS handle."#;

const DESC: &str = r#"`net::close` releases the operating-system socket behind a network resource and
marks the handle closed, so any later `net::` call that takes the same value
raises an error rather than touching a stale descriptor. It spans all three
`net` handle types: a connected TCP `Socket`, a TCP `Listener`, and a bound UDP
`UdpSocket`.

`net::close` is the only `net` call that **consumes** its handle. Every other
function borrows the resource and leaves it open; `close` moves the value into
the call, after which it cannot be referenced again.

Closing a `Socket` or `UdpSocket` tears down the connection or binding, so a peer
reading from a closed connection observes the end of the stream. Closing a
`Listener` stops it from accepting new connections but does not affect sockets
already returned by `net::accept`; each of those is an independent resource with
its own lifetime.

Closing is otherwise automatic. Every `net` resource is closed by lexical drop
when the binding holding it leaves scope, so `net::close` is needed only when the
handle must be torn down earlier — to free a listening port for reuse, to let a
peer observe the end of the stream promptly, or to bound how many descriptors a
long-running program holds open. Closing a resource and then letting it drop is
safe: the drop sees the closed flag and does nothing.

Unlike `tls::close`, `net::close` treats an already-closed handle as an error
rather than a no-op. The handle record's closed word is checked first, and a
non-zero value refuses the call. The same word also carries the *moved* bit that
`thread::transfer` sets, so a handle that was transferred to another thread is
refused too — but with `ErrResourceMoved`, which names the real reason it is
unusable, instead of `ErrResourceClosed`. The closed flag is set before the
result of the host `close` is examined, so a host failure surfaces
`ErrCloseFailed` exactly once and a second `net::close` on the same value is
refused rather than closing a descriptor number that may by then name an
unrelated file."#;

const EX: &str = r#"Release a listening port as soon as it is no longer needed:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  net::close(server)
  net::writeText(client, "hi")
  io::print(net::readText(conn, 16))
  RETURN 0
END FUNC
```

Close both UDP sockets explicitly at the end of an exchange:

```
IMPORT net

FUNC main AS Integer
  RES server = net::bindUdp("127.0.0.1", 0)
  RES client = net::bindUdp("127.0.0.1", 0)
  net::close(server)
  net::close(client)
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `net::close` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
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
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "close",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket or Listener or UdpSocket"),
        internal_only: false,
        implementations: vec![
            overload(super::socket()),
            overload(super::listener()),
            overload(super::udp()),
        ],
    });
}
