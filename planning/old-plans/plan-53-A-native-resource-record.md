# plan-53-A: a stateful native resource becomes an 80-byte record

Status: **CORE LANDED + validated.** The producing thunk builds the 80-byte record
(handle@0, closed@8, STATE@16 null → bind default-inits it); a param whose resource
TYPE is a stateful native resource loads the handle from FD@0 (bare-param `close`/
`exec` included — record-ness is per-type). The corruption repro (`String` STATE +
real native call) now runs clean: `native-resource-state-rt` opens a sqlite Db with
STATE, sets state (incl. a String field), runs real `exec` through a bare param
(the created table proves the handle round-tripped), reads state back, and closes.
Artifact gate **0 diffs / 1143 goldens** — every existing native binding is
byte-identical (the change is purely additive). 981 acceptance + 2901 unit green.

**The model change §3(b) proposed was unnecessary.** Registering a stateful native
resource as a `Reference` in `TypeModel` turned out not to be needed: the existing
"scalar" treatment copies the resource VALUE (now a record pointer) as an opaque
8-byte value, which is exactly right for a shared resource handle — copying a
pointer is what a resource bind/return should do. `.state` reads offset 16 of that
pointer (the record's STATE slot); the bind default-inits it; drop reclaims it
(plan-52-B). No `validation.rs` change was made.

**Guardrails — LANDED.** `TYPE_STATE_MISMATCH` now rejects two native declarations
of one resource with disagreeing STATE types (`check_link_functions`), with fixture
`tests/syntax/native/native-resource-state-mismatch-invalid`. The grammar spec
(`mfb spec language grammar`, `native-libraries`) documents STATE-on-native.

Last updated: 2026-07-17
Overall Effort: large (3h–1d)
Effort: medium (1h–2h)
Depends on: nothing (plan-52 A–D are landed and assumed)

Makes a native `LINK` resource that is declared with a `STATE` clause a real
**80-byte resource record** — a handle at `FILE_OFFSET_FD` (0), the closed flag at
`FILE_OFFSET_CLOSED` (8), and a STATE payload pointer at `FILE_OFFSET_STATE` (16) —
instead of the raw `CPtr` scalar every native resource is today. This is the
representation foundation without which STATE on a native resource cannot exist:
there is nowhere to put it.

The single outcome: **`FUNC open(path) AS RES SoundFile STATE FileInfo` (a native
`LINK` func) produces a resource whose `.state` reads a default-initialized
`FileInfo` (all zero), whose native handle survives a subsequent real native call,
and whose close op releases that handle — with NO memory corruption.** (Populating
the STATE from the call's `OUT` struct is plan-53-B; this sub-plan lands the record
and default-inits the STATE, exactly as a built-in `File STATE T` does.)

References:

- The demonstrated defect this fixes: a `STATE` attached to a native resource
  writes into the native handle's own memory (offset 16 of e.g. a `sqlite3*`),
  because the resource is a scalar. With a `String` STATE field + real handle use
  it fails `7-701-0001 Allocation failed`. Reproduced 2026-07-17.
- `planning/old-plans/plan-52-D-stateful-returns.md` §Non-goals — "LINK native
  funcs keep having no STATE clause… wrap it in an ordinary EXPORT FUNC." **This
  sub-plan reverses that**; see §3 for why the wrapper cannot express libsnd.
- `planning/res.md` §3.4 — the constructor-attached vs user-attached STATE
  distinction. libsnd is constructor-attached: every `SNDFILE*` has an `SF_INFO`.
- `bindings/libsnd/src/lib.mfb:109-121` — the motivating `openFile`/`closeFile`.
- `./mfb spec language resource-management` §15.5 (STATE model), `./mfb spec
  language native-libraries` (the LINK model), `./mfb spec language grammar` §17.

## 1. Goal

- A native `LINK` resource type named anywhere with a `STATE T` clause (on a native
  func's return or a native func's `RES` param) is modeled as a **resource**
  (a pointer to an 80-byte record), not a scalar and not a copyable record.
- The producing native thunk allocates the 80-byte record, stores the native
  handle at `FILE_OFFSET_FD`, zeroes `FILE_OFFSET_CLOSED`, and default-initializes
  the STATE payload at `FILE_OFFSET_STATE` (a pointer to an arena-allocated,
  zeroed `T` record) — identical to a built-in `File STATE T`.
- The close op for such a resource loads the handle from `FILE_OFFSET_FD` before
  the native call, rather than receiving the raw handle by value.
- **A bare native resource (no STATE clause anywhere) is byte-for-byte unchanged**
  — it stays the raw `CPtr` scalar. Zero regression to every existing native
  binding (sqlite3, etc.).
- `RESOURCE_RECORD_SIZE_BYTES` stays **80**.

### Non-goals (explicit constraints)

- **`BIND STATE`** (populating the STATE from an `OUT` struct) — that is plan-53-B.
  Here the STATE default-initializes, so the observable is `.state` reads zero.
- **Grammar for `BIND STATE`** — plan-53-B. This sub-plan parses only the `STATE T`
  clause on a native func return and param.
- **Changing bare native resources.** A native resource with no STATE stays a
  scalar. Do not migrate existing bindings; this is additive.
- **`RESOURCE`-declaration STATE.** `RESOURCE SoundFile STATE FileInfo` on the
  declaration stays rejected/undecided (plan-52-A Non-goal). The STATE is declared
  on the native func, as libsnd writes it — the compiler infers the type's
  record-ness from those declarations (§4).
- **Record size / offsets.** 80 bytes, offsets 0/8/16 exactly as a built-in
  resource. A native resource simply joins the existing layout.

## 2. Current State

- **Native resources are scalars.** `TypeModel::from_module_and_packages`
  (`src/target/shared/code/validation.rs:343-360`) deliberately skips native
  resource type exports from the record model "and let[s] them default to 8-byte
  scalars… its runtime value is a raw `CPtr` scalar handle — never a record."
  Registering them as records was rejected because it "would make the backend copy
  it by value on bind/return (an empty copy that loses the handle)." The fix is to
  register a *stateful* native resource as a **resource (reference)** — a pointer
  to a record, exactly like `File` — not as a copyable data record.
- **The native thunk returns the raw pointer.** `lower_link_thunk`
  (`src/target/shared/code/link_thunk.rs:336-979`) calls the native symbol
  (`:656-664`) and returns the handle through `emit_return_passthrough`'s `"CPtr"`
  path (`~:1035`). `return_resource` is read nowhere in codegen (only test fixtures
  at `:1433/:1461`). No 80-byte alloc, no FD/CLOSED/STATE stores.
- **The built-in precedent to mirror.** `fs::openFile`'s helper
  (`src/target/shared/code/fs_helpers_io.rs:752-803`) `arena_alloc`s
  `RESOURCE_RECORD_SIZE` (80), then writes `FILE_OFFSET_FD`(0)/`FILE_OFFSET_CLOSED`(8)
  and leaves the STATE/buffer words for `emit_resource_state_init`. Offsets:
  `src/target/shared/code/error_constants.rs:646-696`.
- **The STATE default-init to reuse.** `emit_resource_state_init`
  (`src/target/shared/code/builder_value_semantics.rs:10-36`) allocates a default
  `T` record and stores its pointer at `FILE_OFFSET_STATE` iff the slot is null.
  Runs at bind for a built-in (`builder_control.rs:293-297`). A native producer
  needs the equivalent — allocate a default STATE record inside the thunk (or make
  the bind path run it for a native resource too).
- **Native resource parsing.** `parse_link_function`
  (`src/ast/items.rs:748-907`) parses `AS [RES] type` at `:771-776` with
  `parse_type_name()` — which stops before `STATE` — then `consume_statement_end`
  rejects the leftover `STATE FileInfo`. The regular-func path parses the STATE via
  `parse_optional_state` (`:83-88`). `LinkFunction` (`src/ast/types.rs:312-330`)
  has no `return_state_type` and its params carry no state either.
- **IR.** `IrLinkFunction` (`src/ir/link.rs:394-431`) collected at
  `src/ir/lower.rs:320-397`. No STATE fields.
- **Close dispatch.** A native resource's close op is a native func whose `ABI (db
  CPtr)` receives the resource value directly (the raw handle today). Registered in
  the RESOURCE_TABLE; drop calls it (`emit_resource_cleanup_call`, plan-52-B).

## 3. Design Overview

Three layers, each mirroring an existing one.

**(a) Front end: parse the STATE clause on a native func return and param.** Call
`parse_optional_state()` after the native return type (`items.rs:771-776`) and let
native params carry `state_type` like ordinary params already do (§Current State —
`param` grammar already has `[ "STATE" type ]`). Carry both onto `LinkFunction` and
`IrLinkFunction`.

**(b) Model: a native resource named with STATE is a resource, not a scalar.** In
`TypeModel` (`validation.rs`), a native resource type that appears with a STATE
clause on any native func registers as a **resource reference** (pointer to record,
storage class `Reference`, size 8 — the value is a pointer, the record is 80 bytes
in the arena), the same class `File` uses. A native resource with no STATE anywhere
stays skipped → scalar, unchanged. The per-type decision is a scan over the
module's (and imported packages') native func declarations.

**(c) Codegen: the producing thunk builds the record.** When
`lower_link_thunk`'s function returns a *stateful* native resource, after the native
call: `arena_alloc(80)`, store the native handle (currently in `CRET`/`x0`) at
`FILE_OFFSET_FD`, zero `FILE_OFFSET_CLOSED`, allocate + store a default STATE record
at `FILE_OFFSET_STATE`, and return the record pointer instead of the raw handle.
Model: `fs_helpers_io.rs:752-803` for the alloc; `emit_resource_state_init` /
`lower_default_value` for the STATE record. The close thunk, for a stateful native
resource param, loads `FILE_OFFSET_FD` from the record before staging the `CPtr`
arg.

**Where the correctness risk concentrates:** the close op. A bare native resource's
close receives the raw handle; a stateful one must receive `record[FD@0]`. Getting
this wrong double-frees or closes garbage. It is gated behind the leak/rt tests in
Phase 3.

**Rejected: make ALL native resources records (uniform).** Cleaner conceptually,
but it migrates every existing native binding onto a new path — real regression
risk (close dispatch, thread transfer, the "copy by value loses the handle" hazard
the scalar path was created to avoid) for zero benefit to bare resources. Additive
(stateful → record, bare → unchanged) is the same end state for libsnd with none of
the blast radius.

**Rejected: a side table mapping handle → STATE.** The LUT res.md §10 rejected —
a dependent load on every `.state`, and cross-thread re-registration. A record slot
is a direct pointer.

**Rejected: STATE on the `RESOURCE` declaration.** plan-52-A Non-goal, and libsnd
does not write it that way. The compiler infers record-ness from the native func
STATE declarations instead (§4).

## 4. Detailed Design

### 4.1 Which native resources become records

A native resource type `R` is a **stateful native resource** iff some native `FUNC`
in scope declares `R` with a STATE clause — either `AS RES R STATE S` (a producer)
or a param `RES x AS R STATE S` (a consumer such as a close op). The STATE type `S`
must be identical across every such declaration for `R` (extend plan-52-C's
`TYPE_STATE_MISMATCH` to the native-func boundary — a resolver/verify check). A
stateful native resource is registered as a resource-reference in `TypeModel`; a
bare one is skipped (scalar) exactly as today.

### 4.2 The producing thunk (stateful native resource return)

After the native call and success gate, when the function's `return_resource` is set
and its `return_state_type` is `Some(S)`:

1. `arena_alloc(RESOURCE_RECORD_SIZE)` → record pointer `rec` (fail path: existing
   allocation-error return).
2. `store_u64(handle, rec, FILE_OFFSET_FD)` — `handle` is the native return value
   (the `CRET`/`RETURN <slot>` value the thunk already computes).
3. `store_u64(ZERO, rec, FILE_OFFSET_CLOSED)`.
4. Default-init STATE: allocate a default `S` record (reuse `lower_default_value`
   for `S`), `store_u64(state_rec, rec, FILE_OFFSET_STATE)`. (plan-53-B replaces the
   *value* via `BIND STATE`; the null-check in `emit_resource_state_init` means a
   populated slot is kept, so B composes by writing before this default runs — or
   by this default running only when B did not populate.)
5. Zero the buffer words (24–72) so the record is a valid resource record even
   though a native resource uses no File buffers (plan-52-B's `resource_uses_io_buffers`
   already returns false for a non-`File`, so drop won't free them; but they must be
   zeroed, not poison — mirror the File open helper).
6. Return `rec` (a pointer) instead of the raw handle.

### 4.3 The close thunk (stateful native resource param)

For a native `FUNC close(RES x AS R STATE S)` where `R` is a stateful native
resource: the incoming value is the record pointer. Before staging the `CPtr` arg
for the native symbol, `load_u64(handle, x, FILE_OFFSET_FD)` and pass `handle`.
Drop reclaims the STATE record via plan-52-B's `emit_resource_block_reclaim`
(state_type = S, has_io_buffers = false).

### 4.4 Interaction with plan-52-B reclamation

A stateful native resource is now a resource record, so plan-52-B's drop-path
reclaim applies: the STATE payload is freed at drop (`has_io_buffers=false`, so no
buffer frees — correct, a native resource has none). The 80-byte record is retained
as the tombstone. This is automatic once the record representation lands — verify it
does not double-free (the native close releases the OS handle; drop frees the STATE
record; they are disjoint).

## Compatibility / Format Impact

- **Bare native resources: unchanged.** Scalar `CPtr`, same codegen, same `.mfp`.
- **Stateful native resources: new.** No existing binding declares one, so nothing
  in the wild changes meaning.
- **`.mfp`:** a stateful native resource's exported signature carries `R STATE S`
  via the kind-11 type already added in plan-52-D. The RESOURCE_TABLE entry marks
  it native as before. Confirm the type export round-trips (Phase 2).
- **Codegen goldens:** a new native-resource-with-STATE fixture adds goldens; no
  existing fixture moves (bare native resources untouched). Verify via artifact gate.

## Phases

### Phase 1 — parse + model (front end, no codegen)

Delivers the grammar and the type-model decision. Safe alone: a stateful native
resource parses and is modeled as a resource, but no thunk change yet — so a
program using one still mis-codegens; this phase is verified by AST/IR goldens and
the model, not runtime.

- [ ] `src/ast/items.rs:771-776` — parse `parse_optional_state()` after the native
      return type; carry `return_state_type` onto `LinkFunction`
      (`src/ast/types.rs:312-330`). Native params already parse `STATE` via
      `parse_params` — confirm and thread it.
- [ ] `src/ir/link.rs:394-431` + `src/ir/lower.rs:320-397` — add `return_state_type`
      (and param state) to `IrLinkFunction`.
- [ ] `src/target/shared/code/validation.rs:343-360` — register a native resource
      named with STATE as a resource reference; keep bare ones skipped.
- [ ] Extend `TYPE_STATE_MISMATCH` (plan-52-C) to reject two native declarations of
      one resource with disagreeing STATE types (resolver or ir::verify).
- [ ] Tests: `tests/syntax/resources/native-resource-state-*` for the parse + the
      mismatch rejection; AST/IR goldens.

Acceptance: `FUNC open() AS RES SoundFile STATE FileInfo` parses and lands in AST/IR
with its STATE; two disagreeing STATE declarations for one native resource are
rejected `TYPE_STATE_MISMATCH`; the model reports SoundFile as a resource. Artifact
gate: only the new fixtures move.
Commit: —

### Phase 2 — the producing thunk builds the record

- [ ] `src/target/shared/code/link_thunk.rs` — when the function returns a stateful
      native resource, emit §4.2 (alloc 80, handle@0, zero CLOSED@8, default STATE
      @16, zero buffer words, return the record pointer). Model:
      `fs_helpers_io.rs:752-803` + `lower_default_value`.
- [ ] Confirm `arena_alloc`'s caller-saved clobber is handled (spill the handle
      before the alloc; `.ai/compiler.md`).

Acceptance: a native `open() AS RES R STATE S` (no `BIND STATE` yet) returns a
resource whose `.state` reads the default `S` (all zero) at runtime, and whose
handle survives a subsequent real native call (no `Allocation failed`, no corruption
— the exact program that failed in Current State now prints the default state and
runs clean).
Commit: —

### Phase 3 — the close thunk + reclamation (highest-risk last)

- [ ] `src/target/shared/code/link_thunk.rs` — a native close op whose `RES` param
      is a stateful native resource loads `FILE_OFFSET_FD` before staging the `CPtr`
      arg (§4.3).
- [ ] Confirm plan-52-B drop reclaims the STATE record (state_type=S,
      has_io_buffers=false) and does NOT double-free against the native close.
- [ ] Tests: `tests/rt-behavior/resources/native-resource-state-drop-*` — a loop of
      open/close on a stateful native resource showing flat per-resource retention
      and no crash; the leak-proof shape from plan-52-B.

Acceptance: a stateful native resource opens, its STATE reads default, it closes via
its native op with the handle correctly loaded from FD@0, and a loop of N cycles is
flat per resource with no corruption or double-free. Full suite green.
Commit: —

## Validation Plan

- Tests: the parse + mismatch fixtures (Phase 1); the runtime record + handle-survival
  fixture (Phase 2); the close + leak fixture (Phase 3). `tests/{syntax,rt-behavior}/resources/`.
- Runtime proof: **required and load-bearing.** The Current-State repro (native open
  with a `String` STATE field + a real native call) must go from `7-701-0001
  Allocation failed` to printing the default state and running clean. A build
  assertion cannot distinguish "record allocated" from "wrote into the handle."
  Uses `libsqlite3` as the exercisable native library (as `native-resource-link-valid`
  does).
- Doc sync: `./mfb spec language grammar` (native return/param may carry STATE),
  `./mfb spec language native-libraries` (native resources may be records),
  `./mfb spec language resource-management` §15.5 (native resources join the STATE
  model). Update the diagnostics registry if a new code is added.
- Acceptance: `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, `cargo test --bin mfb`.

## Open Decisions

- **Where the record-ness scan lives** — resolver vs. `TypeModel` construction.
  Recommend `TypeModel` (it already scans package resources and decides scalar-vs-record
  there), with the STATE-agreement check in ir::verify beside plan-52-C's rule.
- **Does a bare producer of a stateful native resource type make sense?** e.g. one
  native func returns `AS RES SoundFile` (bare) and another `AS RES SoundFile STATE
  FileInfo`. Recommend **reject** — once a native resource type is stateful, every
  producer must declare the STATE (it decides the representation). Same shape as
  plan-52-D's bare-return rejection, at the native boundary.

## Summary

The representation change is the whole of this sub-plan: a stateful native resource
stops being a raw handle and becomes a pointer to an 80-byte record, so there is a
slot to hold STATE. The risk is the close op (load the handle from FD@0, not the
value) and once-only reclamation against the native close. Everything is additive —
bare native resources never touch the new path — so no existing binding regresses.
plan-53-B then fills the STATE from the call's OUT struct; this sub-plan default-inits
it, proving the record and the handle round-trip before any marshalling.
