//! `tls::localAddress` — descriptor entry (native OS-seam, plan-110-D).
//!
//! Two overloads over the two handle types, mirroring `tcp::localAddress`
//! (bug-465). They cannot share one body: a `Socket` and a `Listener` are asked
//! different questions on macOS, so the `Listener` form declares the code-form
//! alias `localAddressListener` and `builder_values` routes to it off the
//! argument's static type — the same overload-split `tls::close` uses for its
//! `Listener` form.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str =
    r#"Report the local end of a TLS connection or the bound address of a listener."#;

const DESC: &str = r#"`tls::localAddress` returns the `net::Address` the OS has assigned to this end of
a handle — the local address and port a TLS `Socket`'s encrypted connection runs
over, or the address and port a `Listener` from `tls::listen` is bound to.

The address describes the *transport* underneath TLS, not the TLS session: it is
the same value the equivalent plaintext connection would report, and it says
nothing about the cipher suite or the peer's certificate.

The listener form's main use is learning the port after binding port `0`. Asking
the OS for a free port and then reading back which one it chose is the only
race-free way to bind: picking a port number in advance and hoping it is free
loses to any other process that wants it.

The handle stays open — you still close it. The result is an ordinary `net::Address`
value independent of the handle, so it stays valid after the handle is
closed.

**A file that uses the returned address must `IMPORT net` as well as `tls`.**
Imports are not transitive and packages cannot re-export types, so `net::Address` is
nameable only where `net` is imported.

**One platform difference, on the listener form only.** On Linux and Windows the
bound address is read back from the listening socket, so a listener bound by
*name* reports the resolved numeric address — `tls::listen("localhost", 0, …)`
reports `127.0.0.1`, exactly as `tcp::localAddress` does. macOS has no such
read-back: a TLS `Listener` there is a Network.framework `nw_listener`, which
exposes its port but not its address, so the host comes back as the string it was
bound with (`localhost`). The port is exact on every platform, and binding a
numeric host — the usual case, and the one that matters for port `0` — reports
the same value everywhere."#;

const EX: &str = r#"Report which local port a TLS connection went out on:

```
IMPORT net
IMPORT tls
IMPORT io

FUNC main AS Integer
  RES conn = tls::connect("example.com", 443, 5000)
  LET at = tls::localAddress(conn)
  io::print("local port " & toString(at.port))
  RETURN 0
END FUNC
```

Bind an OS-chosen port for a TLS server and report it, so a client can be told
where to connect:

```
IMPORT net
IMPORT tls
IMPORT io

FUNC main AS Integer
  RES server = tls::listen("127.0.0.1", 0, "cert.pem", "key.pem")
  LET bound = tls::localAddress(server)
  io::print("serving TLS on port " & toString(bound.port))
  RES conn = tls::accept(server)
  tls::write(conn, "hello")
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `tls::localAddress`. The `Listener` overload arrives
/// under the `tls.localAddressListener` code form: on macOS the two handle types
/// answer through different Network.framework calls, and on Linux/Windows both
/// reduce to `getsockname` over the descriptor in the record's handle slot.
pub(crate) fn lower_local_address(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = if ctx.call == super::LOCAL_ADDRESS_LISTENER {
        super::gen_shared::lower_tls_listener_address_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?
    } else {
        super::gen_shared::lower_tls_address_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
            false,
        )?
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

fn overload(
    type_id: &'static str,
    desc: &'static str,
    os_aliases: &'static [&'static str],
) -> Implementation {
    Implementation {
        params: vec![Parameter {
            name: "resource",
            desc,
            aliases: &["sock", "listener"],
            ty: ParameterType::named(type_id),
            default: DefaultValue::None,
        }],
        return_type: ParameterType::named(crate::codegen::builtins::net::ADDRESS_TYPE),
        errors: vec![],
        body: Body::abi_function_aliased(lower_local_address, os_aliases),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "localAddress",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket or Listener"),
        internal_only: false,
        implementations: vec![
            overload(
                super::TLS_SOCKET_TYPE_ID,
                "An open TLS socket whose local end to report. The handle stays open — you still close it.",
                &[],
            ),
            overload(
                super::TLS_LISTENER_TYPE_ID,
                "An open listener whose bound address to report — the way to learn the port after binding `0`. The handle stays open — you still close it.",
                &["localAddressListener"],
            ),
        ],
    });
}
