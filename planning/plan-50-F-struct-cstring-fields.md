# plan-50-F: `CString` struct fields

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: plan-50-E (struct slots, `BIND IN`, the field-marshaling loop)

Lifts plan-50-E's `CString`-field restriction: a `const char *` field inside a C
struct becomes an owned MFBASIC `String` on the way out, and an MFBASIC `String`
becomes a call-lifetime C string on the way in.

This is the last compiler phase — after it, `SF_FORMAT_INFO` is fully
expressible and `getFormats()` is buildable in plain MFBASIC (plan-50-G).

It is split from plan-50-E because the pointer path is a different problem from
the scalar path: it drags in arena allocation, UTF-8 validation, NULL handling,
and — the reason it earns its own sub-plan — **an ownership question that a wrong
answer turns into either a leak or a double-free**.

The single behavioral outcome: `sf_command(NULL, SFC_GET_SIMPLE_FORMAT, &info,
24)` returns a record whose `name` is `"WAV (Microsoft 16 bit PCM)"` and whose
`extension` is `"wav"` — real strings read out of libsndfile's static tables,
owned by MFBASIC, with libsndfile's own storage left untouched.

References (read first):

- `src/target/shared/code/link_thunk.rs:emit_copy_cstring_to_string` (called at
  `:791`, defined near `:868`) — the exact machinery this phase reuses per field:
  copies a C string into an owned MFBASIC `String`, validates UTF-8 (→
  `ErrEncoding` `77020004`), maps NULL to `""`, and **leaves the source pointer
  untouched** (copy-and-leave).
- `src/target/shared/code/link_thunk.rs:emit_copy_string_to_cstring` (`:868`,
  called at `:439`) — the reverse: arena-allocates a NUL-terminated C buffer for
  the call's duration.
- `src/target/shared/code/link_thunk.rs:357-358` — `needs_encoding` gating, and
  the `encoding_fail` label the per-field path reuses.
- `.ai/compiler.md` §"Native Codegen Register Lifetimes" + memory
  `arena-alloc-clobbers-x14-x15`: **`_mfb_arena_alloc` destroys all of
  `x0`–`x17`.** Each `CString` field marshals via an allocating call, so this
  phase performs *N* clobbering calls inside one field loop.
- `src/docs/spec/language/17_native-libraries.md` — the `FREE` section: *"Without
  a `FREE` block a `CPtr` result is copied and the source pointer is left
  untouched (copy-and-leave), which leaks a caller-owned buffer."*
- `libsndfile-1.2.2/src/command.c:112-122` — `psf_get_format_simple`:
  ```c
  indx = data->format ;
  memcpy (data, &(simple_formats [indx]), SIGNED_SIZEOF (SF_FORMAT_INFO)) ;
  ```
  `simple_formats` is a **`static … const` table** (cf. `static SF_FORMAT_INFO
  const major_formats []` at `command.c:128`), so the `name`/`extension` pointers
  it hands back point into libsndfile's own read-only storage. **This is why
  copy-and-leave is correct here and `FREE` must not be applied** — freeing them
  would be a wild free of a static.
- Ground truth (gcc, aarch64 box 2223):
  `SF_FORMAT_INFO size=24 align=8 format=0 name=8 extension=16`.

## 1. Goal

- A `CSTRUCT` field of ctype `CString` maps to an MFBASIC `String` record field.
- **Out direction:** after the call, the field's `const char *` is copied into a
  fresh owned MFBASIC `String`; the native pointer is left untouched. Invalid
  UTF-8 fails with `ErrEncoding`; NULL yields `""`.
- **In direction:** an MFBASIC `String` record field is copied into an
  arena-allocated NUL-terminated C buffer whose pointer is written into the struct
  field, valid for the duration of the call.
- Plan-50-E's `NATIVE_STRUCT_FIELD_MISMATCH` rejection of `CString` fields is
  removed; `CPtr` fields stay rejected.
- `SF_FORMAT_INFO` round-trips: `format` in, `name`/`extension` out.

### Non-goals (explicit constraints)

- **No `FREE` for struct fields.** A caller-owned pointer field (one the C library
  expects you to `free`) is **rejected**, not leaked — see §4.4. libsndfile's
  format strings are static, so this phase needs no `FREE`, and inventing a
  per-field `FREE` with no in-tree consumer would be untested machinery.
- **No `CPtr` field escape.** Unchanged from plan-50-E: `NATIVE_CPTR_ESCAPE`.
- No change to the top-level `CPtr` → `String` **return** path (`:790`) or to
  `emit_copy_string_to_cstring`'s existing arg-slot use (`:439`). This phase
  *calls* them; it does not alter them.
- No `.mfp` format change (plan-50-C carried the ctypes already).
- No new rule codes — this phase *removes* a rejection.

## 2. Current State

Both halves of the machinery already exist and are proven by the top-level slot
paths:

| direction | helper | used today at | behavior |
|---|---|---|---|
| C → MFBASIC | `emit_copy_cstring_to_string` | `:791` (a `CPtr` return typed `String`) | copies, validates UTF-8 → `ErrEncoding`, NULL → `""`, **leaves source** |
| MFBASIC → C | `emit_copy_string_to_cstring` | `:439` (a `CString` argument slot) | arena-allocates a NUL-terminated copy, stores the pointer |

Neither has ever been called at a *field* offset — both write to a fixed frame
slot (`cret_off` / `cslot_off`). Plan-50-E's field loop (§4.4/§4.5) established
the per-field addressing this phase plugs them into.

Plan-50-E rejects a `CString` field with `NATIVE_STRUCT_FIELD_MISMATCH`. That
rejection is the thing this phase deletes.

The ownership rule is already written down for the return path and is exactly
transferable: without `FREE`, a `CPtr` result is copy-and-leave, which is correct
for library-owned storage and a leak for caller-owned storage. Struct fields
inherit the same fork.

## 3. Design Overview

```
  marshal_struct_out, per CString field:
      load_u64 %v9, [sp + struct_off + field.offset]   ; the const char* the C lib wrote
      store_u64 %v9, [sp + scratch]                    ; spill: the next call clobbers x0-x17
      emit_copy_cstring_to_string(scratch -> owned String)   ; existing helper
      reload record ptr from its spill slot             ; it did NOT survive the call
      store_u64 <string>, [record + 8*j]

  marshal_struct_in, per CString field:
      load record ptr from spill; load [record + 8*j]  ; the MFBASIC String
      emit_copy_string_to_cstring(-> arena C buffer)   ; existing helper (allocates!)
      store_u64 <c_ptr>, [sp + struct_off + field.offset]
```

**Where the risk concentrates — register lifetime, acutely.** Plan-50-E's field
loop performs at most one allocating call (the result record). This phase performs
**one allocating call per `CString` field**, inside the loop. Every one destroys
`x0`–`x17` with no survivor set. The rules, which the implementation must follow
mechanically:

1. Never hold a field value, a record pointer, or a loop index in a caller-saved
   register across a field's marshal call. Spill first, reload after.
2. The struct buffer is `sp`-relative and therefore survives by construction —
   re-derive field addresses from `sp` after every call, never cache them.
3. The record pointer must be reloaded from its spill slot after **every** field.

This is the bug class `.ai/compiler.md` calls out as *"layout- and value-sensitive:
the corrupted value may still produce correct results for small inputs and only
fail past a threshold"* — a 2-field struct may pass while a 6-field struct
corrupts. The memory `copy-record-register-aliasing` is the worked precedent
(a field-copy clobbered by an `x9` result-ptr reload).

**Ownership.** `simple_formats` is a `static const` table
(`command.c:112-122`, cf. `:128`), so the `name`/`extension` pointers are
library-owned and permanently valid. Copy-and-leave is correct and `FREE` would be
a wild free. The design does not guess: a struct field is **always**
copy-and-leave, and a binding that needs caller-owned field semantics is rejected
(§4.4) rather than silently leaked.

Rejected alternative: **support per-field `FREE`** (`FREE info.name` with a nested
deallocator). Rejected for now: no in-tree consumer, so it would be untested
machinery guarding a real memory bug — and the `FREE`-block IR is already the
weakest link on the package path (`IrFree` drops its ctypes;
`src/ir/verify/mod.rs:2756-2768`), which is not a foundation to extend. Rejecting
is honest and reversible; leaking is neither. (§Open Decisions)

Rejected alternative: **return `CString` fields as borrowed views** into the C
buffer to avoid a copy. Rejected outright: the pointer's lifetime is the C
library's, MFBASIC `String`s are owned arena values, and this is precisely the
`CPtr`-escape the ABI forbids.

## 4. Detailed Design

### 4.1 Out direction

Per `CString` field, inside plan-50-E §4.5's loop, after the record is allocated:

1. `load_u64` the field's pointer from `sp + struct_off + field.offset`.
2. Spill it to a scratch slot (the next step allocates).
3. Call `emit_copy_cstring_to_string` against that scratch slot, reusing the
   thunk's existing `alloc_fail` and `encoding_fail` labels.
4. Reload the record pointer from its spill slot; store the `String` at `8*j`.

`needs_encoding` (`:357`) currently gates on the *return* being a `CPtr`→`String`;
it must widen to "**or any mapped `CString` struct field exists**", or the
`encoding_fail` label is never emitted and the branch dangles. This is a concrete,
easy-to-miss break — the label is only materialized when `needs_encoding` is true.

NULL → `""` and invalid UTF-8 → `ErrEncoding` come free from the helper; both
must be tested per field, not assumed inherited.

### 4.2 In direction

Per `CString` field, inside plan-50-E §4.4's loop:

1. Reload the record pointer from its spill slot; `load_u64` the `String` at `8*j`.
2. Call `emit_copy_string_to_cstring` (allocates; `alloc_fail` on failure) to
   produce a NUL-terminated C buffer.
3. `store_u64` that pointer into `sp + struct_off + field.offset`.

Lifetime: the arena buffer must outlive the call, and does — it is arena-allocated
and the arena outlives the thunk. **Confirm when implementing** whether the
existing `:439` arg path frees its buffer after the call or lets the arena reclaim
it, and match that behavior exactly rather than inventing a second policy. Note
the spec's warning that embedded-NUL rejection is *intended but not enforced* in
the current marshaler; a `String` field with an interior NUL therefore truncates
on the C side. Inherit that known gap; do not silently paper over it, and record
it in the spec's struct-field section.

### 4.3 Direction and field marshaling

A `CString` field is marshaled **in** only for an `IN`/`INOUT` slot, and **out**
only for an `OUT`/`INOUT` slot — the same direction gate as scalar fields. An
`INOUT` struct with a `CString` field writes a C buffer in and reads a
(possibly different) pointer out; for `SFC_GET_SIMPLE_FORMAT` the input pointers
are overwritten wholesale by libsndfile's `memcpy`, so the in-direction buffers
are simply discarded — correct, if slightly wasteful. `getFormats` avoids the
waste by using an `OUT` slot and pinning `format` via a separate scalar field
(plan-50-G §4).

### 4.4 Rejecting caller-owned fields

There is no way for the compiler to *know* whether a `const char *` field is
library-owned or caller-owned — it is a fact about the C API, not the type. The
design's answer: **struct fields are always copy-and-leave**, documented as such,
and a binding whose C library hands back a caller-owned pointer field has no way
to express `FREE` and must not use a struct slot for it. Since no diagnostic can
detect the situation, this is a **specification obligation**, not a check: the
spec must state plainly that a `CString` struct field is copy-and-leave and that
using one for caller-owned storage leaks. Say it in the spec where a binding
author will read it.

## Compatibility / Format Impact

- **Changes:** a `CSTRUCT` may have `CString` fields; plan-50-E's rejection is
  removed. `needs_encoding` widens (§4.1), so thunks with `CString` struct fields
  gain an `encoding_fail` block.
- **Unchanged:** the `.mfp` format; scalar field marshaling; the top-level
  `CPtr`→`String` return path; `emit_copy_*` themselves; `CPtr`-field rejection.
- No rule codes added or removed (`NATIVE_STRUCT_FIELD_MISMATCH` survives — it
  still fires for `CPtr` fields, name/type mismatch, and partial coverage).

## Phases

One landable unit.

### Phase 1 — `CString` fields both directions

- [ ] `src/syntaxcheck/mod.rs` + `src/ir/verify/mod.rs`: stop rejecting `CString`
      struct fields; map `CString` ↔ `String` in the field type table. Keep the
      `CPtr` rejection.
- [ ] `link_thunk.rs`: widen `needs_encoding` (`:357`) to include mapped `CString`
      struct fields (§4.1) — otherwise `encoding_fail` is never emitted.
- [ ] `link_thunk.rs`: out-direction per-field copy (§4.1), spilling the pointer
      and reloading the record pointer around the allocating call.
- [ ] `link_thunk.rs`: in-direction per-field copy (§4.2), reloading the record
      pointer around each allocating call.
- [ ] Tests: `tests/rt-behavior/native/native-struct-cstring-rt/` — the runtime
      proof below.
- [ ] Tests: a struct whose `CString` field the C side leaves NULL → `""`; a field
      containing invalid UTF-8 → `ErrEncoding`. Both need a C fixture that can
      produce them — if libsndfile cannot, add a tiny purpose-built C fixture
      under `tests/_data/`, or state plainly on the ledger line that the NULL/UTF-8
      paths are covered only by the helper's existing top-level tests and are
      **not** independently proven at field granularity.
- [ ] Tests: **a ≥4-field struct with multiple `CString` fields** — the
      register-lifetime regression test. A 2-field struct can pass while a 6-field
      struct corrupts (`.ai/compiler.md`); the test must be wide enough to catch it.
- [ ] Spec: `17_native-libraries.md` — `CString` struct fields, both directions,
      the **copy-and-leave ownership rule and its caller-owned caveat** (§4.4), and
      the inherited embedded-NUL gap. Cite
      `[[src/target/shared/code/link_thunk.rs:emit_copy_cstring_to_string]]`.

Acceptance: the runtime proof returns `extension = "wav"` and a non-empty `name`
for format index 0 on **aarch64, x86_64, and riscv64**; the wide multi-`CString`
struct returns every field uncorrupted; `scripts/test-accept.sh` green;
`scripts/artifact-gate.sh` shows no churn in thunks without struct `CString`
fields.
Commits: `fab685f0` (capability + static coverage) and the bug-255 fix.
**Acceptance MET.** `tests/rt-behavior/native/native-struct-cstring-rt` runs a
real C-owned `const char *` field (`gmtime_r`'s `tm_zone`) back into a record
and copies it; `bindings/libsnd`'s `getFormats()` returns 17 correct formats with
two `CString` fields each. `scripts/test-accept.sh` green (964);
`scripts/artifact-gate.sh` 0 diffs across 1125 goldens — no churn in thunks
without struct `CString` fields.

**Landed note — this sub-plan's first implementation was wrong, see `bugs/bug-255`.**
It assumed a record's `String` field holds a **pointer**, so it allocated `8*n`
and copied each `const char *` into its own arena String. Records do not work
that way: every record except `Address`/`Datagram`/`DatagramText`/`AudioDevice`
**inlines** its `String` fields as `{len, bytes, NUL}` sub-blocks in a trailing
data region, and the word at `8*i` is the **block-relative offset** of that
sub-block, not a pointer (`record_field_is_inlined`,
`emit_build_inlined_record`). `marshal_struct_out` now measures every field's
length first, makes **one** allocation sized `8*n + Σ align8(len+9)`, and writes
the fixed slots plus the data region — mirroring the builder. The per-field
allocation is gone with it, and so is the "record pointer must survive N
allocations" hazard.

The tagged-helper work this sub-plan added (a label tag + a save slot, so
`emit_copy_cstring_to_string` could run per field) was **generality for a model
that does not exist**; it has been collapsed back to the single-caller
whole-return form. Its NULL-path fix — the path never wrote the save slot,
leaving the caller reading garbage — was a real latent bug and is kept.

## Validation Plan

- Tests: as above. The wide-struct register-lifetime test is the one that matters
  most — it is the only thing standing between this phase and a clobber bug that
  passes every small test.
- **Runtime proof** (Hard Completion Gate): a binding calling
  `sf_command(NULL, 0x1021 /* SFC_GET_SIMPLE_FORMAT */, info, 24)` with `info` an
  `INOUT SfFormatInfo` slot + `RETURN info`, its `format` field bound to 0 via
  `BIND IN`,
  asserting `extension == "wav"` and `name` non-empty. Run on aarch64 (2223),
  x86_64 (2228), riscv64 (2229) using the libraries vendored under
  `bindings/libsnd/vendor/`. A wrong offset shows up immediately: reading `name`
  at 16 instead of 8 returns `extension`, and reading at 12 returns a torn pointer
  → almost certainly a segfault, not a wrong answer.
- Doc sync: `src/docs/spec/language/17_native-libraries.md`; `cargo build`,
  `cargo test --bin mfb spec`, no leaked `[[` markers.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` +
  `scripts/artifact-gate.sh`.

## Open Decisions

- **Per-field `FREE`.** Recommend **defer and reject** (§4.4): no in-tree consumer,
  and the `FREE` IR is already the weakest package-path check
  (`src/ir/verify/mod.rs:2756-2768`). Alternative: implement it now for
  completeness — rejected, as untested memory-management machinery is worse than
  an honest gap. Revisit when a binding actually needs it.
- **In-direction `CString` fields at all?** `getFormats` needs only the out
  direction, and `SFC_GET_SIMPLE_FORMAT` overwrites the input pointers anyway
  (§4.3). Recommend implementing **both** regardless: the helper already exists,
  the symmetry costs ~15 lines, and shipping out-only would leave a hole the next
  binding hits immediately. Alternative: out-only, rejecting `CString` in an
  `IN`/`INOUT` slot — smaller, but a `CSTRUCT` field type whose legality depends
  on slot direction is a confusing rule to document.
- **Embedded NUL in an in-direction `String` field** silently truncates on the C
  side, inheriting the marshaler's existing documented gap. Recommend inheriting
  and documenting; fixing it belongs in a separate bug against
  `emit_copy_string_to_cstring`, which affects the existing `:439` arg path too.

## Summary

The risk is register lifetime, and it is worse here than anywhere else in
plan-50: one clobbering allocation **per field**, each destroying `x0`–`x17`, in a
loop. The defense is structural (the `sp`-relative buffer survives; spill and
reload everything else) and the wide multi-`CString` test is what proves it,
because narrow tests will not.

The ownership question is settled by evidence rather than assumption:
libsndfile's format strings live in a `static const` table, so copy-and-leave is
correct and `FREE` would be a wild free. Caller-owned pointer fields are rejected,
not leaked.

After this phase the compiler is done — `getFormats()` is ordinary MFBASIC, and
plan-50-G writes it.
