//! `udp::bind` — descriptor entry (native OS-seam, plan-110-C).

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use super::gen_io;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::shared as gen_shared;
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Open a UDP socket bound to a local address and port."#;

const DESC: &str = r#"`udp::bind` opens a datagram socket on a local interface and port and returns the
`Socket` that `udp::send` and `udp::receive` work through. There is no connect
step and no peer: a single bound socket exchanges datagrams with any number of
addresses.

`host` selects the interface: `"127.0.0.1"` receives only local traffic,
`"0.0.0.0"` receives on every interface, and `""` is the same bind-all as
`"0.0.0.0"`.

Passing port `0` asks the OS to choose a free port, then `udp::localAddress`
reports which one it assigned. That is the right way for a client to bind — it
needs *a* port to receive replies on but does not care which, and picking one in
advance races every other process that wants it.

The returned `Socket` is a handle closed when its binding goes out of scope, or earlier with
`udp::close`."#;

const EX: &str = r#"Bind an OS-chosen port and report it:

```
IMPORT net
IMPORT udp
IMPORT io

FUNC main AS Integer
  RES sock = udp::bind("127.0.0.1", 0)
  LET at = udp::localAddress(sock)
  io::print("listening on port " & toString(at.port))
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `udp::bind`.
pub(crate) fn lower_bind(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        gen_io::lower_net_bind_udp_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "bind",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req(
                    "host",
                    "The local interface to bind. `\"0.0.0.0\"` or `\"\"` binds every interface; `\"127.0.0.1\"` receives only local datagrams.",
                    &[],
                    ParameterType::String,
                ),
                super::req(
                    "port",
                    "The local UDP port to bind. `0` asks the OS to choose a free port, which `udp::localAddress` then reports.",
                    &[],
                    ParameterType::Integer,
                ),
            ],
            return_type: super::socket(),
            errors: vec![],
            body: super::native_body(lower_bind, &[]),
        }],
    });
}
