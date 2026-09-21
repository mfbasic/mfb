# plan-142-H: In place for module-level globals (S2)

Last updated: 2026-09-20
Effort: large (3h–1d)
Depends on: plan-142-G, and (via plan-142-A's Prerequisites) bug-665 and bug-666 fixed

Prerequisites: see plan-142-A. **Re-run the bug-665 and bug-666 rows before
starting this letter** — this letter is the one that relies on them.

Every write to a module-level `MUT` lowers through `NirOp::StoreGlobal`
(`builder_control.rs:1060`), which calls no arm: it builds a fresh block and frees
the old one. That is plan-141's headline finding (global `List` set 22,198 ns vs
36 ns on a local; global `Map` set 468,418 ns). After H, `g = op(g, …)` on a global
`List`/`Map`/`Set`, and `gs = gs & t` on a global `String`, run the same arms as a
local, on the global's own block.

References: plan-141 findings Appendix B.1 (every global write reaches
`StoreGlobal`; no optimizer localizes globals); `.ai/canvas-threading.md:24-35`
(globals are per-thread, x19-relative — no cross-thread race);
`src/codegen/engine/value/operand_snapshot.rs` (bug-496).

## 1. Goal

- `S2` in `ENABLED_SITES`; every `Arm` row fires at S2; harness S2 template flat in `N`.
- The plan-141 probes `c_setL_S2` and `c_setM_S2` show an arm marker, and the
  Brogue-style timing loop (`/tmp/inplace_probe`) is within 2× of the local case
  (recorded, not a gate).
- Runtime cases (value semantics): `LET y = g` before a global self-update is
  unchanged after it; `f(g)` where `f` self-updates `g` (bug-665's shape) prints the
  entry-time parameter; `FOR EACH v IN g` whose body self-updates `g` (bug-666's
  shape) visits the entry-time elements; a failed self-update inside a `TRAP`
  leaves `g` unchanged.

### Non-goals

- `StoreGlobal`'s behavior for a non-self-update (`g = other`) is unchanged.
- Global storage layout: globals stay one 8-byte slot each, **except** the
  `String` capacity shadow (Open Decision 1).
- Global records (S5) — out of scope.

## 2. Current State

- A global read yields its live block pointer (`builder_values.rs:1764-1792`);
  `lower_value_owned` deep-copies a `Global` source (`value_is_aliasing_source`,
  `builder_values.rs:1343-1354`), so every *bind* of a global is a copy.
- Borrowers of a global's block that can be live during `g = op(g, …)`:
  (1) a parameter of an active frame passed `g` — made safe by bug-665 (a callee
  that can write `g` receives a copy); (2) a `FOR EACH` over `g` — made safe by
  bug-666 (a loop whose body can write `g` walks a copy); (3) operand 0 of the same
  statement when a later operand can run user code — bug-496's snapshot;
  (4) a builtin's callback that writes `g` while the builtin walks it — bug-665's
  callback row. No other borrower was found (plan-142 research, 2026-09-20).
- The G25 walker (`inplace_dest.rs:894-1010`), generalized by bug-665 to a
  `StoreGlobal` leaf, answers "can this operand's evaluation write `g`".

## 3. Design

- **`InPlaceDest::Global { name, scratch_slot }`**: open = `load_global_address(name)`
  then load the block pointer into a scratch frame slot; the arm runs on it as a
  `Direct` destination; close = store the scratch slot back to the global's address
  (the global's address is re-derived after the arm, because the arm's `arena_free`
  calls clobber caller-saved registers — the same care `StoreGlobal` takes at
  `builder_control.rs:1120-1129`).
- **Dispatch**: `NirOp::StoreGlobal` calls `try_inplace_self_update` with that
  site before its copying path, when the value is a self-update of the same global.
- **Gate G-global-operand** (new, named in the table): decline if any operand other
  than operand 0 can reach a `StoreGlobal` of `g` (the generalized walker). Such an
  operand would reallocate `g` under the arm. This is the only remaining decline at
  S2; it gets a runtime case that proves the copying path stays correct.
- **`String` concat**: the `concat` arm needs a capacity shadow for its target
  (`string_capacity_slots`, a frame slot today). A global has no frame; see Open
  Decision 1.

Risk: this letter is the plan's highest blast radius. A borrower missed in §2 is a
use-after-free. The four runtime cases in §1 exist to catch exactly those, and the
matrix runs every arm at S2.

## Phases

### Phase 1 — Borrower audit, re-verified

- [x] Re-read the four borrower rows in §2 against the code as it is after bug-665
      and bug-666 landed; for each, name the runtime case that proves it (the four
      in §1). Record any new borrower found and add its case.
      Prerequisite rows re-run first: `cargo test --test rt_global_argument_reassigned_by_callee
      --test rt_for_each_over_reassigned_global` → `7 passed` and `6 passed`.
      (1) a parameter passed `g` — `want_arguments_the_call_can_free` snapshots it when
      the call reaches `StoreGlobal(g)`: case `parameter`. (2) `FOR EACH v IN g` —
      `lower_for_each`'s bug-666 snapshot: case `for_each`. (3) operand 0 with a later
      user-code operand — bug-496's snapshot on the copying path, and G-global-operand
      declines the arm: case `later_operand_writes`. (4) a builtin's callback writing
      `g` while the builtin walks it — G-global-operand asks the call itself
      (`call_reaches_store`), not only the operands (Correction H1). Also confirmed:
      a bind (`LET y = g`) deep-copies a `Global` source (case `let_copy`), and the
      read-only `get` borrow (plan-86 E) only ever borrows from a local
      (`collect_borrow_get_locals` matches `NirValue::Local` containers), so no
      global has that borrower. Added: failure atomicity (`trap`), Map and Set
      globals (`map_and_set`), the global `String` (`string_concat`), and its
      reassignment (`string_reassigned`, Phase 3).
- [x] Add those runtime cases to `tests/runtime/rt_global_self_update.rs` (+ stanza),
      passing against the **copying** path first (they test value semantics, which
      must hold before and after). The first seven passed before any H code
      (`test result: ok. 1 passed`, on the copying path); all eight pass after.

Acceptance: `cargo test --test rt_global_self_update` → pass (est. 3 min).
Verified 2026-09-21: `test result: ok. 1 passed; 0 failed` (all eight cases).
Commit: —

### Phase 2 — `InPlaceDest::Global` and S2 for collections

- [x] `InPlaceDest::Global` open/close; `StoreGlobal` dispatch; G-global-operand.
      `InPlaceDest::Global { name, block_slot }`: `open_inplace_ref_dest` copies the
      global's block pointer into a frame slot, `close_inplace_dest` re-derives the
      global's address after the arm and stores it back. `StoreGlobal` builds the
      site for `g = f(g, …)` (`is_global_self_update_call`) and `gs = gs & …`, and
      falls through to its copying path when every arm declines. The arms name
      their binding through `SelfUpdateSite::is_self`/`read_by` (G5/G6 and the
      self-alias checks), so a global site matches `NirValue::Global` (Correction
      H2). G-global-operand is in `resolve_self_update` and the concat arm.
- [x] Add S2 to `ENABLED_SITES` and the harness (the global declared at module
      level, the statement inside a SUB). `Site::Global` in both: the matrix probe's
      loop is in `SUB run1()`, the harness puts the whole program body (setup,
      `before`, the loop, the checks) in `SUB run1()`, which `main` calls. The first
      S2 run found `sortBy`/`mapValues` declining — their monomorph target hid the
      callback from G-global-operand (Correction H1) — and a front-end bug: a user
      top-level `x` beside `collections::union` did not compile (fixed first,
      `a74a27718`, Correction H4).

Acceptance: `cargo test --bin mfb self_update && cargo test --test rt_inplace_self_update --test rt_global_self_update`
→ pass at S1, S2, S7, S9 (est. 25 min).
Verified 2026-09-21: `cargo test --bin mfb self_update` → `test result: ok. 4 passed`;
`cargo test --test rt_inplace_self_update` → `test result: ok. 1 passed; 0 failed`
(716.24s; 253 case/site pairs — 64 at S1, 63 each at S2, S7, S9; the `&` row joined S2
in Phase 3); `cargo test --test rt_global_self_update` → `test result: ok. 1 passed`.
Recorded (not a gate), `/tmp/inplace_probe`, ns/op before → after: global `List` set
21298 → 9 (local 8), global `Map` set 465549 → 50 (local 36); the matrix shows the
`set` arm's marker at S2 for both overloads (plan-141's `c_setL_S2`/`c_setM_S2`).
Commit: —

### Phase 3 — Global `String` concat

- [x] Implement Open Decision 1's choice; add `gs = gs & t` to the harness at S2.
      `add_global_string_capacities` (`self_update.rs`), run right after opt1
      (`target/shared/lower.rs`) so no optimizer row drops storage only codegen
      names, declares `$strcap$<g>` beside each global `String` with a self-append.
      The concat arm works on it through a frame slot; every other `StoreGlobal` to
      `g` frees the old block with that capacity and resets it to 0 — which also
      zeroes it from the global's own initializer store (Correction H3). The
      harness's `Site::applies` and the matrix now include the `&` row at S2. Case
      `string_reassigned` RED-checked by deleting the reset: the program crashes
      (`printed "" … exit None`).

Acceptance: `cargo test --test rt_inplace_self_update` → the `concat` S2 row flat in `N` (est. 5 min).
Verified 2026-09-21: `MFB_SELF_UPDATE_FILTER='&' cargo test --test rt_inplace_self_update`
→ `test result: ok. 1 passed` (the `&` line at S1 and S2); `cargo test --bin mfb
self_update` → `4 passed` with the `&` probe at S2.
Commit: —

### Phase 4 — Expected outputs

- [ ] Globals self-updated in committed fixtures — measure with
      `rg -lP '^MUT (\w+)' tests examples --glob '*.mfb'` intersected with files
      containing `\1 = …(\1` (a two-step script; record the count here), regenerate
      any committed golden among them; each diff must be a `StoreGlobal` replaced by
      an arm. `examples/brogue` must still match its oracle.

Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` green
(est. 15 min — globals appear in every package's fixtures, so no directory subset
is known to cover them).
Commit: —

## Validation Plan

- Tests: `rt_global_self_update`; matrix + harness at S2.
- Runtime proof: plan-141's `/tmp/inplace_probe` timings re-run and recorded.

## Open Decisions

1. **RESOLVED (user, 2026-09-20): a hidden shadow global.** A hidden 8-byte global
   slot is allocated beside each global that is a self-concat target (found by the
   same prescan as locals, run over the whole module) and zeroed by the global
   initializer. The concat arm reads and writes it as it does a local's frame shadow.

## Corrections

- **H1 (Phase 2): G-global-operand asks the call too.** §3 has it decline when an
  operand other than operand 0 can reach a `StoreGlobal` of `g`. A callback is not
  an operand evaluation — `g = filter(g, p)` where `p` writes `g` runs `p` inside
  the arm — so the gate also asks `call_reaches_store` of the call itself. A
  `Body::Mfb` member arrives as `#collections_X$T`, whose body calls the callback
  through a `FUNC` parameter (opaque to the walk), so the gate asks it as
  `collections.X`, for which the walk follows the callback.
- **H2 (Phase 2): the self-alias checks name the binding through the site.** The
  arms compared `args[1]` (and the concat operands) against `NirValue::Local(name)`;
  at S2 the binding is `NirValue::Global`. `SelfUpdateSite::is_self`/`read_by`
  answer for either, and G5/G6, bulk `append`, `union`/`merge` and `concat` use them.
- **H3 (Phase 3): the shadow's placement and reset.** Open Decision 1's hidden global
  is added after opt1, not with the other globals: dead-global elimination would
  otherwise remove storage no NIR op names. "Zeroed by the global initializer"
  holds through the reset every non-self-append `StoreGlobal` performs — which is
  what keeps the shadow honest at all (plan-142-G Correction G1's overflow, for a
  global), and the initializer's own store is one.
- **H4 (Phase 2): a front-end bug, fixed first.** The S2 harness's global `x` made
  every `union` line fail to build: a built-in member's monomorph is emitted into
  the user's first file, and its local `x` hit `SYMBOL_SHADOWS_TOP_LEVEL_BINDING`.
  Fixed in `a74a27718` (`rt_user_global_vs_builtin_locals`).

## Summary

The mechanism is small (one destination kind, one dispatch call, one gate); the
risk is entirely the borrower audit, which is why it is re-verified first and each
row carries a runtime case.
