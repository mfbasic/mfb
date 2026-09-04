# bug-483 — a native overload's *code form* is selected from an argument's static type, and a CALL RESULT has no static type

Last updated: 2026-08-31
Effort: small (<1h) — the fix is one new probe and nine call sites
Severity: HIGH — silent miscompile (wrong helper, wrong marshalling) across ~10 members
Class: Codegen / overload selection

Status: **FIXED** — landed with bug-476 (same root cause; bug-476 is the symptom
that found it). This document exists so the *general* defect is findable: someone
hunting a `udp::send` or `io::print` miscompile will not search for an http bug.

## What was wrong

Several built-in members collapse two overloads into one name and choose the
*lowering* at codegen, in
`src/codegen/engine/value/builder_values.rs:CodeBuilder::lower_runtime_helper_call`,
from the static type of one argument:

| member | choice | deciding argument |
|---|---|---|
| `tcp::write`, `tls::write` | bytes vs text | arg 1 |
| `udp::send` | bytes vs text | arg 2 |
| `tcp::connect`, `tls::connect` | host/port vs `net::Address` | arg 0 |
| `net::ping` | host vs `net::Address` | arg 0 |
| `tcp::poll`, `udp::poll`, `tls::poll` | scalar vs list (**and the return type**) | arg 0 |
| `tls::localAddress` | `Socket` vs `Listener` | arg 0 |
| `io::print`, `io::write` | `AttributedString` → `toString(a)` rewrite | arg 0 |

The probe was `CodeBuilder::static_type_name`
(`src/codegen/memory/value/builder_value_semantics.rs`), whose `NirValue::Call`
arm is a **hand-written table of about a dozen builtins**
(`toString`, `len`, `strings.*`, `math.*`, `collections.get`, …). Every other
call — every user or package function, and every untabulated builtin — answered
`None`. `None` is not "unknown, ask someone else"; each selector reads it as
"not the special form" and silently emits the fallback lowering.

So `tcp::write(sock, buildHead(x))` emitted the **bytes** helper for a `String`
argument. That helper reads an element count out of `collection + COUNT` and a
data base derived from the capacity — applied to a `String*`, both are garbage.
The `write(2)` failed, and the failure path classifies a non-EAGAIN/EINTR error
as `ErrConnectionClosed` (77070004), so the call raised "the peer went away"
with a healthy peer and nothing on the wire. Binding the identical expression to
a `LET` first worked, which is what made every symptom look like a transport
fault instead of an overload-selection one.

Reproduction (no `http` involved), on the compiler before the fix:

```mfbasic
IMPORT tcp
IMPORT net
IMPORT io

FUNC head(n AS Integer) AS String
  RETURN "value=" & toString(n)
END FUNC

FUNC main AS Integer
  RES server AS tcp::Listener = tcp::listen("127.0.0.1", 0, 8)
  LET bound AS net::Address = tcp::localAddress(server)
  RES client AS tcp::Socket = tcp::connect("127.0.0.1", bound.port)
  RES conn AS tcp::Socket = tcp::accept(server)
  tcp::write(conn, head(7)) TRAP(e)
    io::print("write raised " & toString(e.code))   ' prints 77070004
    RECOVER
  END TRAP
  RETURN 0
END FUNC
```

## Fix

`CodeBuilder::overload_arg_type` — a probe used **only** for code-form selection.
It tries `static_type_name` first, then resolves a call against the same
return-type tables `emit_call` already uses (the NIR function set, then the
package return types), then falls back to `static_type_name_for_fold`'s registry
resolver. All nine type-driven selectors in `lower_runtime_helper_call` (plus its
two predicates `net_connect_is_address_form` / `net_poll_is_list_form`) read it.

It deliberately does **not** widen `static_type_name` itself. That function also
gates the in-place `collections::append`/`set` fast path, numeric-result typing
and the slice specialisation — including the `x = collections::append(x, f())`
aliasing decision, which is a known use-after-free shape in this tree. Naming
more call results there changes codegen far outside an overload choice.

## Regression tests

`tests/rt_native_write_overload_call_argument.rs`, one per selector *shape*
(they all read the same one line):

- bytes-vs-text — `tcp::write` (both forms) and `udp::send`
- scalar-vs-list — `tcp::poll` (also pins the return type, which the same probe picks)
- `AttributedString` rewrite — `io::print(astrings::fromString(…))`

All four measured RED on the unfixed compiler and GREEN after.

`tcp::connect`'s host/port-vs-`Address` shape is exercised there too but did NOT
reproduce: a record-returning call is spilled to a temporary before the selector
runs, so it already sees a `Local`. It is kept as a guard on the rewired
predicate, not as a witness.

**Still unproven by a test**, sharing the identical probe: `tls::write`,
`tls::connect`, `tls::localAddress`, `tls::poll` (each needs a TLS identity and a
live handshake) and `net::ping` (needs raw-socket permission).

## Related

- bug-476 — `http::handleRequest` served an empty reply. The same defect, reached
  through `tcp::write(sock, __http_serializeHead(resp))`; fixed by this change.
