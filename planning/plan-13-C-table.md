# plan-13-C: `app::Table`

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: plan-13-S (the shadow tree, and specifically a **re-entrant** solver — §3.1)
and plan-13-D's detach rules. **Not on plan-13-B** — see §Open Decisions 1.
Feature-wide precondition: plan-13 master §Prerequisites.
Produces: `app::Table`, the widget-cell grid, and native-side virtualization.

A widget-cell grid: `app::Table` is a container addressed by `(row, col)` where each cell
holds exactly **one ordinary widget** — any `app::Widget`, including a real `Container` for
composite cells. Header rows are a second, pinned grid over the scrolling data grid.

The single behavioral outcome: a loop-built grid of thousands of cells scrolls smoothly
with only the visible rows instantiated natively, and every cell widget is destroyed
exactly once.

References (read first):

- `planning/plan-13-C-app-table.md` §3–§5 (the 2026-07-09 original) — the design this
  preserves, including its 2026-07-02 redesign note replacing a template+data virtualized
  table with a grid of ordinary widgets.
- `planning/plan-13-S-layout-solver.md` §2.2 — **re-entrancy, which exists for this unit.**
- `src/docs/spec/language/15_resource-management.md` — §15.6 resources in collections, and
  the loop-body ownership float this unit's §6 pattern depends on.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-13-S has landed **and its solver is re-entrant** | `rg -n '_mfb_rt_app_layout' src/` | **NOT MET** |
| plan-13-A has landed (union params, `WIDGET_VARIANTS`) | `ls src/builtins/app.rs` | **NOT MET** |
| A backend exists to render into | `rg -n 'host_present' src/target/` | **NOT MET** |
| Loop-body ownership float behaves as §6 assumes | `rg -n 'escape' src/docs/spec/language/` — read the escape-analysis topic | **UNVERIFIED — confirm before Phase 2** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> before continuing and again before deciding to stop; report every row if you stop.

## 1. Goal

- `app::Table` shadow node with **two** cell grids (header and data), `(row, col)` slots
  holding widget **borrows**.
- `addTable`; the table `add*` overloads (born-attached into a cell, required `region`);
  `setWidget` (detach-occupant-then-attach; `ErrInvalidArgument` on a `Table` argument —
  no nesting) / `clearCell` (detach).
- Extents (`tableRowCount`/`tableColCount`/`tableHeaderRowCount`), `setColumnWidth`/
  `setRowHeight`.
- **Native-side virtualization only** — no user-visible recycling. Roughly
  `visible rows × columns` native peers exist at a time; the shadow grid is complete.
- Lifetime: cell slots are borrows; a cell widget's drop empties its cell; table
  close/drop detaches all cells.

### Non-goals (explicit constraints)

- **No user-visible cell recycling.** The 2026-07-02 redesign removed the template+data
  model and its table-owned "second lifetime regime" deliberately; do not reintroduce it.
- **No nested tables.** `setWidget` with a `Table` is `ErrInvalidArgument`.
- **No solver change.** The scroll path *calls* the solver; it does not alter it.
- **No dependency on 13-B.** The `addTextArea` table overload waits for it; nothing else
  does (§Open Decisions 1).

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| New seam ops | **8** (`host_create_table`, `_set_extent`, `_set_column_width`, `_set_row_height`, `_place_cell`, `_clear_cell`, `_set_selection_mode`, `_set_selection`) | 2026-07-09 plan-13-C §5.3 |
| — × 3 backends | **24 implementations** | macOS, GTK4, headless |
| `app::` callables this unit adds | **22** | the 2026-07-09 surface section |
| Solver callers after this lands | **3** (`host_present`, native resize, **this unit's scroll handler**) | §3.1 |
| Widget variants after this lands | 7 concrete + 1 union | master |

### 2.2 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| The solver must be re-entrant, and this unit is the reason | **CONFIRMED** | the 2026-07-09 plan-13-C says so in its own words: *"'re-entrant on the main thread' is a hard constraint that **this plan, not plan-13-A, is the reason for**"* |
| Cells hold borrows, not owned handles | **CONFIRMED design** | §15.6 resources in collections; a cell slot never owns |
| B and C are independent | **CONFIRMED** | both 2026-07-09 docs say so, and neither seam block references the other's symbols |
| Loop-body ownership float supports the §6 pattern | **UNVERIFIED** | a Prerequisites row — confirm against the escape-analysis spec before Phase 2 depends on it |
| A 100 000-iteration loop-built grid drops each widget exactly once | **UNVERIFIED — an acceptance criterion** | proven under the leak checker |

## 3. Design Overview

A complete shadow grid, a partial native grid, and a scroll handler that reconciles them.

### 3.1 Re-entrancy is a reverse dependency

This unit's scroll handler is a **third** caller of `_mfb_rt_app_layout`, alongside
`host_present` and the native resize handler. It runs on the main thread, during a scroll,
while a present may already be in flight — which is what makes re-entrancy a hard
requirement rather than a nicety.

**The justification for that property lives here, but the property must be built in 13-S.**
That is recorded in both documents so nobody drops it from 13-S as unneeded; re-entrancy
cannot be retrofitted into an emitted helper cheaply.

**Where design uncertainty concentrates: the virtualizer.** Instantiating only visible rows
while the shadow grid stays complete means the native peer for a given cell appears and
disappears under scrolling, while the MFBASIC handle stays valid throughout. That mapping —
stable logical cell, transient native peer — has no precedent in this codebase.

**Where correctness risk concentrates:** the cell-drop interaction. A cell holds a
*borrow*; the widget is owned by whatever bound it. So a widget dropping must empty its
cell, and a table dropping must detach rather than destroy. Get this backwards and either
the table frees something it does not own (double free) or a dropped widget leaves a
dangling native peer in a cell.

**Rejected alternative:** *the template+data virtualized table* (prototype rows,
`bindColumn` slots, recycled cell instances). Rejected in the 2026-07-02 redesign and the
rejection stands: it introduced a table-owned second lifetime regime alongside MFBASIC's
scope-drop model, i.e. two owners for one widget.

**Rejected alternative:** *instantiate every cell natively.* Rejected: a 100 000-cell grid
would create 100 000 native peers, which neither toolkit tolerates.

## Compatibility / Format Impact

- **New:** `app::Table`, 8 seam ops × 3 backends, one `WIDGET_VARIANTS` row, one close op,
  `Region`/`SelectionMode`/`TableCellEvent` types in the high reserved ID range.
- **Unchanged:** the solver's behavior (it gains a caller, not a change); the seam's
  existing ops.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 1 — the shadow grid, headless

- [ ] `app::Table` shadow node (two cell grids, `(row, col)` borrow slots) + the
      `app::Widget` `Table` variant + `WIDGET_VARIANTS` row + the internal
      `app::destroy(RES app::Table)` close op; the three new types in the reserved range.
- [ ] `addTable`; the table `add*` overloads (born-attached, required `region`);
      `setWidget` / `clearCell`; extents; `setColumnWidth`/`setRowHeight`.
- [ ] Lifetime wiring: cell slots are borrows; a cell widget's drop empties its cell;
      table close/drop detaches all cells.
- [ ] **Confirm loop-body ownership float** against the escape-analysis spec before Phase 2
      depends on it.
- [ ] **Re-run 13-A's overload-name coherence `#[test]`** after adding the arity-3 table
      `add*` forms — and enumerate the collisions exhaustively. The 2026-07-09 draft hedged
      this as keeping "*most* of them" clear of A's container forms; the test enforces all.
- [ ] Tests: `tests/syntax/app/table-*` — arity/types, union widening (`setWidget` accepts
      every variant), skipped-middle-argument rejection, **nested-table rejection**, extent
      math, detach-not-destroy, RES-borrow rejection.

Acceptance: the full Table surface typechecks; the shadow grid model is verified headless;
a **100 000-iteration loop-built grid compiles and drops each widget exactly once**.
Commit: —

### Phase 2 — the virtualizer, one backend (the risk)

- [ ] macOS: the native grid, the config mirror, and the virtualizer — only visible rows
      instantiated, shadow grid complete.
- [ ] The scroll handler as the third solver caller (§3.1).

Acceptance: a large grid scrolls smoothly with roughly `visible rows × columns` native
peers, and a cell's MFBASIC handle stays valid across scrolling it out of view and back.
Commit: —

### Phase 3 — the second backend, selection, activation

- [ ] GTK4 grid + virtualizer; re-run Phase 2's proofs.
- [ ] Selection modes and cell activation events.

Acceptance: identical scrolling and lifetime behavior on GTK4; selection and activation
behave the same on both.
Commit: —

## Validation Plan

- Tests: syntax fixtures; the headless model proofs; the 100 000-cell drop-exactly-once
  proof under the leak checker.
- Coverage check: `tests/syntax/app/` is golden-backed; the virtualizer proofs are
  on-device and stated as such.
- Runtime proof: macOS and the Debian aarch64 GTK4 box.
- Doc sync: `src/docs/spec/stdlib/` + `src/docs/man/builtins/app/` — **not
  `src/docs/spec/package/`**, which the 2026-07-09 draft named in three places and which is
  the binary container format (master §2.5).
- Acceptance: the project's full suite, no leaks.

## Open Decisions

1. **The `addTextArea` table overload.** The 2026-07-09 draft called plan-13-B "a **soft
   dependency**". Soft dependencies are how two plans braid. Recommended: **ship this unit
   without that overload** and add it when 13-B lands — then the dependency is either
   absent or hard, never soft.
2. **Whether header rows are a separate grid or row 0 with a pin flag.** Recommended
   separate, as designed: a pinned region has different scroll behavior and folding it into
   the data grid puts a conditional in every cell path.
3. **How many rows beyond the viewport to instantiate.** Recommended one screen of overscan
   each way, measured on the slower backend rather than guessed.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **`Depends on:` moved into the header.** The old document buried it 403
  lines in. Its content was good — it named A's Phase 0 and Phase 2 precisely — but a
  reader deciding what to land first never reached it.
- 2026-07-20 — **The "soft dependency" on plan-13-B is removed.** This unit now ships
  without the `addTextArea` overload and gains it when 13-B lands (§Open Decisions 1).
- 2026-07-20 — **The overload-collision hedge is removed.** The draft said the table `add*`
  forms keep "*most* of them" clear of plan-13-A's container forms; 13-A's coherence
  `#[test]` enforces **all**, and this unit re-runs it after adding its arity-3 forms.
- 2026-07-20 — **Re-entrancy recorded as a reverse dependency in both documents** (§3.1),
  so it cannot be dropped from 13-S as unneeded.
- 2026-07-20 — Documentation destination corrected in three places to `stdlib/` +
  `man/builtins/`.

## Summary

The engineering risk is the virtualizer's mapping: a stable logical cell whose native peer
appears and disappears under scrolling, while the MFBASIC handle stays valid throughout.
Nothing in this codebase does that today.

The correctness risk is narrower and is about ownership direction: cells hold borrows, so a
widget's drop must empty its cell and a table's drop must detach. Backwards, and either the
table frees what it does not own or a dropped widget leaves a live native peer behind.

The structural point is that this unit is the *reason* 13-S's solver must be re-entrant,
which is why that requirement is written in both places rather than only where it is used.

What is left untouched: the solver's behavior, the seam's existing ops, and the recycling
model the 2026-07-02 redesign deliberately removed.
