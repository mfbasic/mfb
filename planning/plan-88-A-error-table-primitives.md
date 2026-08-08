# plan-88-A: errorCode table as the single source of truth — audit + primitives

Last updated: 2026-08-03
Overall Effort: x-large (1d–3d)
Effort: medium (1h–2h)
Depends on: nothing

This is sub-plan **A** of plan-88. The feature consolidates *all* runtime-error
emission in native codegen onto one source of truth — the `errorCode` table
(`ERRORCODE_CONSTANTS` in `src/builtins/errorcode.rs`), keyed by error **name**.
Today the same `(code, message)` pair is authored in three unsynchronised
places (`error_constants.rs` `ERR_*` triples, the spec MD registry, and the
per-site literals), and emitted through three different code paths. The end
state: `ERRORCODE_CONSTANTS` defines each error once as `(name, code, message)`;
`BuiltinFunction.errors` is the per-function validated contract; a single
`raise_error(func_id, name)` codegen primitive (plus a func-less
`raise_error_bare(name)`) replaces every emit path; and a module-scoped *used
set* drives minimal emission of the error strings, retiring the manual gating.

**Sub-plan A delivers the audit and the dormant primitives**: it proves the
`ERR_* ↔ table` parity, then adds `raise_error` / `raise_error_bare` and the
used-set plumbing that *wrap the existing emission unchanged* (byte-identical),
with no caller migrated yet. Landing A changes no codegen output.

**Definition of done for the whole feature — both must hold, or it was
pointless:**

1. **Exactly two error-emission entry points exist:** `raise_error` and
   `raise_error_bare`. No `emit_*_return` wrapper, no `emit_error_code_return`
   caller, no `push_error_message_address` caller, no third path anywhere.
2. **Exactly one error-metadata location exists:** `ERRORCODE_CONSTANTS`. No
   `ERR_*` code/message/symbol constant remains anywhere.

**Byte-identical codegen is NOT the goal.** The gate is a **runtime test per
modified site**, not a golden diff. Every letter after A obeys this **per-site
acceptance gate**:

1. **Before** modifying an error site, there must be a runtime test that triggers
   that error and asserts its `Error.code` (and message). If none exists, **add
   one** — an `.mfb` fixture under `tests/rt-error/<area>/<name>/` (the existing
   runtime-error suite) — and confirm it **passes before** the change.
2. **After** the change, re-run it:
   - **Different `Error.code` → FAIL.** A code change is a real regression, full
     stop.
   - **Different message → investigate.** It is acceptable *only if* it is a
     consolidation artifact: a given code could be emitted with more than one
     message string today (the codegen `ERR_*_MESSAGE` vs the table message — the
     only divergence axis, since no code is shared across two `ERR_*` consts), and
     the feature collapses that to the single table message. If the new message is
     the table message for that code and the code was on the Phase-0 expected-change
     list, update the test’s expected message and note it. Otherwise it is a FAIL.
   - **Same code + same message → pass.**

The word “byte-identical” appears in **A only** (where it is a true artifact of
migrating nothing). B, C, D are gated on the rt-error tests above; goldens
re-baseline freely as emission unifies.

References:

- `src/builtins/errorcode.rs` — `ERRORCODE_CONSTANTS` (the table) + `runtime_error` accessor
- `src/builtins/descriptor.rs` — `BuiltinFunction.errors`, `REGISTRY.function`
- `src/target/shared/code/error_constants.rs` — the `ERR_*` triples to retire
- `src/target/shared/code/builder_error_emission.rs` — `emit_error_code_return`, the `emit_*_return` wrappers
- `src/target/shared/code/data_objects.rs` — `push_error_message_address`, the manual `push_string_value` gating
- `src/docs/spec/diagnostics/02_error-codes.md` — the spec registry (message source of truth for the table)
- `.ai/compiler.md` — runtime-completion gate, validation & function tests
- Reference implementation already in the worktree: `collections.get`’s out-of-range path in `builder_collection_query.rs` `lower_list_get_common`, and `prepend_wrong_mode_gate` taking a `function_id`.

## Prerequisites

These hold for every letter of plan-88; stated once here.

| Must be true | Command | Status |
|---|---|---|
| On a working branch, not `main` | `git branch --show-current` (≠ `main`) | MET (`worktree-new-man`) |
| Worktree builds green | `cargo build --bin mfb` | MET |
| Descriptor `errors` field exists on `BuiltinFunction` | `grep -n 'pub(crate) errors' src/builtins/descriptor.rs` | MET |
| `errorcode::runtime_error` accessor exists | `grep -n 'fn runtime_error' src/builtins/errorcode.rs` | MET |
| `ERRORCODE_CONSTANTS` is `(name, code, message)` | `grep -n 'ERRORCODE_CONSTANTS: &\[(&str, &str, &str)\]' src/builtins/errorcode.rs` | MET |

> The Status column is a snapshot; re-run each Command before continuing. The
> worktree currently carries the uncommitted descriptor-field / errorCode-3-tuple
> / collections-threading / codegen-probe work — commit or confirm that in tree
> before starting A so the parity audit runs against the intended base.

## 1. Goal

- **A**: `ERR_* ↔ ERRORCODE_CONSTANTS` parity is proven (a test asserts every
  `ERR_*_CODE`/`ERR_*_MESSAGE` equals the table row for the same code), and
  `raise_error(func_id, name)` / `raise_error_bare(name)` exist and emit
  **byte-identical** output to the wrapper they wrap, recording each raised name
  into a module-scoped used-set that nothing consumes yet.

### Non-goals (explicit constraints)

- No caller is migrated in A (that is B/C). No `ERR_*` constant, wrapper, or
  gating is deleted in A (that is D).
- A **itself** changes no codegen output — but only because A migrates nothing:
  `raise_error` in A delegates to the existing `emit_error_code_return`, and the
  one poc caller already emitted identically. This byte-identity is an artifact
  of A's small scope, **not** a constraint the feature must uphold (B/C/D will
  and should change bytes as emission unifies — see Definition of done).
- `ErrWrapped` (dynamic, composed at runtime) is out of scope — it is not on the
  emit path (verified below).

## 2. Current State

Runtime errors are emitted through three paths, all authored against
`error_constants.rs` `ERR_*` constants rather than the table:

1. **Per-code wrappers** — `builder_error_emission.rs:4-49` defines 12 thin
   `emit_*_return(&mut self)` methods, each `emit_error_code_return(ERR_X_CODE,
   ERR_X_MESSAGE)`. Called from 146 sites.
2. **Direct `emit_error_code_return`** — 25 non-wrapper call sites (all
   allocation-failure: `ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE`) plus
   the 1 `collections.get` proof-of-concept (already table-driven).
3. **Symbol path** — `push_error_message_address(.., ERR_X_SYMBOL, ..)` loads a
   pre-interned runtime string symbol into `RESULT_ERROR_MESSAGE_REGISTER`; used
   by fixed native-helper assembly. 49 calls across 11 files. Each referenced
   symbol’s string data object is emitted by a **manual** `push_string_value(&mut
   values, ERR_X_MESSAGE)` in `data_objects.rs` (18 gating sites) — miss one and
   the relocation dangles at link (the “bug-256 class”, documented throughout
   `data_objects.rs`).

The reference implementation for the target shape is already wired:
`collections.get` resolves `(code,message)` from the descriptor→table via
`errorcode::runtime_error` and emits byte-identically; `prepend_wrong_mode_gate`
already takes a `function_id` and looks up the descriptor.

### Measured populations

| What | Count | Command |
|---|---|---|
| errorCode table entries | 44 | `grep -cE '^    \("Err' src/builtins/errorcode.rs` → 44 |
| `emit_*_return` wrapper fns | 12 | `grep -cE 'pub\(super\) fn emit_[a-z_]+_return\(&mut self\) -> Result' src/target/shared/code/builder_error_emission.rs` → 12 |
| wrapper call sites | 146 | `grep -rcE 'self\.emit_[a-z_]+_return\(\)' src/target/shared/code/*.rs \| awk -F: '{s+=$2} END{print s}'` → 146 |
| direct `emit_error_code_return` callers (non-wrapper, non-poc) | 25 | 38 total lines − 12 wrapper bodies − 1 poc (`grep -rnE 'emit_error_code_return\(' src/target/shared/code/*.rs \| grep -v 'fn emit_error_code_return'` → 38) |
| symbol-path calls (`push_error_message_address`) | 49 | `grep -rcE 'push_error_message_address' src/target/shared/code/*.rs \| awk -F: '$2>0{s+=$2} END{print s}'` → 49 |
| files using the symbol path (excl. `data_objects.rs`) | 11 | `grep -rlE 'push_error_message_address' src/target/shared/code/*.rs \| grep -v data_objects.rs \| wc -l` → 11 |
| `ERR_*` consts in `error_constants.rs` (41 code/41 msg/41 symbol) | 123 | `grep -cE 'pub\(crate\) const ERR_' src/target/shared/code/error_constants.rs` → 123 |
| total consts in `error_constants.rs` | 345 | `grep -cE 'pub\(crate\) const ' src/target/shared/code/error_constants.rs` → 345 |
| manual gating sites in `data_objects.rs` | 18 | `grep -cE 'push_string_value\(&mut values, ERR_[A-Z_]+_MESSAGE' src/target/shared/code/data_objects.rs` → 18 |
| operator-family wrapper calls (bare candidates) | 37 | overflow 21 + underflow 2 + float_domain 6 + float_nan 3 + float_inf 3 + float_overflow 2 (`grep -rcE 'self\.emit_<w>_return\(\)' …`) |

### Verified properties

- **Every `ERR_*_CODE` value is in the table.** `comm -23 <(ERR_*_CODE values) <(table codes)` → empty. Three table codes have no `ERR_*` const: `77050006` (ErrPermissionDenied), `77050016` (ErrAuthenticationFailed), `77060001` (ErrWrapped). So `ERR_* codes ⊂ table codes` (41 ⊂ 44). **Message-text equality is NOT yet verified — that is Phase 0.**
- **`ErrWrapped` is not on the emit path.** `grep -rln 'ErrWrapped\|ERR_WRAPPED' src/target/shared/code/*.rs` → none; it appears only in `src/builtins/errorcode.rs`. It is composed dynamically and stays on separate machinery.
- **The `collections.get` poc is byte-identical and green.** `cargo build --bin mfb` clean; `cargo test --bin mfb` → 3751 passed. (The poc emits the same `("77050001", "List or string index/range is outside valid bounds.")` the old wrapper did.)
- **`ERR_*` names are not 1:1 with table names** — some are code-aliases (`ERR_ALLOCATION`→`ErrOutOfMemory`, `ERR_OUTPUT`→`ErrWriteFailed`, `ERR_EOF`→`ErrEndOfFile`, `ERR_INPUT`→`ErrInputFailed`, `ERR_NATIVE_LINK_LOAD`→`ErrNativeBindingUnavailable`, `ERR_NATIVE_LINK_CALL`→`ErrNativeBindingCallFailed`). The audit maps **by code**, not by name. `ERR_DIRECTORY_NOT_EMPTY` needs its table code confirmed in Phase 0.

## 3. Design Overview

Three layers, each landed by a later letter:

- **The table is the hub** (`ERRORCODE_CONSTANTS`): `name → (code, message)`. The
  error **name** is the logical key. `BuiltinFunction.errors` lists the names a
  function may raise — a *documented contract*, validated, never an index the
  codegen reads positionally.
- **One primitive, two entry points** (this sub-plan): `raise_error(func_id,
  name)` validates `name ∈ func.errors`, looks up `(code,message)` from the
  table, emits, and records `name` into a module used-set. `raise_error_bare(
  name)` is the func-less form for operator/TRAP sites (skip the contract
  check). Both **wrap the existing `emit_error_code_return`** so output is
  unchanged until D.
- **The used-set drives emission** (activated in D): every raise records its
  name; at module finalization only the referenced error strings are emitted —
  replacing the 18 manual gating sites. Until D the used-set is inert and the
  existing string emission stands.

**Correctness risk** concentrates in B and C (172+49 call-site migrations that
must preserve the *error raised at runtime* — same code+message) and in D
(switching to used-set emission without a dangling relocation, then deleting the
old path). B stays byte-identical for free (same underlying emit); C deliberately
changes bytes as the helper shape unifies (goldens re-baseline). A’s risk is low:
dormant code + a test.

**Design uncertainty** (schedule first, here in A): does the **table text**
exactly equal every `ERR_*_MESSAGE` today? If not, migrating changes the *message
a program observes*, which is a real behavior change, not a golden churn. Phase 0
resolves this by asserting parity of the table code+message against every `ERR_*`
constant. (There is no symbol-scheme uncertainty — that is resolved: one scheme,
re-baseline; see Definition of done and Open Decisions.)

Rejected: **emit all 44 error strings unconditionally** (~3–4KB rodata,
`grep`-estimated from message lengths) — rejected in favor of the used-set,
which keeps binaries minimal *and* removes the manual gating. Rejected:
**positional `errors[0]`** as the codegen source — a site names its own error;
`errors` is only the validated contract (a function has multiple error sites).

## 4. Detailed Design — primitives

- `errorcode::runtime_error(name) -> Option<(&'static str, &'static str)>`
  already returns `(code, message)`. Add `errorcode::message_symbol(name) ->
  &'static str` **only if** C adopts a name-derived symbol (Open Decision);
  A does not need it.
- On the code builder (same `impl` as `emit_error_code_return`):
  - `fn raise_error(&mut self, function_id: &str, error_name: &str) -> Result<(), String>`:
    `debug_assert!` the owning `BuiltinFunction.errors` (via
    `REGISTRY.function(function_id)`) contains `error_name`; `let (code, message)
    = errorcode::runtime_error(error_name).expect(..)`; record `error_name` in
    `self.used_errors`; `self.emit_error_code_return(code, message)`.
  - `fn raise_error_bare(&mut self, error_name: &str) -> Result<(), String>`:
    same, minus the contract check.
- `self.used_errors: BTreeSet<&'static str>` — a module-scoped accumulator on
  the builder (deterministic order). Nothing reads it in A; D consumes it.
- The `debug_assert` is a compiler-bug backstop (Tier-2, per the diagnostics
  split): a site raising an undeclared error is caught when the test suite
  exercises that path. It must not fire on the existing corpus — so any error a
  migrated site raises must be in the owning function’s `errors` **before** the
  site is migrated (B/C add those declarations alongside each migration).

## Compatibility / Format Impact

None in A. No public surface, IR/`.mfp` format, ABI, or codegen output changes.
The used-set is internal builder state.

## Phases

### Phase 0 — parity audit (falsify the premise first)

Prove the table can replace `ERR_*` with zero **runtime** behavior change; if any
message text differs, migrating would change the error a program observes (not
just its bytes), so it must be reconciled here first.

- [ ] Add a test `error_constants_match_table` (in `error_constants.rs` under
      `#[cfg(test)]`): for every `ERR_*_CODE` / paired `ERR_*_MESSAGE`, look up the
      table row **by code** via `errorcode::runtime_error` and compare. Assert the
      **code** always matches (a code mismatch is a bug). For the **message**, do
      NOT hard-fail on inequality — instead collect every `(code, ERR_* message,
      table message)` where they differ into an **expected-change list** the test
      prints/asserts against a checked-in snapshot. This is the authoritative set
      of sites whose runtime message legitimately changes on consolidation.
- [ ] Resolve the name-alias map by **code** for the 6 known aliases +
      `ERR_DIRECTORY_NOT_EMPTY`; record each `ERR_* → Err*` pair (and its code) in
      Corrections.
- [ ] Confirm the 3 table-only codes (`ErrPermissionDenied`,
      `ErrAuthenticationFailed`, `ErrWrapped`) have no emit-path obligation in A
      (`ErrWrapped` verified off-path; the other two: `grep` their codes in
      `src/target/shared/code/`).
- [ ] Tests: `error_constants_match_table` passes (all codes match; message
      divergences enumerated in the snapshot). The runtime-error suite lives at
      `tests/rt-error/**` — the home for the per-site tests B/C/D add.

Acceptance: `cargo test --bin mfb error_constants_match_table` passes, proving
every `ERR_*` code equals its table row and producing the expected-message-change
list — the audit + change-manifest that licenses B–D’s per-site gate.
Commit: —

### Phase 1 — dormant primitives + used-set

Add `raise_error` / `raise_error_bare` / `used_errors` with no caller migrated.

- [ ] Add `used_errors: BTreeSet<&'static str>` to the code builder struct;
      initialize empty; expose an accessor for D. Not read yet.
- [ ] Add `raise_error(function_id, error_name)` and `raise_error_bare(error_name)`
      to `builder_error_emission.rs` per §4, each delegating to
      `emit_error_code_return` so output is identical.
- [ ] Convert the existing `collections.get` poc in `builder_collection_query.rs`
      to call `self.raise_error("collections.get", "ErrIndexOutOfRange")` (it
      currently open-codes the same lookup) — the one live caller, proving the
      primitive end-to-end while staying byte-identical.
- [ ] Tests: a unit test that `raise_error` for a known `(func, name)` and
      `raise_error_bare(name)` each produce the same `CodeInstruction` sequence as
      the corresponding `emit_*_return` wrapper (assert on emitted ops, per the
      Windows-codegen-emit-inspection precedent), and that `used_errors` records
      the name.

Acceptance: `cargo test --bin mfb` green (incl. the emit-equivalence + used-set
unit test), and the out-of-range runtime program still raises
`ErrIndexOutOfRange` with the same message. Goldens should be unchanged **here**
only because A migrates nothing (the poc already emitted identically) — that is
an observation, not a constraint to defend; do not add scope to preserve it.
Commit: —

## Validation Plan

- Tests: `error_constants_match_table` (Phase 0); the `raise_error` emit-equivalence
  + used-set unit test (Phase 1). Run via `cargo test --bin mfb` (compiler tests
  live in the bin target, not `--lib`).
- Coverage check: confirm the new tests are in the bin suite’s denominator
  (they run under `cargo test --bin mfb`, not a filtered subset).
- Runtime proof: build a `.mfb` that indexes a list out of range; `mfb build` +
  run yields the same `ErrIndexOutOfRange` message as before A (the poc path).
- Codegen parity: `scripts/artifact-gate.sh` (execution-free, ~15-20 min; do NOT
  run concurrently with another gate — check `pgrep -f artifact-gate`) — zero
  `.ncode`/`.ncodesum` delta is the gate that A changed no output.
- Doc sync: none in A (the table already matches `02_error-codes.md`; Phase 0
  proves it).
- Acceptance: `cargo test --bin mfb` + one clean `artifact-gate.sh` at A’s close.

## Open Decisions

- **`raise_error_bare` vs a sentinel `func_id`.** A separate method (recommended)
  vs. `raise_error(None, name)`. Recommendation: separate method — the call sites
  read clearly and the contract-check branch stays out of the hot path. Either
  way the invariant is *exactly two* entry points. (§4)

> **Resolved (was an open decision):** the string-symbol scheme is unified to a
> single mechanism inside `raise_error`; there is no second "fragment" primitive
> and no attempt to preserve the old per-site symbol bytes. Byte churn from that
> unification re-baselines goldens (C, D). Keeping the old scheme to stay
> byte-identical was rejected because it would leave a third emission path alive,
> violating Definition-of-done invariant #1.
  Decision: separate method

## Corrections

<Filled in during execution — especially any Phase-0 message-text divergence and
the resolved `ERR_* → Err*` alias-by-code map.>

## Summary

A is the low-risk anchor: it proves `ERR_* ↔ table` parity (Phase 0 — the premise
the whole feature rests on) and lands the `raise_error`/`raise_error_bare`/used-set
machinery wrapping today’s emission unchanged. Nothing is migrated or deleted; no
codegen output moves. All engineering risk is deferred to B (172 per-call-site
migrations), C (49 symbol-path sites + `errors[]` declarations), and D (deletions +
gating removal). Untouched: `ErrWrapped`, the IR/`.mfp` format, and the string
data-object machinery itself.
