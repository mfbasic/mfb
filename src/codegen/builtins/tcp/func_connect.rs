//! `tcp::connect` — descriptor entry (native OS-seam, plan-110-B). Four
//! argument-shape overloads (host/port, host/port/timeout, address,
//! address/timeout); the two `Address` forms lower to the `tcp.connectAddr` code
//! form. The overload split and timeout padding live in `builder_values`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::shared as gen_shared;
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Open a TCP connection to a host and port or to a resolved address."#;

const DESC: &str = r#"`tcp::connect` establishes an outbound TCP connection and returns a connected
`Socket`. The peer is named either by a host string plus a port, or by a
`net::Address` whose `host` and `port` fields supply both — so an address from
`net::lookup` can be passed straight through. When `host` is a name rather than a
textual IP address it is resolved first and the first resolved result is used.

Every connect takes the same non-blocking-connect plus readiness-poll path. The
socket is switched to non-blocking mode, the connect is issued, and the call then
polls for writability against a deadline; on success the original blocking mode
is restored and the socket's pending error is checked before the handle is built,
so a connection that failed asynchronously is reported as a failure rather than
handed back as connected. A signal that interrupts the poll re-issues it instead
of surfacing a spurious error.

`timeoutMs` follows the language timeout convention (see `mfb spec language
builtin-functions` → "Timeout convention"). **Omitted, the connect blocks** until
the connection completes or the OS refuses it — there is no hidden default
deadline. `0` is one immediate, non-blocking attempt: it succeeds if the connect
completes at once, otherwise it raises `ErrTimeout` without waiting. A positive
value bounds the attempt and raises `ErrTimeout` on the deadline. A negative
`timeoutMs` raises `ErrInvalidArgument`. On any failure the pending descriptor and
the resolver results are released first. Because the underlying wait takes a C
`int`, a deadline above 2147483647 milliseconds is clamped to that value.

**A caller that must not wedge on a black-holed peer passes a positive
`timeoutMs`.** Blocking forever is the documented behaviour of omitting it, not an
oversight.

The four overloads do not share a positional layout: `timeoutMs` is parameter 2
of the host/port forms but parameter 1 of the `Address` forms. Named arguments
therefore bind per-overload, against whichever overload the argument types select.

The returned `Socket` is a handle closed when its binding goes out of scope
when its binding leaves scope or earlier with `tcp::close`."#;

const EX: &str = r#"Connect to a local listener by host and port:

```
IMPORT tcp
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  RES client = tcp::connect("127.0.0.1", bound.port)
  io::print(toString(tcp::remoteAddress(client).port))
  RETURN 0
END FUNC
```

Connect to a resolved `net::Address` with an explicit deadline:

```
IMPORT collections
IMPORT net
IMPORT tcp
IMPORT io

FUNC main AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  LET dest = collections::get(net::lookup("127.0.0.1", bound.port), 0)
  RES client = tcp::connect(dest, 5000)
  io::print("connected")
  RETURN 0
END FUNC
```"#;

const TIMEOUT_DESC: &str = "Optional. The maximum time the connection attempt may take, in milliseconds. Omit to block until the connection resolves; `0` is one immediate attempt (`ErrTimeout` unless it completes at once); a positive value bounds the attempt and raises `ErrTimeout` when it elapses (clamped to `2147483647`); a negative value raises `ErrInvalidArgument`.";
const HOST_DESC: &str = "The peer's host name or textual IP address. Passed to the host resolver; a name with no address record raises an error.";
const PORT_DESC: &str =
    "The TCP port to connect to on the peer. Written directly into the resolved address.";
const ADDRESS_DESC: &str = "A destination supplying both the peer host and the peer port, typically from `net::lookup`. Replaces the separate `host` and `port` arguments.";

/// `abi_function` body for `tcp::connect` — the `Address` overloads arrive under
/// the `tcp.connectAddr` code form and read the endpoint out of the record.
pub(crate) fn lower_connect(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = if ctx.call == "tcp.connectAddr" {
        gen_shared::lower_net_connect_tcp_addr_helper(&symbol, ctx.platform_imports, ctx.platform)?
    } else {
        gen_shared::lower_net_connect_tcp_helper(&symbol, ctx.platform_imports, ctx.platform)?
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let ret = super::socket;
    pkg.add_function(RegistryFunction {
        name: "connect",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, Integer, Integer or Address, Integer"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    super::req("host", HOST_DESC, &[], ParameterType::String),
                    super::req("port", PORT_DESC, &[], ParameterType::Integer),
                ],
                return_type: ret(),
                errors: vec![],
                body: super::native_body(lower_connect, &[]),
            },
            Implementation {
                params: vec![
                    super::req("host", HOST_DESC, &[], ParameterType::String),
                    super::req("port", PORT_DESC, &[], ParameterType::Integer),
                    super::req("timeoutMs", TIMEOUT_DESC, &[], ParameterType::Integer),
                ],
                return_type: ret(),
                errors: vec![],
                body: super::native_body(lower_connect, &[]),
            },
            Implementation {
                params: vec![super::req("address", ADDRESS_DESC, &[], super::address())],
                return_type: ret(),
                errors: vec![],
                body: super::native_body(lower_connect, &["connectAddr"]),
            },
            Implementation {
                params: vec![
                    super::req("address", ADDRESS_DESC, &[], super::address()),
                    super::req("timeoutMs", TIMEOUT_DESC, &[], ParameterType::Integer),
                ],
                return_type: ret(),
                errors: vec![],
                body: super::native_body(lower_connect, &[]),
            },
        ],
    });
}
