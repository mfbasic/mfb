//! `tcp::localAddress` — descriptor entry (native OS-seam, plan-110-B). Spans both
//! resources: the bound address of a listener and the local end of a socket are
//! the same query.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};

use crate::codegen::builtins::net::{gen_io, gen_shared};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Report the local end of a socket or the bound address of a listener."#;

const DESC: &str = r#"`tcp::localAddress` returns the `net::Address` the OS has assigned to this end of
a handle — the local address and port of a connected `Socket`, or the bound
address and port of a `Listener`.

Its main use is learning the port after binding port `0`. Asking the OS for a
free port and then reading back which one it chose is the only race-free way to
bind: picking a port number in advance and hoping it is free loses to any other
process that wants it.

The handle is borrowed, not consumed. The result is an ordinary `net::Address`
value with no tie to the handle's lifetime, so it stays valid after the handle is
closed.

**A file that uses the returned address must `IMPORT net` as well as `tcp`.**
Imports are not transitive and packages cannot re-export types, so `Address` is
nameable only where `net` is imported. Without it the returned value has no
nameable type and the next call that consumes it fails to resolve."#;

const EX: &str = r#"Learn the port the OS chose:

```
IMPORT tcp
IMPORT io

FUNC main AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  io::print(bound.host & ":" & toString(bound.port))
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `tcp::localAddress` (`remote = false` → `getsockname`).
pub(crate) fn lower_local_address(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        gen_io::lower_net_address_helper(&symbol, ctx.platform_imports, ctx.platform, false)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
}

fn overload(ty: crate::types::ParameterType, desc: &'static str) -> Implementation {
    Implementation {
        params: vec![super::req("resource", desc, &["sock", "listener"], ty)],
        return_type: super::address(),
        errors: vec![],
        body: super::native_body(lower_local_address, &[]),
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
                super::socket(),
                "An open connected socket whose local end to report. Borrowed, not consumed.",
            ),
            overload(
                super::listener(),
                "An open listener whose bound address to report — the way to learn the port after binding `0`. Borrowed, not consumed.",
            ),
        ],
    });
}
