//! `tcp::poll` — descriptor entry (native OS-seam, plan-110-B). Return-type
//! overloaded: a scalar socket answers `Boolean`, a list answers with the first
//! ready `Socket` (borrowed). The list form lowers through the `tcp.pollList` code
//! form; `builder_values` selects by the first argument's shape.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::builtins::net::{gen_poll, gen_shared};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Wait until a socket — or one socket out of a list — is readable."#;

const DESC: &str = r#"`tcp::poll` reports readability without consuming anything. It has two shapes.

Given a single `Socket` it answers a `Boolean`: `TRUE` if a following `tcp::read`
would return data (or observe the peer's close) without blocking, `FALSE` if the
deadline passed with nothing to read. Readiness is a *query*, so an expired
deadline is a `FALSE`, not an error.

Given a `List OF RES Socket` it answers with the first ready socket, scanning in
list order. The returned socket is **borrowed**: the list keeps ownership and
still closes each of its members exactly once at scope exit, so the result must
not be closed or transferred. This is the multiplex form — one call waits on many
connections instead of a thread per connection. An empty list raises
`ErrInvalidArgument`, and a deadline that expires with none ready raises
`ErrTimeout`, because unlike the scalar form there is no value that could mean
"nothing".

`timeoutMs` follows the language timeout convention (see `mfb spec language
builtin-functions` → "Timeout convention"). **Omitted, the call blocks** until
something is readable. `0` polls once and returns immediately. A positive value
bounds the wait (clamped to `2147483647`). A negative value raises
`ErrInvalidArgument`. A signal that interrupts the wait re-issues it against the
remaining time rather than returning early.

Readability includes the peer having closed: a closed peer makes `read` return
an empty list immediately, which is "would not block". So a `TRUE` here does not
promise that bytes are available, only that `read` will not wait."#;

const EX: &str = r#"Wait up to a second for data on one socket:

```
IMPORT tcp
IMPORT io

FUNC main AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  RES client = tcp::connect("127.0.0.1", bound.port)
  RES conn = tcp::accept(server)
  tcp::write(client, "hi")
  IF tcp::poll(conn, 1000) THEN
    io::print(toString(len(tcp::read(conn, 16))))
  END IF
  RETURN 0
END FUNC
```

Serve whichever of several connections speaks first:

```
IMPORT collections
IMPORT tcp
IMPORT io

FUNC serve(conns AS List OF RES tcp::Socket) AS Integer
  LET ready = tcp::poll(conns, 5000)
  LET chunk = tcp::read(ready, 4096)
  io::print("read " & toString(len(chunk)) & " bytes")
  RETURN 0
END FUNC
```"#;

const TIMEOUT_DESC: &str = "Optional. The maximum time to wait, in milliseconds. Omit to block until something is readable; `0` polls once; a positive value bounds the wait (clamped to `2147483647`); a negative value raises `ErrInvalidArgument`.";

/// `abi_function` body for `tcp::poll` — the list overload arrives under the
/// `tcp.pollList` code form, which builds a `pollfd[n]` over the list.
pub(crate) fn lower_poll(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = if ctx.call == "tcp.pollList" {
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
                        "An open connected socket to test for readability. Borrowed, not consumed.",
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
                        "The sockets to wait on, scanned in list order. The list retains ownership; an empty list raises `ErrInvalidArgument`.",
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
