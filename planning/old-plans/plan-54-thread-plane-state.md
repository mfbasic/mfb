# plan-54: STATE on the thread plane type — close bug-257

Last updated: 2026-07-17
Effort: large (3h–1d)

Carries a resource's `STATE` on the thread transfer plane type, so
`thread::transfer` checks the resource's STATE against the plane at the call site,
`thread::accept` returns the plane's stateful type, and a stateful resource on a
bare plane is a **compile error** — not the current silent cross-thread type
confusion (bug-257).

The single outcome: **a `File STATE Cursor` transfers only on a `Thread OF RES File
STATE Cursor TO Out` plane, `accept` on that plane returns `RES File STATE Cursor`
(not bare), and putting a `File STATE Cursor` on a bare `Thread OF RES File TO Out`
plane is rejected with `TYPE_STATE_MISMATCH` at the `transfer` call.**

References:

- `bugs/bug-257-cross-thread-state-type-confusion.md` — the bug this closes, with
  the runtime repro (`Cursor{pos:99}` sent, read as `Label{name}` → bogus
  `Allocation failed`). **Read first.**
- `./mfb spec language resource-management` §15.5 — the escape rule this rests on:
  a `RES` is an alias; bare erases STATE only where the alias *cannot escape*; a
  transfer is a **move to a re-typer** (another thread that re-declares the type),
  so it is an escape position and must carry STATE in the contract.
- `planning/old-plans/plan-52-A..D`, `plan-53-A..C` — the STATE model + the
  kind-11 `.mfp` type this reuses.

## 1. Goal

- The plane grammar accepts `STATE T` on its `RES` element:
  `Thread OF RES File STATE Cursor TO Integer`, `ThreadWorker OF RES File STATE
  Cursor TO Integer`. `STATE` is optional; a bare plane is unchanged.
- `thread::transfer(t, f)` requires `f`'s STATE to equal the plane's element STATE;
  a stateful `f` on a **bare** plane is `TYPE_STATE_MISMATCH` (the laundering
  analog of plan-52-D's bare-binding rule, at the thread boundary).
- `thread::accept(t, …)` returns the plane's element type **with STATE**, so the
  receiver's `RES f AS File STATE Cursor = accept(…)` agrees structurally and
  cannot invent a different STATE.
- The plane STATE rides an exported worker signature (`thread::start` infers the
  parent `Thread` type from the worker's `ThreadWorker` param), so it works
  cross-package.

### Non-goals (explicit constraints)

- **The cross-arena STATE-pointer lifetime.** bug-257's *second* half: the transfer
  copies the STATE **pointer**, which still points into the **sender's** arena, so
  the receiver holds a cross-arena pointer whose lifetime is the sender thread's.
  Typing the plane fixes the type *confusion*; it does **not** fix the lifetime.
  That is the harder half and is scoped separately (§5) — this plan may deep-copy
  the STATE into the receiver's arena, or defer it, but must not pretend the
  pointer copy is safe.
- **Renaming "borrow" across the codebase.** `TYPE_RESOURCE_BORROW_INVALIDATE` and
  §15's prose keep their names; §15.5's STATE framing already uses "alias" (done
  2a3a46de). A full de-`borrow` of the spec is a separate wording pass.
- **The `STATE` string encoding.** Reuses the `" STATE "` type-string convention
  and the kind-11 `.mfp` type; no structured field.
- **A runtime type tag on the STATE payload.** Rejected in plan-52 (layout change).
  Agreement is statically decidable from the plane type; no runtime check.

## 2. Current State

- **The plane type is parsed as a string** by `builtins::thread::thread_parts_full`
  → `split_thread_types` (`src/builtins/thread.rs:297-344`), which delimits the
  `RES <res>` element with `type_prefix_len` (`:367`). `type_prefix_len` stops at
  the first non-`[alnum_:.]` char, so it returns just `File` from `File STATE
  Cursor` — the ` STATE Cursor` is not captured. **This is the parse gap.**
- **`Type::Thread(msg, Option<Box<Type>>, out)`** (`src/syntaxcheck/mod.rs:59`) —
  the resource element is a `Type`, and `parse_type` **strips** `STATE` (plan-52-D:
  `Type` has no STATE concept; STATE rides beside it in `LocalInfo`/`ParamSig`). So
  the plane's resource STATE is lost at the `Type` level. **24 `Type::Thread`
  references** across `inference.rs`, `mod.rs`, `types.rs`, `resources.rs`.
- **The resource-plane check** (`src/syntaxcheck/resources.rs:413-452`) uses
  `Type::Thread(_, resource, _)` for sendability only — it never consults STATE.
- **`thread::transfer`** is invalidation event #2 (a **move**, requires ownership;
  a borrowed resource can't be transferred — `TYPE_RESOURCE_BORROW_INVALIDATE`), so
  the transferred `f` is always an owned binding whose STATE is statically known.
- **The runtime already moves the STATE pointer** across the transfer
  (`builder_arena_transfer.rs:copy_resource_to_current_arena`, stores it at
  `FILE_OFFSET_STATE` of the receiver's record). So the payload *does* cross; the
  bug is that the types don't agree and the pointer is cross-arena.
- **`.mfp`:** the `Thread`/`ThreadWorker` type is kind 7/10, payload = message +
  output + optional resource **type ids**. If the resource element's id is a
  **kind-11** type (`File STATE Cursor`, added plan-53), the plane already
  round-trips the STATE structurally — **confirm** the plane encoder threads the
  resource element through `type_id` (which now emits kind 11).

## 3. Design Overview

Two workable representations; pick in Phase 1.

**(a) String-carried (less invasive).** Keep `Type::Thread`'s resource STATE-less;
carry the STATE only in the plane **type string** (`"Thread OF RES File STATE Cursor
TO Int"`). `transfer`/`accept` already resolve against the handle's type string via
`thread_parts_full` — extend that to capture STATE (Phase 1) and consult it at the
two enforcement points. Avoids touching the 24 `Type::Thread` sites. Risk: any path
that rebuilds the plane *string* from the `Type` (which is STATE-less) drops it —
audit those.

**(b) Type-carried (clean, invasive).** Add a resource-STATE to
`Type::Thread`/`ThreadWorker`. Correct and total, but touches all 24 sites + the
`.mfp` decode that rebuilds the `Type`. Mechanical (the compiler enumerates them).

Recommend **(a) first**, falling back to (b) only if a string path proves it drops
STATE where a `Type` round-trip is unavoidable. Either way the enforcement is the
same two checks below.

**Where the risk concentrates:** the `accept` return type must carry STATE (or the
receiver binds bare and the whole point is lost), and the `transfer` check must
reject stateful-on-bare (or the laundering stays open). Both are front-end, gated
behind the bug-257 runtime repro flipping from "runs with confusion" to "compile
error".

## 4. Phases

### Phase 1 — parse STATE on the plane's RES element — DONE

- [x] `src/builtins/thread.rs` — `resource_element_len` captures ` STATE <T>` as
      part of the `RES` element in `split_thread_types`/`thread_body_len`, so
      `thread_parts_full` returns `resource = Some("File STATE Cursor")`. The AST
      grammar parser (`ast/expr.rs::parse_thread_type_name`) gained the same via
      `parse_resource_plane_type` (reusing `parse_optional_state`).
- [x] Representation **(b)** chosen — (a) proved to drop STATE unavoidably: the
      param type round-trips through `Type::Thread`, which strips the resource
      STATE, and storing it in `Type::User("File STATE Cursor")` broke
      `is_resource_type` (exact-name registry lookup). Added a dedicated
      `resource_state: Option<Box<Type>>` field to `Type::Thread`/`ThreadWorker`
      (24 sites), so the resource element stays bare and STATE rides beside it.
      Also fixed `base_resource_name`/`state_type_name` to split only a *top-level*
      STATE (they truncated `ThreadWorker OF RES File STATE Cursor TO Int` to
      `ThreadWorker OF RES File`), and made `resolver` + `ir::verify` STATE-string
      sites composite-safe.
- [x] Tests: `thread_parts_full`/`resolve_call` unit tests for the STATE shapes
      (`plane_parses_state_on_resource_element`, `resolve_transfer_accept_stateful_plane`).

Acceptance: the plane type with STATE parses and round-trips (unit + the rt fixture
goldens); bare planes unchanged (bare `resource_element_len`/`copy_resource` paths
byte-identical).

### Phase 2 — accept returns the stateful type; transfer checks agreement — DONE

- [x] `thread::accept` on a `STATE T` plane returns `RES Res STATE T` (via
      `resolve_call`'s `thread_resource`, now STATE-carrying); on a bare plane
      returns bare. The receiver binding is validated by the existing
      `check_binding_state_agreement` (plan-52-D) — no new accept-side rule.
- [x] `thread::transfer(t, f)` emits `TYPE_STATE_MISMATCH` (ir::verify's
      `check_thread_transfer_state`, the sole rejecter) when `f`'s STATE differs
      from the plane's element STATE — a stateful `f` on a bare plane, a bare `f`
      on a stateful plane, or two different states. The front-end `resolve_call`
      matches on the *base* resource type so the precise diagnostic fires in verify.
- [x] Fixtures: `tests/syntax/threads/thread-transfer-state-mismatch` (bug-257's
      repro → `TYPE_STATE_MISMATCH` compile error); `tests/rt-behavior/threads/
      thread-transfer-state-rt` updated so the STATE rides the plane and is *checked*
      (runs, worker reads `pos = 99`).

Acceptance: the disagreeing-STATE transfer is a compile error; the agreeing one runs
and the worker reads the sent state; `thread-transfer-state-rt` passes with the plane
STATE.

### Phase 3 — cross-package + .mfp + spec — DONE

- [x] The plane STATE rides the exported worker signature: the `.mfp` encoder
      threads the resource element through `type_id`, which emits the kind-11
      composite for `File STATE Cursor`, and the decoder reassembles it via
      `format_thread_type` — **no binary_repr change needed** (Phase 1's parser
      makes it round-trip). `thread::start` infers the parent `Thread OF RES File
      STATE Cursor` from the worker. Cross-package proven: `thread-transfer-state-rt`
      is a `state_xfer_workers` package + importer that transfers/accepts (worker
      reads `99`).
- [x] `./mfb spec language threads` (§16) documents the plane STATE + the
      transfer/accept agreement + the deep-copy; §15.5 drops the "(once enforced)"
      qualifier; `thread::transfer`/`accept` man pages note the STATE rule.
- [x] `bugs/bug-257` → Fixed (both the type confusion and the cross-arena lifetime,
      §5).

Acceptance: cross-package transfer of a stateful resource type-checks and reads the
state; specs current; artifact gate delta only the intended fixtures.

## 5. The cross-arena lifetime (the harder half) — DONE (folded into Phase 2)

Implemented: `copy_resource_to_current_arena` now takes the resource `type_` and,
when it carries a `STATE`, **deep-copies** the STATE record via
`copy_value_to_current_arena` into the current arena (the receiver's — the transfer
switches the arena to the destination, and accept runs in the receiver's own),
storing the fresh pointer at `FILE_OFFSET_STATE` instead of aliasing the sender's.
The source keeps its own STATE (freed normally), so there is no shared pointer and no
cross-thread lifetime coupling. A null STATE slot stays null (lazy init on accept).
Bare resources keep the verbatim word copy (byte-identical). Proven live:
`thread-transfer-state-rt` reads `99`, with the deep-copy path emitted on both the
`transferResource` and `acceptResource` sides.

Original analysis retained below.



Typing the plane closes the **confusion**. It does **not** close bug-257's second
finding: `copy_resource_to_current_arena` copies the STATE **pointer**, which points
into the **sender's** arena. After transfer the receiver's record's `FILE_OFFSET_STATE`
aliases sender-thread memory, freed at the sender's arena teardown. Options:

- **Deep-copy the STATE into the receiver's arena** at transfer (like the message
  plane deep-copies its payload — `copy_value_to_current_arena`). Correct; the STATE
  is a flat record (or inlines its Strings), so a sized block copy suffices. Cost: a
  copy per transfer. **Recommended.**
- Leave the pointer and pin the sender's arena until the receiver drops — cross-thread
  lifetime coupling, rejected (that is the hazard the moved bit was meant to avoid).

Recommend folding the deep-copy into Phase 2 (it is where the transfer already
copies the record), so the lifetime and the confusion are closed together rather
than leaving a second known hole.

## Validation Plan

- Tests: the plane-parse fixtures; bug-257's repro as a compile error; the agreeing
  runtime transfer (worker reads the sent state); the cross-package transfer; the
  bare-plane-rejects-stateful case.
- Runtime proof: **required** — only running proves the worker reads the *sent*
  state (not a re-defaulted one), and that a stateful transfer no longer confuses.
- Doc sync: `./mfb spec language threads`, `resource-management` §15.5.
- Acceptance: `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, `cargo test --bin mfb`.

## Open Decisions

- **Representation (a) string vs (b) Type.** Recommend (a) first (avoids 24 sites);
  fall back to (b) if a `Type` round-trip drops STATE somewhere unavoidable.
- **Fold the cross-arena deep-copy into this plan or defer?** Recommend **fold in**
  (Phase 2 / §5) — leaving the pointer cross-arena is a live lifetime bug, and the
  transfer copy is already right there.

## Summary

The type-level fix is the escape rule applied to the thread boundary: a transfer is
a move to a re-typer, so the plane must name the STATE, `accept` returns it, and a
stateful resource can't ride a bare plane. The invasive part is representation (the
`Type` enum strips STATE, so either thread it through the plane string or extend
`Type::Thread` across 24 sites). The subtle part is not the confusion but the
**cross-arena STATE pointer** (§5) — typing the plane without deep-copying the STATE
would fix bug-257's first half and leave its second half live, so the deep-copy
belongs in the same change.
