# plan-114-A: A dedicated compile-time error for a resource on the thread data plane

Last updated: 2026-08-30
Overall Effort: x-large (1d–3d) — the whole `plan-114` feature (letters A–E)
Effort: medium (1h–2h)
Depends on: nothing

Today a resource that reaches the thread **data** plane is refused by the generic
`TYPE_THREAD_NOT_SENDABLE` (`2-203-0063`, "thread boundary type is not sendable"),
which says nothing about *why* and nothing about the remedy. This letter gives
that case its own rule code and a message that names the offending resource and
points at the resource plane (`thread::transfer` / `thread::accept`).

The source-level parse gap that made this diagnostic unreachable for the
`List OF RES pkg::Type` spelling is **bug-463**, a prerequisite of this letter
rather than scope within it.

Behavioral outcome: a program that puts a resource — bare, `RES`-marked in a
collection element/map value, or (after letter D) a `RES` record field — on any
thread data plane fails with `2-203-0138 TYPE_THREAD_RESOURCE_PLANE_REQUIRED`
naming the resource and the plane, and `TYPE_THREAD_NOT_SENDABLE` keeps firing
unchanged for every non-resource unsendable type (`Func`, `ThreadHandle`).

References:

- `src/docs/spec/language/15_resource-management.md` §15.6 — "Sharing a resource
  collection across threads remains out of scope."
- `src/docs/spec/language/16_threads.md` — the data plane / `RES` plane split.
- `src/docs/spec/diagnostics/01_rule-codes.md` — the rule-code table.
- `.ai/resources-packages.md` — "Thread resource plane split", "Thread transfer
  move-flag is success-gated".
- `.ai/testing-gates.md` — the gate is blind to diagnostic prose; `test-accept.sh`
  is the only thing that catches a message change.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Working tree clean; release `mfb` built | `git status --porcelain` → empty; `ls -la target/release/mfb` | MET (2026-08-31, worktree `P-114`) |
| No other artifact-gate / test-accept running | `pgrep -f '[a]rtifact-gate\|[t]est-accept'` → no output | MET (2026-08-31) |
| A free `2-203-NNNN` code exists for the new rule | `grep -c '"2-203-0138"' src/rules/table.rs` → `0` | MET (2026-08-31) — see Corrections C1: `2-203-0137` was taken by bug-457, so this letter allocates **`2-203-0138`** |
| bug-463 fixed (a `RES` collection on a thread message plane parses) | `ls bugs/completed/bug-463-*` → one match | MET (2026-08-31) — `bugs/completed/bug-463-thread-plane-res-collection-parse.md` |

If bug-463 is not fixed, this letter cannot start, full stop: without it
`Thread OF List OF RES fs::File TO Integer` never reaches the sendability check,
so the new rule is unreachable from MFBASIC source and cannot be fixture-tested.
Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- A resource reaching a thread **data** plane (message type, output type, or a
  `thread::send`/`receive`/`start`/`waitFor` call's plane argument) emits
  `2-203-0138 TYPE_THREAD_RESOURCE_PLANE_REQUIRED` with a message naming the
  resource type, the plane it was found on, and the resource-plane remedy.
- The rule fires for all three shapes: a bare resource nominal (`Thread OF fs::File TO Integer`),
  a `RES`-marked collection element/map value (`List OF RES fs::File`), and a
  resource nested inside a record field (already reachable today via a `STATE`
  payload record holding `List OF RES fs.File` — `src/ir/verify/tests.rs:7975`).
- `TYPE_THREAD_NOT_SENDABLE` still fires, unchanged, for `Func` and `ThreadHandle`
  planes.

### Non-goals (explicit constraints)

- **No change to what is accepted or rejected.** This letter only re-labels an
  existing rejection. The accept/reject set is unchanged.
- **No type-grammar change.** `type_prefix_len` and `ParameterType::parse` are
  bug-463's scope; this letter must not touch them.
- **No change to the resource plane.** `thread::transfer` / `thread::accept`
  semantics, the two direction-isolated queues, and `THREAD_BLOCK_SIZE` are
  untouched.
- **No codegen change.** `artifact-gate.sh <exe> all` must report `diffs=0`.
- `2-203-0063 TYPE_THREAD_NOT_SENDABLE` is **not** retired, renumbered, or
  reworded.
- Records may still not hold resources after this letter (that is letter D).

## 2. Current State

`ir::verify` owns thread-plane checking. `require_thread_sendable`
(`src/ir/verify/resources.rs:486`) is the single emitter — it calls
`is_thread_sendable` (`:416`) and, on false, emits `TYPE_THREAD_NOT_SENDABLE` with
`"{context} requires a thread-sendable type, got `{type}`."`.

`is_thread_sendable` returns false for exactly four causes, read at `:416-485`:

1. `ParameterType::Res(_) => false` (`:439`) — the `RES`-marked element/map value.
   The comment already cites §15.6.
2. `ParameterType::Func(..) | ParameterType::ThreadHandle { .. } => false` (`:440`).
3. A bare resource nominal: `close_op_for(other).is_some()` → `is_resource_sendable(other)`
   (`:450`), false unless the resource type is registered thread-sendable.
4. Structural descent — `ListOf`/`SetOf`/`MapOf`/`ResultOf` (`:431-437`), union
   variants (`:456`), and record fields via `record_fields_sendable` (`:474`),
   any of which can bottom out in (1) or (3).

Causes (1) and (3) are the resource causes; (2) is not. The descent in (4) is what
already makes `rejects_unsendable_resource_plane_state_payload`
(`src/ir/verify/tests.rs:7975`, bug-301 G4) work: a record whose field is
`List OF RES fs.File` is refused as a resource-plane `STATE` payload.

`require_thread_sendable` is called from **11 sites, all in one file**. Six of
them are data planes, three are the resource plane, and two are the resource
plane's `STATE` payload (which is data) — see **Corrections C3** for the
site-by-site table. The emitter therefore takes an explicit `Plane` argument
rather than re-labelling unconditionally.

Two further data-plane rejections do **not** flow through `require_thread_sendable`
at all: the hand-written bare-resource blocks at `:530-540` (declared message
plane) and `:629-639` (`thread::send`). They are what actually reject
`Thread OF fs::File TO Integer`, because `fs::File` is registered thread-sendable
so the cause walk returns "sendable" for it — see **Corrections C4**.

### Measured populations

| What | Count | Command |
|---|---|---|
| `require_thread_sendable` call sites (all in `src/ir/verify/resources.rs`) | 11 | `grep -rn "require_thread_sendable(" src/ --include='*.rs' \| grep -v "fn require" \| wc -l` → `11` |
| Existing `TYPE_THREAD_NOT_SENDABLE` assertions in unit tests | 9 | `grep -rn "TYPE_THREAD_NOT_SENDABLE" src/ir/verify/tests.rs \| wc -l` → `9` |
| Golden `build.log` files naming `TYPE_THREAD_NOT_SENDABLE` | 2 | `grep -rln "TYPE_THREAD_NOT_SENDABLE" tests/` → `tests/syntax/threads/func_thread_send_invalid/golden/build.log`, `tests/syntax/threads/func_thread_start_invalid/golden/build.log` |
| `tests/syntax/threads` fixtures | 31 | `ls tests/syntax/threads \| wc -l` → `31` |
| Highest allocated `2-203-NNNN` rule code | `2-203-0137` | `grep -oE '"2-203-[0-9]{4}"' src/rules/table.rs \| sort -u \| tail -1` → `"2-203-0137"` (re-measured 2026-08-31; was `0136` when authored — see Corrections C1) |

### Verified properties

- **~~A single emitter reaches every plane.~~ FALSE — corrected in C3/C4.** All 11
  call sites (`:528`, `:529`, `:544`, `:548`, `:598`, `:608`, `:610`, `:615`,
  `:620`, `:643`, `:657`) do pass a `context` string and the type and none emits a
  rule itself — but they do not all name a *data* plane (three name the resource
  plane, C3), and two separate bespoke blocks emit the bare-resource data-plane
  rejection without going through the emitter at all (C4). Splitting the emitter by
  cause therefore **does** need call-site edits: a `Plane` argument at all 11, and
  a rule swap in the two bespoke blocks.
- **The record-field descent already exists and is tested.** Read
  `record_fields_sendable` (`:474-482`) — it recurses through `is_thread_sendable`
  on every field type — and `rejects_unsendable_resource_plane_state_payload`
  (`src/ir/verify/tests.rs:7975-8007`), which asserts a `Holder` record with a
  `List OF RES fs.File` field is refused on a `STATE` plane while a plain
  `Integer`/`String` `Holder` is accepted. This is why letter D needs no new
  thread check: a `RES` record field lands on the same descent.
- **The source-level parse gap is a separate, fully characterized bug.** Root-caused
  to `type_prefix_len` (`src/types.rs:1095`) not consuming the `RES ` marker in a
  `List`/`Map` element position, so the thread body split fails and the whole
  spelling survives as an opaque `Named`. Written up with a measured contrast matrix
  as **bug-463**; it is a prerequisite here, not scope. Nothing in this letter
  depends on *how* it is fixed, only that it is.

## 3. Design Overview

Split `require_thread_sendable` into a *check* and a *cause*. `is_thread_sendable`
already knows why it said no; it just throws the reason away. Add a sibling
`thread_unsendable_cause(type_) -> Option<Unsendable>` that walks the same
structure and returns the first blocking leaf as either `Unsendable::Resource(ParameterType)`
or `Unsendable::Other(ParameterType)`. `require_thread_sendable` then emits
`TYPE_THREAD_RESOURCE_PLANE_REQUIRED` for the first and `TYPE_THREAD_NOT_SENDABLE`
for the second.

Writing a second walk that can disagree with the first is the one real risk here:
a leaf that `is_thread_sendable` rejects but `thread_unsendable_cause` reports as
`None` would silently drop a diagnostic. **Do not write two walks.** Make
`thread_unsendable_cause` the single implementation and define
`is_thread_sendable(t) == thread_unsendable_cause(t).is_none()`, so the two can
never diverge. The nine existing `TYPE_THREAD_NOT_SENDABLE` unit assertions are the
regression net for that refactor.

The parse gap is bug-463's problem, not this letter's. Do not absorb any part of
it here — if a spelling still fails to parse mid-flight, that is a bug-463
regression to reopen, not scope to take on.

**Byte-identity IS the right gate for this letter.** Nothing here reaches codegen
— it is a diagnostics change plus a front-end parse fix, and no currently-valid
program changes shape. `scripts/artifact-gate.sh target/release/mfb all` must
report `diffs=0`; a diff is a bug introduced by the parse fix, to be localized by
objdump'ing one fixture, not a reason to stop.

Rejected alternatives:

- *Reword `TYPE_THREAD_NOT_SENDABLE` in place instead of adding a code.* Rejected:
  the message would have to cover `Func` and `ThreadHandle` too, so it could not
  name the resource-plane remedy, which is the whole point.
- *Emit both rules.* Rejected: two errors for one cause, and the golden
  `build.log`s become order-dependent.

## 4. Detailed Design

### 4.1 The new rule

Append to `src/rules/table.rs` (after `2-203-0137`):

```rust
Rule {
    code: "2-203-0138",
    name: "TYPE_THREAD_RESOURCE_PLANE_REQUIRED",
    severity: Severity::Error,
    message: "a resource cannot cross the thread data plane",
},
```

Emitted message shape (the `{context}` strings already supplied by the 11 call
sites are reused verbatim):

```
{context} carries resource `{resource}`; a resource crosses a thread only on the
RES plane. Declare `RES {resource}` on the thread and move it with
thread::transfer / thread::accept.
```

### 4.2 The cause walk

In `src/ir/verify/resources.rs`:

```rust
pub(super) enum Unsendable {
    /// A resource reached a data plane: `Res(_)`, or a bare resource nominal
    /// whose type is not registered thread-sendable.
    Resource(ParameterType),
    /// `Func` / `ThreadHandle` — genuinely unsendable, not a plane mix-up.
    Other(ParameterType),
}
```

`thread_unsendable_cause` mirrors `is_thread_sendable`'s existing match arm for
arm, returning `Some(Unsendable::Resource(t))` at `:439` (`Res(_)`) and at `:450`
(`close_op_for(..).is_some()` and `!is_resource_sendable(..)`), and
`Some(Unsendable::Other(t))` at `:440`. Structural arms return the **first**
`Some` from their children, left to right, so the reported leaf is deterministic.
`is_thread_sendable` becomes a one-line wrapper.

Per **Corrections C3**, `require_thread_sendable` additionally takes a
`Plane { Data, Resource }`. `Unsendable::Resource` maps to `2-203-0138` only on
`Plane::Data`; on `Plane::Resource` — and for `Unsendable::Other` on either — it
keeps `2-203-0063`. The cause walk itself is plane-independent; only the emitter
reads the plane.

### 4.3 What this letter does NOT touch

`type_prefix_len` / `ParameterType::parse` (bug-463) and
`resolve_package_qualified_name`. The one type grammar is out of bounds here; this
letter changes only which rule `require_thread_sendable` emits.

## Compatibility / Format Impact

- **New diagnostic code `2-203-0138`.** Additive; `2-203-0063` keeps its code,
  name, severity, and message.
- **Golden `build.log` churn:** the two fixtures listed in §2 change *only* if
  their unsendable cause is a resource. Check before regenerating — read each
  fixture's source; a `Func`-plane fixture must NOT churn, and if it does, the
  cause walk disagrees with `is_thread_sendable` and that is the bug.
- No `.mfp`, `.ir`, `.nir`, `.ncode` or ABI change.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work it describes; `- [~]` for partial with one line on what
> remains; `- [x] ~~text~~ — moot: <evidence>` for a dropped task. An unticked box
> means NOT DONE.

### Phase 1 — Confirm the source-level path is open

bug-463 (a prerequisite, not scope here) makes `Thread OF List OF RES fs::File TO Integer`
parse and left behind `tests/syntax/threads/thread-res-collection-plane-invalid/`
with a `TYPE_THREAD_NOT_SENDABLE` golden. This phase confirms that starting point
and nothing more.

- [x] Re-run bug-463's contrast matrix (its "Failing Reproduction" section) against
      the current binary and confirm every row still matches its column. If any row
      regressed, that is a bug-463 regression — reopen it, do not work around it here.
      **All 22 rows match** — measured 2026-08-31 with `target/release/mfb build -ast -ir`
      over a one-parameter scratch project per row; full table in Corrections C2.
- [x] Confirm `tests/syntax/threads/thread-res-collection-plane-invalid/golden/build.log`
      shows `2-203-0063 TYPE_THREAD_NOT_SENDABLE`. Phase 3 re-goldens it to `2-203-0138`.
      Confirmed: all three of its `FUNC`s (`viaList`, `viaMap`, `viaWorker`) report
      `2-203-0063`.

Acceptance: every contrast-matrix row matches, and the fixture golden reads
`TYPE_THREAD_NOT_SENDABLE`. **MET** (see Corrections C2).
Commit: 78e83a1e8

### Phase 2 — Cause walk (no behavior change)

Refactor only: one walk, two entry points. The nine existing assertions prove it.

- [x] Add `Unsendable` and `thread_unsendable_cause` in `src/ir/verify/resources.rs`,
      mirroring `is_thread_sendable`'s arms per §4.2. Added `Plane` alongside them
      (Corrections C3), and `Unsendable::leaf()`.
- [x] ~~Reduce `is_thread_sendable` to `thread_unsendable_cause(t).is_none()`.~~
      **Deleted outright instead** — once `require_thread_sendable` reads the cause,
      nothing called the boolean, and `cargo check --all-targets` reported
      `method is_thread_sendable is never used`. A `pub(super)` wrapper kept only
      for a later letter is the dead code AGENTS.md forbids. Its doc comment (the
      per-arm contract) was moved onto `thread_unsendable_cause`, which is now the
      single walk — the anti-divergence intent of the task is met more strongly by
      there being exactly one entry point, not two.
- [x] Tests: in `src/ir/verify/tests.rs`, add a case per cause asserting the
      returned `Unsendable` variant and leaf type — `Res(fs.File)` element,
      bare `fs.File` nominal, `Func`, `ThreadHandle`, and the nested
      record-field case from `rejects_unsendable_resource_plane_state_payload`.
      Eight `cause_walk_*` tests, plus two added beyond the plan: first-blocking-field
      determinism (the `all(..)`→`find_map` change makes order observable) and
      cycle-guard termination.
- [x] Added task: pin that the **resource plane** keeps `2-203-0063`
      (`the_resource_plane_keeps_the_generic_unsendable_rule`) and that a `Func`
      or thread-handle plane never gets the resource remedy
      (`func_and_thread_handle_planes_keep_the_generic_unsendable_rule`). These are
      the RED guard for Corrections C3 — without them the plane split is untested.

Acceptance: all 9 pre-existing `TYPE_THREAD_NOT_SENDABLE` assertions in
`src/ir/verify/tests.rs` still pass **unmodified**, and the new cause tests pass.
`cargo check --all-targets` clean. **MET, with the scope note in Corrections C5**:
7 of the 9 pass unmodified; the 2 that changed are precisely the two resource-cause
assertions §1 requires to move to the new rule, and Phase 3 owns that flip. The
refactor alone was proven neutral before the rule was wired.
`cargo check --all-targets` clean (one dead-code warning, resolved by the deletion
above).
Commit: d8e600599 (Phases 2 and 3 landed as one commit — Corrections C5)

### Phase 3 — Emit the dedicated rule

- [x] Add `2-203-0138 TYPE_THREAD_RESOURCE_PLANE_REQUIRED` to `src/rules/table.rs` per §4.1.
- [x] Split `require_thread_sendable` to emit by cause (§4.1 message shape), gated
      on `Plane::Data` per Corrections C3, and convert the two bespoke bare-resource
      blocks per Corrections C4.
- [x] Add the row to `src/docs/spec/diagnostics/01_rule-codes.md` (the table around
      `:446`, in code order) and cross-reference it from
      `src/docs/spec/language/15_resource-management.md` §15.6 where "Sharing a
      resource collection across threads is out of scope" is stated, and from
      `src/docs/spec/language/16_threads.md` at the data-plane/`RES`-plane split.
      §16 also gained the sentence that the resource plane keeps `2-203-0063` (C3).
- [x] Update the Phase 1 fixture golden to `2-203-0138`
      (`scripts/sync-goldens.sh target/release/mfb 'thread-res-collection-plane-invalid'`
      reported `synced 1 golden file(s) across 1 test(s)`). All three of its `FUNC`s moved.
- [x] Check the two existing golden `build.log`s (§2) and regenerate **only** those
      whose cause is a resource. If a `Func`/`ThreadHandle` fixture churns, stop
      and fix the cause walk. **Neither churned** — both `func_thread_send_invalid`
      and `func_thread_start_invalid` reject `BadMessage`, a record whose field is
      `Thread OF String TO Integer`, an `Other` cause. Measured on each:
      `mfb build -ast -ir <fixture> | grep -c TYPE_THREAD_RESOURCE_PLANE_REQUIRED`
      gave `0`, and `grep -c 2-203-0063` gave `2`. The canary held.
- [x] Tests: flip the resource-cause assertions in `src/ir/verify/tests.rs` to the
      new rule name; keep `Func`/`ThreadHandle` on `TYPE_THREAD_NOT_SENDABLE`.
      Exactly two flipped — `rejects_a_resource_in_the_message_plane` and
      `rejects_unsendable_resource_plane_state_payload` — which are §1's shapes 1
      and 3. The first also now asserts the generic rule does **not** also fire,
      pinning the C4 mutual-exclusion contract.

Acceptance: a source program putting a resource on any of the three plane shapes
(bare nominal, `RES` collection element, resource nested in a record field) fails
with `2-203-0138` naming the resource; a `Func`-planed program still fails with
`2-203-0063`. `mfb man`/spec rule table lists `2-203-0138`. **MET** — all three
shapes verified: bare nominal and record-field by the two flipped unit tests, the
`RES` collection element by the re-goldened source fixture. The `Func` plane is
verified by the two unchanged canary goldens plus
`func_and_thread_handle_planes_keep_the_generic_unsendable_rule`, and
`mfb spec diagnostics rule-codes | grep -c 2-203-0138` gave `1`.
Commit: d8e600599

## Validation Plan

- Tests: `src/ir/verify/tests.rs` (cause-walk unit tests + the re-labelled rule
  assertions); `tests/syntax/threads/thread-res-collection-plane-invalid/` — the
  source-level negative fixture bug-463 created, re-goldened here to `2-203-0138`.
- Coverage check: `src/ir/verify/resources.rs` is reached by `cargo test --bin mfb`;
  confirm the new `thread_unsendable_cause` arms are covered with
  `scripts/coverage.sh` per `.ai/build-tooling.md` (measure with `--bin mfb`).
- Runtime proof: none needed — this letter rejects programs, it does not run them.
  The proof is the fixture `build.log` diff.
- Doc sync: `src/docs/spec/diagnostics/01_rule-codes.md`,
  `src/docs/spec/language/15_resource-management.md` §15.6,
  `src/docs/spec/language/16_threads.md`.
- Acceptance:
  - `cargo test --no-fail-fast` (redirect to a file and check cargo's own status —
    a piped `| tail` reports tail's exit code, per `.ai/testing-gates.md`)
  - `cargo check --all-targets`
  - `scripts/test-accept.sh target/release/mfb /tmp/plan114a-scratch` (full — this
    is a diagnostics change and the artifact gate is blind to prose). Never pass a
    real directory as the second argument; it is `rm -rf`'d.
  - `scripts/artifact-gate.sh target/release/mfb all` → `diffs=0`
  - `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`

## Open Decisions

- **Rule name.** `TYPE_THREAD_RESOURCE_PLANE_REQUIRED` (recommended — says the
  remedy) vs. `TYPE_THREAD_RESOURCE_ON_DATA_PLANE` (says the fault). Prefer the
  remedy; the message already states the fault.
- **Does the bare-resource case belong here?** A bare `Thread OF fs::File TO Integer`
  where `fs::File` *is* registered thread-sendable is accepted today by
  `is_resource_sendable` (`:450`). Recommendation: leave that acceptance exactly as
  is; `2-203-0138` fires only where `is_thread_sendable` already said no. Changing
  which programs are accepted is out of scope for this letter (§1 non-goals).

## Corrections

**C1 — the new rule is `2-203-0138`, not `2-203-0137` (rule-code drift).**
The plan was authored on 2026-08-30 against a table whose highest allocated
`2-203` code was `2-203-0136`. Between authoring and execution, bug-457 allocated
`2-203-0137` to `TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`:

```
$ grep -oE '"2-203-[0-9]{4}"' src/rules/table.rs | sort -u | tail -1
"2-203-0137"
$ git log --oneline -1 -S'TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL' -- src/rules/table.rs
5d02f6931 bug-457: an inline TRAP now covers fallible calls NESTED in its expression
$ grep -c '"2-203-0138"' src/rules/table.rs
0
```

This is a drifted number, not a blocked design — nothing about the cause walk or
the diagnostic depends on which code is free. `TYPE_THREAD_RESOURCE_PLANE_REQUIRED`
is therefore allocated **`2-203-0138`**, and the prerequisite row is restated as
"a free `2-203-NNNN` code exists" so it cannot rot the same way again. Letters D
and E, which cross-reference the code, were updated in the same commit.

**C2 — bug-463's contrast matrix re-measured, all 22 rows match (Phase 1).**
Measured 2026-08-31 against `target/release/mfb` built at `213803f96`, one
`FUNC f(x AS <T>)` parameter per row in an otherwise identical scratch project,
`mfb build -ast -ir`. (A first attempt reported `SYMBOL_DUPLICATE_TOP_LEVEL` on
*every* row: the harness declared a `TYPE Pair`, which collides with a builtin
prelude type, and the prelude diagnostic masked each row's real answer. The
harness was fixed to anchor on diagnostics located in the fixture's own
`src/main.mfb` and to record the exit code — the failure mode
`diagnostic-harness-must-record-exit-and-unlocated-errors` warns about.)

| Type spelling | bug-463 "After" | Measured 2026-08-31 |
|---|---|---|
| `List OF RES fs::File` | works | accepted ✓ |
| `Map OF String TO RES fs::File` | works | accepted ✓ |
| `Map OF String TO List OF RES fs::File` | works | accepted ✓ |
| `List OF List OF RES fs::File` | works | accepted ✓ |
| `Thread OF Integer TO Integer` | works | accepted ✓ |
| `Thread OF List OF Integer TO Integer` | works | accepted ✓ |
| `Thread OF Integer RES fs::File TO Integer` | works | accepted ✓ |
| `Thread OF Integer TO List OF RES fs::File` | `TYPE_THREAD_NOT_SENDABLE` | `2-203-0063` ✓ |
| `Thread OF List OF RES fs::File TO Integer` | `TYPE_THREAD_NOT_SENDABLE` | `2-203-0063` ✓ |
| `Thread OF Map OF String TO RES fs::File TO Integer` | `TYPE_THREAD_NOT_SENDABLE` | `2-203-0063` ✓ |
| `ThreadWorker OF List OF RES fs::File TO Integer` | `TYPE_THREAD_NOT_SENDABLE` | `2-203-0063` ✓ |
| `Thread OF List OF RES fs::File STATE Cursor TO Integer` | `TYPE_THREAD_NOT_SENDABLE` | `2-203-0063` ✓ |
| `Thread OF List OF RES fs::File RES fs::File TO Integer` | `TYPE_THREAD_NOT_SENDABLE` | `2-203-0063` ✓ |
| `Map OF Thread OF List OF RES fs::File TO Integer TO Integer` | `TYPE_COLLECTION_OWNERSHIP_VIOLATION` | `2-203-0056` ✓ |
| `Thread OF Set OF Integer TO Integer` | accepted | accepted ✓ |
| `Thread OF List OF Set OF Integer TO Integer` | accepted | accepted ✓ |
| `Thread OF Map OF Set OF Integer TO Integer TO Integer` | accepted | accepted ✓ |
| `Thread OF <user generic> OF Integer, String TO Integer` | accepted | accepted ✓ |
| `Thread OF FUNC(Integer) AS String TO Integer` | `TYPE_THREAD_NOT_SENDABLE` | `2-203-0063` ✓ |
| `Thread OF Map OF RES fs::File TO Integer TO Integer` (Map **key**) | `MFB_PARSE_INVALID_IDENTIFIER` | `1-102-0003` ✓ |
| `Thread OF MapEntry OF String TO Integer TO Integer` | unchanged (accepted) | accepted ✓ |
| `Thread OF Result OF Integer TO Integer` | `TYPE_RESULT_NOT_USER_VISIBLE` | `2-203-0070` ✓ |

No bug-463 regression. The scratch harness was deleted after measuring —
bug-463's own regression tests (`src/types.rs::type_prefix_len_measures_every_parse_constructor`
and siblings) are the standing guard.

**C3 — §2's "a single change to the emitter reaches every plane" is FALSE: 5 of
the 11 call sites are the RESOURCE plane, not a data plane.**
The plan assumed every `require_thread_sendable` call site is a data plane, so
that re-labelling the emitter by cause would be safe everywhere. Reading all 11
sites shows three of them pass the resource plane's own type:

| Site | Context string | Plane |
|---|---|---|
| `resources.rs:528` | `Thread message type` | data |
| `resources.rs:529` | `Thread output type` | data |
| `resources.rs:544` | `Thread resource type` | **resource** |
| `resources.rs:548` | `Thread resource STATE type` | data (the payload is deep-copied data riding the RES plane) |
| `resources.rs:598` | ``Call to `X` input`` | data |
| `resources.rs:608` | ``Call to `X` message type`` | data |
| `resources.rs:610` | ``Call to `X` resource type`` | **resource** |
| `resources.rs:615` | ``Call to `X` output type`` | data |
| `resources.rs:620` | ``Call to `X` message type`` (`thread.send`) | data |
| `resources.rs:643` | ``Call to `X` resource STATE type`` | data (payload, as `:548`) |
| `resources.rs:657` | ``Call to `X` resource type`` | **resource** |

On the resource plane the `Resource` cause means something entirely different —
"this resource type is not registered thread-sendable" — and §4.1's message
("declare `RES {resource}` on the thread and move it with `thread::transfer`")
would tell the user to put the resource on the plane it is already on. So
`require_thread_sendable` takes an explicit `Plane` argument: it emits
`2-203-0138` only for a `Resource` cause on a **data** plane, and keeps
`2-203-0063 TYPE_THREAD_NOT_SENDABLE` for every cause on the resource plane and
for the `Other` cause everywhere. The two `STATE`-payload sites are classified
**data**, which is what makes the plan's third required shape (a resource nested
in a record field, `rejects_unsendable_resource_plane_state_payload`) emit the
new rule as §1 requires.

**C4 — the bare-resource-on-a-data-plane case does NOT flow through the cause
walk; it has two bespoke emitters that §2 never mentions.**
§1 requires `Thread OF fs::File TO Integer` to emit the new rule, and §2 claims
cause (3) — "a bare resource nominal … false unless the resource type is
registered thread-sendable" — delivers it. Measured: `fs::File` **is** registered
thread-sendable (`src/codegen/builtins/fs/mod.rs:165`, `sendable: true`; also
`tcp`, `tls`, `udp`), so `is_thread_sendable(fs.File)` returns `true` and
`require_thread_sendable` emits nothing for it. The rejection actually comes from
two hand-written blocks that the plan's §2 walk-through omits:

- `resources.rs:530-540` — the declared `Thread OF <resource> TO …` message plane;
- `resources.rs:629-639` — `thread::send` whose message type is a resource.

Both already state exactly the `2-203-0138` remedy in prose ("the data channel is
resource-free — declare it on the resource plane" / "use `thread::transfer`"), so
both are converted to emit `2-203-0138`. To honour §3's rejected alternative
"*Emit both rules* — two errors for one cause", each bespoke block and its
neighbouring `require_thread_sendable` are made **mutually exclusive**: when the
plane type is itself a bare resource the bespoke plane rule fires alone. For a
*non-sendable* bare resource on a message plane this reduces two diagnostics to
one; the set of rejected programs is unchanged, which is §1's non-goal.

**C5 — Phases 2 and 3 landed as one commit, and `is_thread_sendable` was deleted
rather than reduced to a wrapper.**
Two deviations from the phase plan, both forced by what the refactor turned out
to be:

1. *The phases could not be separated by a commit boundary.* Phase 2 was written
   as "refactor only, no behavior change", with Phase 3 flipping the rule. But
   `require_thread_sendable` is the **only** caller of the walk, so the moment it
   reads a cause instead of a boolean it must decide which rule to emit — there
   is no intermediate state that both compiles and changes nothing. The
   neutrality Phase 2 exists to prove was still obtained, in this order: the walk
   was written and the eight `cause_walk_*` tests were made green *before* the
   rule was wired, and the full `ir::verify` suite (443 tests) was run at that
   point. It reported exactly two failures, and they were the two assertions §1
   names as required to move. Nothing else in the suite noticed the refactor,
   which is the evidence Phase 2's acceptance was asking for.

2. *`is_thread_sendable` is deleted, not kept as a one-line wrapper.* With
   `require_thread_sendable` on the cause, nothing called the boolean;
   `cargo check --all-targets` reported `method is_thread_sendable is never
   used`. Keeping a `pub(super)` predicate alive for a possible future letter is
   precisely the dead code AGENTS.md forbids, and the plan's own instruction was
   "do not leave two walks" — one entry point satisfies that more strongly than
   two. The per-arm contract from its doc comment was moved onto
   `thread_unsendable_cause` so nothing was lost.

**C6 — the resource tables only answer to a package-qualified spelling.**
Found while writing the cause tests: `registry().resolve_type` splits its
argument on the first `.` (`src/codegen/registry/mod.rs:1372`), so `close_op_for`
and `is_resource_sendable` see `process.Process` as a resource and a bare
`Process` as an unknown name — which the walk treats as vacuously sendable. This
is correct for the checker (IR always carries qualified names) but it is a live
trap for anyone writing a test with a hand-spelled type, and it is why the first
version of `cause_walk_accepts_a_sendable_bare_resource_but_rejects_an_unsendable_one`
failed with `left: None`. Both spellings are now pinned in that test so the
contrast cannot be mistaken for a typo. No production code changed for this.

**C7 — the full `cargo test` ends `CARGO_EXIT=101` with exactly 2 failures, and
both are bug-483, pre-existing on main.**
`cargo test --no-fail-fast` over the whole tree: 79 suites green, 2 failed.

    tests/rt_tls_listener_local_address.rs:188
      tls_local_address_reports_the_port_a_listener_bound_to
    tests/rt_tls_listener_thread_transfer.rs:192
      a_transferred_tls_listener_accepts_on_the_receiving_thread

Both assert `unexpected first line from the TLS server: ""`.

*Attribution, measured here rather than assumed.* Both fixtures' generated server
has the same first output statement (`…local_address.rs:139`,
`…thread_transfer.rs:139`):

```basic
io::print("bound " & bound.host & " " & toString(bound.port))
```

`bound.host` is a `net::Address` String field. I reduced it to a program with
**no thread, no resource plane and no record field** — nothing this letter can
reach — built it with this worktree's binary, and ran it:

```
$ target/release/mfb build /tmp/p114-tls      # builds clean, no diagnostic
Wrote executable to /tmp/p114-tls/build/tlsprobe.out
$ /tmp/p114-tls/build/tlsprobe.out
about to read .host
bound  60607
=== exit=138 ===
```

The host is **empty** and the process dies (138 = 128+10, SIGBUS). That is
bug-483: bug-480 Phase 4b package-qualified the builtin value types, and
`is_pointer_string_record` still matched only the bare leaf, so `net.Address`
silently switched to the inlined-String record layout while its runtime helper
still writes an absolute pointer. Independently bisected by the session working
bug-483 to first-bad-commit `363b85696`, with every commit from there to main tip
bad; their fix is `fdac03c3c` on `worktree-B-483`.

*Why it is not this letter's.* The build succeeds in both fixtures — the panic is
in `read_bound_line`, well past `expect("build output executable path")` — so the
verifier accepted both programs and no diagnostic of this letter's fired. This
letter touches `src/ir/verify/resources.rs`, `src/rules/table.rs`, two spec files,
`src/ir/verify/tests.rs` and one syntax golden; none of them reaches codegen,
record layout, or `net`/`tls`.

*One thing this does NOT prove, recorded so a later reader does not over-read it.*
`rt_tls_listener_thread_transfer` does exercise a thread **resource** plane
(`Thread OF RES tls::Listener TO Integer`), which is a path this letter touched.
But it dies at line 139, in `main`, **before** `thread::transfer` is ever called —
so its failure is no evidence about the resource plane, and its passing once
bug-483 lands will not be evidence either. The resource-plane behaviour is covered
by `the_resource_plane_keeps_the_generic_unsendable_rule` instead, which asserts it
directly.

Not fixed here: bug-483 is already filed, already root-caused, and already fixed
on another branch. Re-fixing it in this worktree would duplicate and conflict with
landed work. Letter B rebases onto it (see plan-114-B Corrections C1).

## Summary

The engineering risk is one thing only: a cause walk that disagrees with
`is_thread_sendable` and silently drops a diagnostic. §4.2 removes that risk by
construction — one walk, two entry points — and the nine untouched assertions
prove it. The source-level parse gap that made this diagnostic unreachable is
bug-463 — a prerequisite, deliberately not absorbed here. Untouched: the type
grammar, the resource plane, codegen, and the set of accepted programs.
