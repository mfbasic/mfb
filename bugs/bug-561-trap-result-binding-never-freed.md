# bug-561: an inline `TRAP` copies the producer's block into the `Result` and abandons the original

Last updated: 2026-09-06
Effort: medium
Severity: **HIGH** (unbounded leak on every fallible call in an expression)
Class: Memory / correctness

Status: **FIXED** (the two lowering paths whose provenance is already audited;
the third is filed as bug-566 — see "Residual")
Regression Test: `tests/rt_scope_drop_leaks.rs` —
`a_trap_bound_string_result_runs_at_constant_rss`,
`a_trap_bound_collection_result_runs_at_constant_rss`,
`a_trap_bound_inline_builtin_result_runs_at_constant_rss`, the POSITIVE pins
`a_trap_bound_scalar_result_still_runs_at_constant_rss`,
`the_same_producers_without_a_trap_still_run_at_constant_rss` and
`a_trap_whose_call_always_fails_still_produces_the_right_value`, and the
behaviour pin `every_trap_bound_result_still_produces_the_right_value`.

Found while fixing bug-536 shape B-2. Reproduced unchanged on the base commit
(`19880284452`) and is unaffected by that fix.

## The finding — and the correction to the original report

The report said the leak is "the `Result` wrapper, not the payload", and
"type-independent". **It is the opposite.** The `Result` block itself was always
freed (the `$trap_resN` binding is a `ResultOf`, which `is_freeable_flat_value`
admits, so its scope drop `arena_free`s it). What leaks is the block the
PRODUCER returned:

`emit_build_result_inline` builds the flat `{tag @0, size @8, payload @16}`
`Result` by **copying** the producer's block in whole. From that copy onward the
producer's own block has no owner — and the direct user-function raw path in
`builder_values.rs` never freed it. A scalar payload has no block, which is why
`Result OF Integer` never leaked at all and the leak *looked* wrapper-shaped.

Measured on the base compiler (macOS arm64, peak RSS via `/usr/bin/time -l`),
with an infallible control alongside each:

| bound type / producer | N=200k | N=400k | control, no `TRAP` |
| --- | --- | --- | --- |
| `String` from a user `FUNC` | **13.3 MB** | **25.6 MB** | 1.0 / 1.0 MB |
| `List OF Integer` from a user `FUNC` | **50.6 MB** | **100.2 MB** | 1.0 / 1.0 MB |
| `String` from `strings::mid` | **13.3 MB** | **25.6 MB** | 1.0 / 1.0 MB |
| `String` from `collections::get` | **13.3 MB** | **25.6 MB** | 1.0 / 1.0 MB |
| **`Integer`** from a user `FUNC` | 1.0 MB | 1.0 MB | — |

All flat at 1.0 MB after the fix. The report's "`Integer` 128 B/call, 25 MB at
200k, 50 MB at 400k" row does not reproduce; the shape that DOES produce those
numbers is a `String` payload.

The measured `String` callee must return `toString(...)` and not a `&` concat:
a user function that returns a **concatenation** leaks 64 B per call with no
`TRAP` anywhere, which is a separate defect (**bug-567**) and masks this one.

## Root cause, per lowering path

Three lowerings build a `Result` for an inline `TRAP`:

1. **Direct user / `.mfb` callee** (`builder_values.rs`, the `NirValue::CallResult`
   fall-through). Stores the raw success register into `payload_slot`,
   `emit_build_result_inline`s it, and returns. **Nothing frees `payload_slot`.**
2. **Inline builtin under `TRAP`** (`lower_inline_builtin_raw`,
   `lower_inline_infallible_raw`). Runs the member's ordinary lowering
   *directly*, bypassing `lower_value` — the only place `register_pending_temp`
   is called — so the member's fresh block never gets the statement-scope free
   the same member gets outside a `TRAP`.
3. **Runtime helper under `TRAP`** (`lower_runtime_helper_call(raw)` →
   `materialize_current_result`). Copies the helper's value into the current
   arena, inlines THAT copy into the `Result`, and frees the copy (bug-379) —
   abandoning the helper's original. **Not fixed here; see Residual.**

## The fix

The ownership question is one the tree already answers, in
`register_pending_temp`, for every plain-`Call` result. It is extracted verbatim
as `pending_temp_is_freeable(value, type_, fresh_string)` — behaviour-identical,
no call-site change — and asked again at paths 1 and 2:

* `register_call_result_payload_temp` registers the callee's block (from the slot
  it was spilled into) as a statement-scope pending temp, **on the Ok branch
  only**;
* `register_raw_member_result_temp` re-runs, for the raw inline-builtin paths,
  the registration the `lower_value` bypass skipped — on the exact
  `NirValue::Call` node the non-raw path would have carried, including the
  `mark_fresh_string` provenance match, so the answers are identical.

Because the predicate is the audited one, a `String` still needs the callee's own
`function_returns_fresh_string` promise or a native `mark_fresh_string`, a
`thread.*` result is still runtime-managed and untouched, a param-borrow or
rodata result is still copied rather than freed, and anything unproven keeps
leaking. Nothing is freed here that the plain-call path would not already free.

### Why this is not the double free the report warned about

The report's warning — "the success path binds the payload out of the `Result`
and the caller then owns it; freeing the wrapper must not free what was moved out
of it" — describes a *different* fix. Nothing is moved out: `ResultValue` is an
aliasing source, so `$trap_valN = ResultValue($trap_resN)` deep-copies out of the
`Result`, and `emit_build_result_inline` deep-copied INTO it. The producer's
block is a third, independent block with no other owner, and the free is emitted
strictly after the copy that consumed it.

The real hazard is the **error branch**, where the raw success register holds an
error code, not a block. The registration is therefore emitted on the Ok path
only, so its slot is written only there; the statement-end drop null-guards and
nulls it (bug-246 / bug-440), exactly as for any conditionally-initialized owned
temp. `a_trap_whose_call_always_fails_still_produces_the_right_value` is the
program where EVERY call fails, asserted on the exit status and the value.

### Spec

`mfb spec language memory-semantics` §14 preamble: "Each live value is owned by
exactly one binding, container slot, temporary, closure environment, thread
message, or return slot. Values are reclaimed by deterministic drop at the end of
the owning scope." The producer's block, once `emit_build_result_inline` has
copied it, is owned by nothing at all — it is not the `Result` (that is a copy,
§14.1: "Copy creates an independent value with no shared mutable state"), and it
is not the binding. The fix names its one owner — this statement's temporary —
and drops it at that scope. It only ADDS a free: `_mfb_arena_alloc` counts across
the whole builtin corpus are unchanged.

## Golden delta

35 goldens moved (of 1 947): 5 targets × http, json, net, strings, tcp, tls, udp
— the packages whose `.mfb` bodies inline-`TRAP` a call with a heap payload.
Every other fixture — csv, encoding, crypto, collections, regex, datetime, audio,
term, vector, io, math, fs — is byte-identical.

Diffed against the previous commit's compiler, per package: **zero functions
added or removed, zero data objects changed**, and 1 changed function (7 for
http):

| package | changed function(s) | `_mfb_arena_alloc` | `drop_owned_string` | `drop_owned_collection` |
| --- | --- | --- | --- | --- |
| json | `__json_parseHexQuad` | 397 → 397 | 356 → 357 | 186 → 186 |
| net / tcp / tls / udp | `__net_decodeQueryComponent` | unchanged | +1 | unchanged |
| strings | `__strings_fromScalars` | 230 → 230 | 125 → 126 | 298 → 298 |
| http | 6 `__http_*` + `__net_decodeQueryComponent` | 684 → 684 | 846 → 850 | 405 → 407 |

**The allocation count is unchanged everywhere and only frees are added — that is
the semantics proof.** At instruction level (`net`), the delta is one new stack
slot `call_result_payload_temp`, its prologue zero, a two-instruction spill of
the payload on the Ok path, one `bl _mfb_rt_drop_owned_string`, and an 8-byte
shift of every later slot offset.

## Residual — filed, not fixed

* **bug-566** — path 3 above, the runtime-helper raw `Result`. `fs::readText`
  under `TRAP` leaks ~190 B per call (4.7 MB at 20k, 8.4 MB at 40k), byte-identical
  before and after this change. `pending_temp_is_freeable` answers *false* for a
  `RuntimeCall` `String` (no mark can cross the helper boundary), which is the
  fail-closed answer; making it true needs a per-helper arena-ownership audit,
  because `thread.waitFor`'s result lives in the WORKER's arena.
* **bug-565** — the inline-`TRAP` ERROR path leaks ~780 B per trapped error
  (149.8 MB at 200k, 298.6 MB at 400k), byte-identical before and after.
* **bug-567** — `RETURN <concat>` from a user function leaks 64 B per call with
  no `TRAP` involved.

## On the `csv::parse` claim

The report (via bug-536) attributed "the entirety of `csv::parse`'s residual
112 MB per repeat call" to this bug and bug-560. That number does not reproduce
here: `csv::parse` over 1.26 MB of empty fields at 1, 2, 4 and 16 calls holds a
**flat 382.8 MB peak RSS on the base compiler and after both fixes** — there is
no per-call residual to attribute with that input. The leaks measured above are
real and are fixed; the 112 MB figure needs its own input recorded before anyone
tracks it again.
