//! `tls::close` — descriptor entry (native OS-seam).
//!
//! `close` spans both handle types: `tls::close(Socket)` and
//! `tls::close(Listener)` — two overloads over the resource union, both
//! returning `Nothing` (the datetime/net idiom, no custom resolver). The public
//! name always lowers to `tls.close` for a socket; IR lowering rewrites a
//! `Listener` operand (and a listener scope-drop) to the internal
//! `tls.closeListener` body, which the listener overload declares as its code-form
//! `os_alias` so the generic OS dispatch routes it to this member's lowering.
//! `close` consumes the handle it is given.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Close a TLS socket or listener and release its OS handle."#;
const DESC: &str = r#"`close` shuts down a connected `Socket` and releases the resources behind it.
On Linux it performs an orderly TLS shutdown and frees the OpenSSL objects
(`SSL_shutdown`, `SSL_free`, `SSL_CTX_free`) before closing the underlying socket
file descriptor; on macOS it cancels the Network.framework connection. After a
successful return the socket is marked closed and must not be used again — any
later `tls::` call that takes the same value raises an error rather than touching
a stale handle.

`close` consumes the `Socket` it is given: the value is moved into the call and
cannot be referenced afterward. The call is idempotent with respect to a socket
that is already closed — closing a socket whose closed flag is already set does
nothing and returns successfully — so closing a socket and then letting it drop is
safe. This differs from `tcp::close`, which treats an already-closed resource as
an error.

`close` also closes a `Listener` from `tls::listen`. The same name spans both
handle types: given a listener it closes the listening socket and frees the
server TLS context the listener owns. Because every accepted `Socket` only
*borrows* that shared context, closing the listener is safe while accepted
sockets are still open — the context is freed exactly once, when the listener
closes, and an accepted socket's own close never touches it. The listener close
is likewise idempotent and consumes its handle.

Closing is otherwise automatic. Every `Socket` and `Listener` is closed by
lexical drop when the binding that holds it leaves scope. Call `tls::close` only
when the handle must be torn down earlier than that."#;
const EX: &str = r#"Close a TLS connection explicitly once the exchange is complete:

```
IMPORT encoding
IMPORT tls

SUB main()
  RES conn = tls::connect("example.com", 443)
  tls::write(conn, "GET / HTTP/1.0\r\n\r\n")
  LET response = encoding::utf8Decode(tls::read(conn, 4096))
  tls::close(conn)
END SUB
```"#;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `tls::close` — calls the shared `lower_tls_*_helper`
/// family dispatcher and finalizes.
pub(crate) fn lower_close(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = if ctx.call == "tls.closeListener" {
        super::gen_shared::lower_tls_close_listener_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?
    } else {
        super::gen_shared::lower_tls_close_helper(&symbol, ctx.platform_imports, ctx.platform)?
    };
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
        expected_arguments: Some("Socket or Listener"),
        internal_only: false,
        implementations: vec![
            // Socket close — lowers to the public `tls.close` body.
            Implementation {
                params: vec![Parameter {
                    name: "sock",
                    desc: "The connected TLS socket to close, as returned by `tls::connect` or `tls::accept`. The value is consumed by the call. Closing a socket that is already closed is harmless and returns successfully.",
                    aliases: &["resource"],
                    ty: ParameterType::named(super::TLS_SOCKET_TYPE_ID),
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Nothing,
                errors: vec![],
                body: Body::abi_function(lower_close),
            },
            // Listener close — rewritten to the internal `tls.closeListener` body
            // (the listener-shaped close), declared here as the code-form alias.
            Implementation {
                params: vec![Parameter {
                    name: "listener",
                    desc: "Alternatively, the listener to close, as returned by `tls::listen`. Closes the listening socket and frees the server TLS context it owns; safe to call while accepted sockets are still open. Consumed by the call; closing an already-closed listener returns successfully.",
                    aliases: &["resource"],
                    ty: ParameterType::named(super::TLS_LISTENER_TYPE_ID),
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Nothing,
                errors: vec![],
                body: Body::abi_function_aliased(lower_close, &["closeListener"]),
            },
        ],
    });
}
