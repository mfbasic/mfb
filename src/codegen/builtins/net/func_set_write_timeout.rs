//! `net::setWriteTimeout` — descriptor entry (native OS-seam). Overloaded over
//! `Socket` / `UdpSocket`. Docs in
//! `src/docs/man/builtins/net/setWriteTimeout.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

fn overload(ty: ParameterType) -> Implementation {
    Implementation {
        params: vec![
            super::req("sock", "The open connected TCP socket or bound UDP socket whose subsequent sends are to be bounded. The handle is borrowed, not consumed.", &[], ty),
            super::req("timeoutMs", "The maximum time a subsequent send may block waiting for buffer space, in milliseconds. `0` makes sends non-blocking (immediate `ErrTimeout` when no progress can be made); a positive value bounds the wait. Must not be negative.", &[], ParameterType::Integer),
        ],
        return_type: ParameterType::Nothing,
        errors: vec![],
        body: super::native_body(lower_set_write_timeout, &[]),
    }
}

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Bound how long a send on a socket may block."#;

const DESC: &str = r#"`net::setWriteTimeout` sets the maximum time, in milliseconds, that a send on
`sock` may block waiting for the host's send buffer to accept data. It applies to
a connected TCP `Socket` or a bound UDP `UdpSocket` and takes effect on every
subsequent send: `net::write` and `net::writeText` for a `Socket`, and
`net::sendTo` and `net::sendTextTo` for a `UdpSocket`. The socket is borrowed and
stays open.

The millisecond value is converted into a whole-seconds and microseconds pair and
installed as the socket's send-timeout option; the conversion is exact integer
division, so the value is used as given.

When the timeout elapses before the send can make progress, the pending send
fails with `ErrTimeout` rather than blocking further. It bounds a single
underlying send. That distinction matters for `net::write` and `net::writeText`,
which loop until the whole payload has been handed over: each iteration is
separately bounded, and a timeout in the middle of that loop raises
`ErrTimeout` after part of the payload has already been sent. A partially
written stream cannot be resumed from the error, so treat it as fatal to that
connection.

Per the language timeout convention (see `mfb spec language builtin-functions` →
"Timeout convention"), a `timeoutMs` of `0` makes subsequent sends
**non-blocking**: a send that cannot make progress fails at once with `ErrTimeout`
rather than waiting for buffer space. A positive value bounds the wait. A negative
`timeoutMs` is rejected with `ErrInvalidArgument`. The socket's *initial* state is
unbounded (a send blocks until buffer space frees); the setter can only bound, so
unbounded cannot be re-established through it once a bound is set."#;

const EX: &str = r#"Fail a TCP write that stalls for more than two seconds:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  net::setWriteTimeout(client, 2000)
  net::writeText(client, "hello")
  io::print("sent")
  RETURN 0
END FUNC
```

Bound a UDP send so a full buffer does not block forever:

```
IMPORT collections
IMPORT net
IMPORT io

FUNC main AS Integer
  RES sock = net::bindUdp("127.0.0.1", 0)
  net::setWriteTimeout(sock, 1000)
  LET dest = collections::get(net::lookup("127.0.0.1", 9000), 0)
  net::sendTextTo(sock, dest, "ping")
  io::print("sent")
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `net::set_write_timeout` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_set_write_timeout(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_poll::lower_net_set_timeout_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        true,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setWriteTimeout",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket or UdpSocket, Integer"),
        internal_only: false,
        implementations: vec![overload(super::socket())],
    });
}
