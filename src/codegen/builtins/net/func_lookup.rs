//! `net::lookup` — descriptor entry (native OS-seam). Docs in
//! `src/docs/man/builtins/net/lookup.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Resolve a host name to a list of IPv4 network addresses."#;

const DESC: &str = r#"`net::lookup` hands `host` to the host resolver and returns the matching results
as a `List OF Address`. `host` may be a host name such as `"example.com"` or a
textual IP address; the resolver is asked for `SOCK_STREAM` endpoints. The result
list is built in the resolver's own order.

Only IPv4 results are returned. The resolver's answer chain is walked twice —
once to count `AF_INET` nodes and once to fill the list — and every node of any
other address family is skipped. The returned list can therefore be shorter than
the resolver's full answer, and it is empty when the host resolves but has no
IPv4 address. Note that the resolver failing outright is an error, not an empty
list.

Each returned `Address` carries a `host` field holding the textual IPv4 address
and a `port` field holding the requested port. `port` does not influence
resolution: it is not passed to the resolver as a service name but written
directly into each result's port field, so that the `Address` can be handed
straight to `net::connectTcp` or a UDP send. When `port` is omitted the compiler
supplies `0`, and every returned `Address` carries port `0`.

`net::lookup` exposes no resolver metadata — no record types, TTLs, or canonical
names — and adds no caching of its own beyond whatever the host resolver
provides. It opens no sockets and has no side effects; the resolver's answer
chain is released on both the success and the failure exits."#;

const EX: &str = r#"Resolve a host and inspect the first address:

```
IMPORT collections
IMPORT net
IMPORT io

FUNC main AS Integer
  LET addresses = net::lookup("127.0.0.1", 80)
  LET first = collections::get(addresses, 0)
  io::print(first.host & " " & toString(first.port))
  RETURN 0
END FUNC
```

Resolve without a port and print every result:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  LET addresses = net::lookup("localhost")
  FOR EACH address IN addresses
    io::print(address.host)
  NEXT
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `net::lookup` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_lookup(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_io::lower_net_lookup_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "lookup",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                super::req("host", "The host name or textual IP address to resolve. Passed to the host resolver as written; a malformed or unresolvable value raises an error.", &[], ParameterType::String),
                super::opt("port", "Optional, defaulting to `0`. The port recorded on every returned `Address`. It is stored on the results and does not influence resolution.", ParameterType::Integer),
            ],
            return_type: ParameterType::list_of(ParameterType::named(super::ADDRESS_TYPE)),
            errors: vec![],
            body: super::native_body(lower_lookup, &[]),
        }],
    });
}
