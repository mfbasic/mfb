# bug-497: `tcp/tls/udp` write of a `String`-returning call selects the byte-list lowering → remote peer-controlled memory disclosure

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: CRITICAL
Class: security (memory safety — out-of-bounds read / remote information disclosure)

Status: Open (found in audit-3, Surface 4 OS-50; reproduced live end-to-end by the lead). Root cause of open bug-476.

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
