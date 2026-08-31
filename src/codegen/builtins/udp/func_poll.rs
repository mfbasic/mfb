//! `udp::poll` — descriptor entry (native OS-seam, plan-110-C). Return-type
//! overloaded exactly as `tcp::poll` is: a scalar socket answers `Boolean`, a
//! list answers with the first ready `Socket` (borrowed) through `udp.pollList`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::os::socket::{poll as gen_poll, shared as gen_shared};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Wait until a datagram is queued — on one socket, or one of a list."#;

const DESC: &str = r#"`udp::poll` reports whether a datagram is waiting, without reading it. The
following `udp::receive` returns that same datagram intact.

Given a single `Socket` it answers a `Boolean`: `TRUE` if a datagram is queued,
`FALSE` if the deadline passed with none. Readiness is a *query*, so an expired
deadline is a `FALSE`, not an error.

Given a `List OF RES Socket` it answers with the first socket that has one,
scanning in list order. The returned socket is an **alias** of the one in the
list: the list still closes each member exactly once at scope exit, so the result
must not be closed or transferred. An empty list raises `ErrInvalidArgument`, and
a deadline that expires with none ready raises `ErrTimeout` — unlike the scalar
form there is no value that could mean "nothing".

`timeoutMs` follows the language timeout convention (see `mfb spec language
builtin-functions` → "Timeout convention"). **Omitted, the call blocks** until a
datagram arrives. `0` polls once and returns immediately. A positive value bounds
the wait (clamped to `2147483647`). A negative value raises
`ErrInvalidArgument`. A signal that interrupts the wait re-issues it against the
remaining time rather than returning early.

Unlike a stream, readiness here is unambiguous: there is no "peer closed" state
that could make a ready socket yield nothing. A `TRUE` means a real datagram —
possibly a zero-length one — is queued."#;

const EX: &str = r#"Check without committing to a receive:

```
IMPORT net
IMPORT udp
IMPORT io

FUNC main AS Integer
  RES server = udp::bind("127.0.0.1", 0)
  RES client = udp::bind("127.0.0.1", 0)
  io::print("idle=" & toString(udp::poll(server, 50)))
  udp::send(client, udp::localAddress(server), "hi")
  io::print("ready=" & toString(udp::poll(server, 1000)))
  io::print("size=" & toString(len(udp::receive(server, 64).bytes)))
  RETURN 0
END FUNC
```

Serve whichever of several sockets has traffic first. The list form returns the
socket that is ready rather than a flag, and that socket is an alias of the one
in the list — receiving from it receives on that socket:

```
IMPORT net
IMPORT udp
IMPORT io

FUNC main AS Integer
  RES a = udp::bind("127.0.0.1", 0)
  RES b = udp::bind("127.0.0.1", 0)
  RES client = udp::bind("127.0.0.1", 0)

  ' Only b is sent anything.
  udp::send(client, udp::localAddress(b), "over here")

  LET socks AS List OF RES udp::Socket = [a, b]
  RES ready = udp::poll(socks, 5000)
  LET datagram = udp::receive(ready, 4096)
  io::print("got " & toString(len(datagram.bytes)) & " bytes")
  RETURN 0
END FUNC
```

prints:

```
got 9 bytes
```"#;

const TIMEOUT_DESC: &str = "Optional. The maximum time to wait, in milliseconds. Omit to block until a datagram arrives; `0` polls once; a positive value bounds the wait (clamped to `2147483647`); a negative value raises `ErrInvalidArgument`.";

/// `abi_function` body for `udp::poll` — the list overload arrives under the
/// `udp.pollList` code form.
pub(crate) fn lower_poll(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = if ctx.call == "udp.pollList" {
        gen_poll::lower_net_poll_list_helper(&symbol, ctx.platform_imports, ctx.platform)?
    } else {
        gen_poll::lower_net_poll_helper(&symbol, ctx.platform_imports, ctx.platform)?
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "poll",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket, Integer or List OF RES Socket, Integer"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    super::req(
                        "sock",
                        "An open bound socket to test. The handle stays open — you still close it.",
                        &[],
                        super::socket(),
                    ),
                    super::opt("timeoutMs", TIMEOUT_DESC, ParameterType::Integer),
                ],
                return_type: ParameterType::Boolean,
                errors: vec![],
                body: super::native_body(lower_poll, &[]),
            },
            Implementation {
                params: vec![
                    super::req(
                        "socks",
                        "The sockets to wait on, scanned in list order. The list still closes each socket; an empty list raises `ErrInvalidArgument`.",
                        &[],
                        ParameterType::list_of(ParameterType::Res(Box::new(super::socket()))),
                    ),
                    super::opt("timeoutMs", TIMEOUT_DESC, ParameterType::Integer),
                ],
                return_type: super::socket(),
                errors: vec![],
                body: super::native_body(lower_poll, &["pollList"]),
            },
        ],
    });
}
