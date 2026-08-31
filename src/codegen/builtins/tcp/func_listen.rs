//! `tcp::listen` — descriptor entry (native OS-seam, plan-110-B). Two arities;
//! the omitted `backlog` is padded in `builder_values`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::shared as gen_shared;
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Bind a TCP port and start accepting connections on it."#;

const DESC: &str = r#"`tcp::listen` binds a local address and port and returns a `Listener` that
`tcp::accept` draws connections from. `host` selects the interface to bind:
`"127.0.0.1"` accepts only local connections, `"0.0.0.0"` accepts on every
interface, and `""` is the same bind-all as `"0.0.0.0"`.

Passing port `0` asks the OS to choose a free port. That is how a test or a
server that does not care which port it gets should bind — call
`tcp::localAddress` on the returned listener to learn which port was actually
assigned, rather than guessing one and racing another process for it.

`backlog` bounds how many completed-but-not-yet-accepted connections the OS
queues; it defaults to `128`. Connections beyond the backlog are refused or
dropped by the OS, not queued in the program.

**`tls::listen`'s backlog defaults differently** — to `0`, meaning the host
default — so a plaintext and a TLS listener written the same way do not get the
same queue depth. Pass an explicit `backlog` when the depth matters and the code
must behave identically on both transports (bug-465).

The address is bound with address reuse enabled, so a listener can rebind a port
whose previous connections are still winding down in the OS rather than failing
for the couple of minutes that would otherwise take.

The returned `Listener` is a handle closed when its binding goes out of scope, or earlier
with `tcp::close`. Closing it stops new connections from being accepted but does
not affect sockets already returned by `tcp::accept`: each of those is an
independent resource, closed on its own."#;

const EX: &str = r#"Bind an OS-chosen port and report it:

```
IMPORT tcp
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  io::print("listening on port " & toString(bound.port))
  RETURN 0
END FUNC
```

Bind every interface with an explicit backlog. A real server would name its port
(`tcp::listen("0.0.0.0", 8080, 16)`); this one asks the OS for a free one so it
can connect to itself and finish:

```
IMPORT tcp
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = tcp::listen("0.0.0.0", 0, 16)
  LET bound = tcp::localAddress(server)
  RES client = tcp::connect("127.0.0.1", bound.port)
  RES peer = tcp::accept(server, 1000)
  tcp::write(peer, "hello")
  io::print("served one connection")
  RETURN 0
END FUNC
```

prints:

```
served one connection
```"#;

/// `abi_function` body for `tcp::listen`.
pub(crate) fn lower_listen(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        gen_shared::lower_net_listen_tcp_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "listen",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, Integer, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req(
                    "host",
                    "The local interface to bind. `\"0.0.0.0\"` or `\"\"` binds every interface; `\"127.0.0.1\"` accepts only local connections.",
                    &[],
                    ParameterType::String,
                ),
                super::req(
                    "port",
                    "The local TCP port to bind. `0` asks the OS to choose a free port, which `tcp::localAddress` then reports.",
                    &[],
                    ParameterType::Integer,
                ),
                super::opt(
                    "backlog",
                    "Optional, defaulting to `128`. How many completed-but-unaccepted connections the OS queues before refusing more.",
                    ParameterType::Integer,
                ),
            ],
            return_type: super::listener(),
            errors: vec![],
            body: super::native_body(lower_listen, &[]),
        }],
    });
}
