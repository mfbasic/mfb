//! `net::setReadTimeout` — descriptor entry (native OS-seam). Overloaded over
//! `Socket` / `UdpSocket`. Docs in
//! `src/docs/man/builtins/net/setReadTimeout.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

fn overload(ty: ParameterType) -> Implementation {
    Implementation {
        params: vec![
            super::req("sock", "The open connected TCP socket or bound UDP socket whose subsequent receives are to be bounded. The handle is borrowed, not consumed.", &[], ty),
            super::req("timeoutMs", "The maximum time a subsequent receive may block waiting for data, in milliseconds. `0` makes receives non-blocking (immediate `ErrTimeout` when no data is ready); a positive value bounds the wait. Must not be negative.", &[], ParameterType::Integer),
        ],
        return_type: ParameterType::Nothing,
        errors: vec![],
        body: super::native_body(lower_set_read_timeout, &[]),
    }
}

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Bound how long a receive on a socket may block."#;

const DESC: &str = r#"`net::setReadTimeout` sets the maximum time, in milliseconds, that a receive on
`sock` may block waiting for data. It applies to a connected TCP `Socket` or a
bound UDP `UdpSocket` and takes effect on every subsequent receive: `net::read`
and `net::readText` for a `Socket`, and `net::receiveFrom` and
`net::receiveTextFrom` for a `UdpSocket`. The socket is borrowed and stays open.

The millisecond value is converted into a whole-seconds and microseconds pair and
installed as the socket's receive-timeout option. Because the conversion is exact
integer division, a `timeoutMs` under one millisecond of resolution is not
rounded up — the value is used as given.

When the timeout elapses before any data arrives, the pending receive fails with
`ErrTimeout` rather than blocking further. The timeout governs only how long
a *single* receive waits for its first data; it does not cap the total time a
loop of receives may take, and it does not abort a receive that has already
started delivering bytes.

Per the language timeout convention (see `mfb spec language builtin-functions` →
"Timeout convention"), a `timeoutMs` of `0` makes subsequent receives
**non-blocking**: a receive with no data ready fails at once with `ErrTimeout`
rather than waiting. A positive value bounds the wait. A negative `timeoutMs` is
rejected with `ErrInvalidArgument`. The socket's *initial* state is unbounded
(a receive blocks until data), but the setter can only bound — it has no "restore
unbounded" value, so unbounded cannot be re-established through it once a bound is
set.

`net::setReadTimeout` bounds a blocking receive; `net::poll` instead asks whether
a receive would block at all. They compose: poll for readiness, and keep a
timeout installed as a backstop."#;

const EX: &str = r#"Fail a TCP read that stalls for more than two seconds:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  net::setReadTimeout(client, 2000)
  io::print("armed")
  RETURN 0
END FUNC
```

Bound a UDP receive so a missing reply does not block forever:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES sock = net::bindUdp("127.0.0.1", 0)
  net::setReadTimeout(sock, 1000)
  LET dg = net::receiveTextFrom(sock, 512)
  io::print(dg.value)
  RETURN 0
  TRAP(e)
    io::print(toString(e.code))
    RETURN 0
  END TRAP
END FUNC
```"#;

/// `abi_function` body for `net::set_read_timeout` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_set_read_timeout(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_poll::lower_net_set_timeout_helper(
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
        name: "setReadTimeout",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket or UdpSocket, Integer"),
        internal_only: false,
        implementations: vec![overload(super::socket())],
    });
}
