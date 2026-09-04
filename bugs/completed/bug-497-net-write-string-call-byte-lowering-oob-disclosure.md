# bug-497: `tcp/tls/udp` write of a `String`-returning call selects the byte-list lowering → remote peer-controlled memory disclosure

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: CRITICAL
Class: security (memory safety — out-of-bounds read / remote information disclosure)

Status: FIXED (<HASH>). Found in audit-3, Surface 4 OS-50; reproduced live end-to-end by the lead. Root cause of bug-476.

Regression Test: an rt fixture asserting `tcp::write(sock, f(str))` writes exactly `f(str)`'s bytes; and a codegen-inspection test asserting the text helper is selected for a String-returning call.

## Summary

`tcp::write` / `tls::write` / `udp::send` are single overloaded members with a
`List OF Byte` form and a `String` form. The code form is chosen by the payload's
*static* type, but the selector fails **open**: when the type is unknown it picks
the byte-list form. A payload that is any user function call has unknown static
type (`static_type_name` returns `None` for a call not in a short hand table), so
a `String` returned by a call is read as a collection block — the write length
becomes the string's first 8 payload bytes and the source is 40 bytes past the
block header. Against a network peer those first 8 bytes are attacker-supplied, so
**the remote peer chooses how many bytes of process memory are sent back to it.**
Demonstrated: a 22-byte request with a length field of 1024 returned 1024 bytes of
live process memory (program strings + heap).

## Mechanism

```rust
// src/codegen/engine/value/builder_values.rs:2419
"tcp.write" => {
    if args.get(1).and_then(|arg| self.static_type_name(arg))
        .is_some_and(|type_| matches!(type_, ParameterType::String))
    { "tcp.writeText" } else { "tcp.write" }   // None -> byte-list form
}
```

```rust
// src/codegen/memory/value/builder_value_semantics.rs:1113
NirValue::Call { target, .. } | CallResult { .. } | RuntimeCall { .. } =>
    match target.as_str() {
        "replace" | "typeName" | "toString" => Some(String), ... ,
        _ => None,          // every user function and most builtins land here
    }
```

The byte form reads the `String` block `[u64 byteLength][bytes][nul]` as a
collection block: `load_u64(count, arg1, COLLECTION_OFFSET_COUNT)` takes the write
length from `String+8`, and the data base is `String+40`
(`src/codegen/builtins/tcp/gen_io.rs:819-831`). Verified in emitted code
(`_mfb_rt_tcp_tcp_write` prologue: `ldr x22,[x1,#8]` length, `add x23,x1,#40`
base). `udp.send` (`:2439`), `tls.write` (`:2469`) are identical; the same
`is_some_and(...)`-else-default shape is also used by `tls.connect`, `tcp.poll`,
`udp.poll`, etc. and should be audited together.

## Reproduction (lead-run, live)

`spikes/audit-3/OS-50/`:

```
mfb build spikes/audit-3/OS-50
./spikes/audit-3/OS-50/build/mfb_project.out &
python3 spikes/audit-3/OS-50/peer.py
# SENT 22 bytes; PEER GOT 1024 bytes
# leak.bin: "echoing 22 chars" (another live string) + arena/heap bytes
```

First 8 bytes = 1_000_000 → 65_536 bytes returned before an unmapped page.
Expected: exactly 22 bytes.

`http::handleRequest` writes `__http_serializeHead(resp)` (a call node); its head
starts `HTTP/1.1` (first 8 bytes ≈ 3.5e18) so the write fails and the server
answers nothing — the symptom in open bug-476, whose "read loop produced nothing"
hypothesis is wrong: it is this head write.

## Best fix

Two parts, both needed.
1. In `builder_values.rs`, make the selection type-correct: pick the text form
   when the payload's static type is `String`, the byte form when `ListOf(Byte)`,
   and when `static_type_name` is `None` resolve the callee's *declared return
   type* (`builtins::resolve_call_return_type` / `self.functions[..].returns` /
   `package_return_types`, already used at `builder_value_semantics.rs:1072`)
   before falling back. Audit the sibling `tls.connect`/`tcp.poll`/`udp.poll`/…
   selectors that share the fail-open shape.
2. Harden the sink: `lower_net_write_helper`'s byte mode should reject a block
   whose tag byte is not a collection tag and raise `ErrInvalidArgument` rather
   than read a length out of payload bytes, so a future mis-selection is a clean
   error, not a disclosure.

## Non-goals

No MFBASIC surface change (`tcp::write` stays one overloaded member); do not
remove the `List OF Byte` overload; correctly-typed calls' `.ncodesum` must not
move.

## Prior art

bug-476 (`bugs/bug-476-handlerequest-writes-no-response.md`, OPEN) records the
*symptom* on the http server without the cause or the disclosure consequence —
this is its root cause. Related: bug-157 (`tls::write` byte payload used count not
capacity for the base — same collection-base arithmetic, fixed there). Searched
`writeText`, `static_type_name`, `overload`, `handleRequest`.

## STATUS: FIXED (<HASH>)

Landed on `worktree-B-497`, merged into `main`.

**Reproduced first** (`spikes/audit-3/OS-50`, main compiler `2ec9835d9`): the
peer sent 22 bytes and received **1024 bytes** of process memory (`leak.bin`
began `"22"`, `"echoing 22 chars"`, arena pointers) — the documented mechanism,
`_mfb_rt_tcp_tcp_write` reading a `String` block as a collection (`ldr x22,[x1,#8]`
length, base `x1+40`).

**Fix, three parts:**

1. *Type-correct selection.* bug-476's `overload_arg_type` (merged from
   `worktree-B-476`) resolves a call to a NAMED function or package member. A
   30-shape audit (`/tmp`-built probes, one `tcp::write` per payload shape) found
   two `String` shapes still taking the byte form after it: a call **through a
   FUNC-typed value** (`LET f AS FUNC(String) AS String = reply` … `f(x)`) and a
   **field of a call result** (`makeRec().body`). `overload_arg_type` now resolves
   both (the callee's declared `Func(_, returns, _)`; `member_type_of`, split out
   of `static_type_name`, applied to a target only this resolver can type) and
   `ResultValue`. All 30 shapes (17 `String`, 13 `List OF Byte`) select correctly.
2. *Fail closed.* `builder_values::net_write_payload_form` replaces the three
   `is_some_and(String) … else bytes` selectors (`tcp.write`, `udp.send`,
   `tls.write`): `String` → text, `List OF _` → bytes, anything else is a codegen
   error naming the member. A payload shape the resolver cannot type is a build
   failure, never a guess. The sibling selectors (`tls.connect`, `net.ping`,
   `tcp/udp/tls.poll`, `tls.localAddress`, `io.print`) were moved onto
   `overload_arg_type` by bug-476; their fallbacks pick a *different* correct
   overload, not a different memory layout, so they are not fail-open in the
   memory-safety sense.
3. *Hardened sink.* One shared `codegen::memory::marshal::push_write_payload_view`
   emits the payload view for all five backends (tcp, udp, OpenSSL, macOS
   Network.framework, Schannel). Its byte form verifies the block header —
   `kind == list_block_kind(Byte)`, `keyType == NONE`, `valueType == BYTE`, which
   every byte-list producer in the tree writes — and branches to
   `<sym>_bad_payload` → `ErrInvalidArgument` before reading `count`. The text
   form is instruction-identical to the old per-backend code. Residual: a
   `String` whose length is exactly `0x0007_0002` (+16 MiB multiples) would still
   pass the header check; the selector is the closure, the sink is depth.

**After:** OS-50 peer receives exactly 22 bytes. Fixes bug-476's symptom
(`http::handleRequest` head write) at its root; bug-508 is the Schannel member of
the same family and lands in the same change.

**Tests:** `tests/codegen_net_write_payload_view.rs` (selection for all three
call shapes; header check + count-at-+8 in every byte-form helper on all five
targets — RED on main for both), `tests/rt_net_write_payload_shapes.rs` (the two
newly closed shapes arrive verbatim over loopback, exactly 22 bytes),
`tests/rt_native_write_overload_call_argument.rs` (bug-476), Schannel unit tests
`write_payload_tests`.

**Semantics:** no MFBASIC surface change. A correctly-typed program's
`tcp::write(sock, "literal")` / `tcp::write(sock, byteList)` selects the same
form as before; the only `.ncode` movement is inside the three byte-form runtime
helpers (header check) and the Schannel `tls::write` body (bug-508). See the
artifact-gate localisation in the landing commit.

## Fail-closed means the typing must be COMPLETE (regression found and fixed)

`net_write_payload_form` refusing an unresolved payload is the right closure —
guessing is what leaked memory. But it converts every gap in `overload_arg_type`
from "silently picks the wrong form" into "**refuses to build a valid
program**", which is a language-surface change by another route. The first
full-suite run after the fix caught exactly that:

```
tests/rt_macos_d4_union_state_tls.rs — build failed:
error: native runtime tls.write: payload static type <unresolved> is neither
String nor List OF Byte; refusing to select a lowering (bug-497)
       while lowering eval call tls.write while lowering match
```

The program is valid and had always compiled:

```
RES client AS Stream STATE PendingState = tls::accept(listener)
MATCH client
  CASE tls::Socket(t)
    tls::write(t, client.state.raw)
```

`member_type_of` knows record/union-variant fields, a thread handle's `result`
and a typed map's `key`/`value` — but not `res.state`, so **every payload
reached through a resource's STATE block was unresolved**. The lowering itself
had always known the type (`emit_member_access` reads `type_.state()`); only the
static side was missing.

Fixed by resolving `res.state` in `overload_arg_type`'s `MemberAccess` arm.
Deliberately **not** in `member_type_of`: that is shared with
`static_type_name`, which also gates the in-place append/set fast path, so
typing `client.state.raw` there would retype
`client.state.raw = collections::append(client.state.raw, …)` and shift that
statement's codegen and aliasing decision — far outside an overload choice, and
into bug-496's territory.

Pinned at codegen level on all five targets by two new rows in
`codegen_net_write_payload_view` (`st.state.raw` → byte form,
`st.state.note` → text form), so a future narrowing of the resolver fails the
selection assertion rather than a distant macOS TLS runtime test.

**Lesson for the next fail-closed selector:** the refusal set and the resolver
are two lists, and a shape missing from the resolver is a *valid program the
compiler now rejects*. The full suite — not the targeted tests — is what found
it; the targeted tests were green the whole time.
