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

- [ ] Re-read the four borrower rows in §2 against the code as it is after bug-665
      and bug-666 landed; for each, name the runtime case that proves it (the four
      in §1). Record any new borrower found and add its case.
- [ ] Add those runtime cases to `tests/runtime/rt_global_self_update.rs` (+ stanza),
      passing against the **copying** path first (they test value semantics, which
      must hold before and after).

Acceptance: `cargo test --test rt_global_self_update` → pass (est. 3 min).
Commit: —

### Phase 2 — `InPlaceDest::Global` and S2 for collections

- [ ] `InPlaceDest::Global` open/close; `StoreGlobal` dispatch; G-global-operand.
- [ ] Add S2 to `ENABLED_SITES` and the harness (the global declared at module
      level, the statement inside a SUB).

Acceptance: `cargo test --bin mfb self_update && cargo test --test rt_inplace_self_update --test rt_global_self_update`
→ pass at S1, S2, S7, S9 (est. 25 min).
Commit: —

### Phase 3 — Global `String` concat

- [ ] Implement Open Decision 1's choice; add `gs = gs & t` to the harness at S2.

Acceptance: `cargo test --test rt_inplace_self_update` → the `concat` S2 row flat in `N` (est. 5 min).
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

## Summary

The mechanism is small (one destination kind, one dispatch call, one gate); the
risk is entirely the borrower audit, which is why it is re-verified first and each
row carries a runtime case.
