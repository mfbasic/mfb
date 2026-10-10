# plan-13-D: the shadow tree, the emitted layout solver, and the headless host

Last updated: 2026-07-20
Effort: **UNMEASURED — Phase 0 measures it.** Provisionally medium–large; if Phase 0
returns large, split by axis before continuing (§Phase 0).
Depends on: plan-13-C. Feature-wide precondition: plan-13 master §Prerequisites.
Produces: the worker-side shadow tree + dirty model, `_mfb_rt_app_layout` (the 125th
runtime helper), and the `headless` host + `--app-host headless`. Consumed by 13-E, 13-F,
13-H, 13-I.

The worker-side model, the emitted solver, and the backend that makes both testable
without a display.

The single behavioral outcome: the **real emitted solver** produces correct frames for the
full `Direction × Justification × Align` matrix under the headless host, on macOS and
Linux, with no display server — byte-identical between them.

**This is the largest single item in plan-13 and the one number nobody measured.** The
2026-07-09 draft budgets "~1500–2500 lines of emitter" with no derivation. The closest
precedent in the tree, `term_grid.rs`, is **1202 lines** for a strictly simpler problem
(fixed cell grid, no measure callback, no nested flex). That is a floor, not an estimate.

References (read first):

- `src/target/shared/code/term_grid.rs` — the precedent. 1202 lines; find its entry points
  with `rg -n 'fn emit_grid_alloc|fn emit_grid_present'`.
- `src/target/shared/code/float_format.rs` — 596 lines emitting one helper; the draft's
  stated comparator ("several times `float_format.rs`").
- `src/target/shared/runtime/mod.rs` — `RuntimeHelperSpec`. **The registry the draft never
  mentions** (§3.2).
- `planning/old-plans/superseded-plan-13-A-app-builtin.md` §7, §8.1, §8.3 — the shadow/dirty model and the
  measurement contract this preserves.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-13-C has landed | `ls src/builtins/app.rs` | **NOT MET** |
| The runtime-helper registry has room and a known shape | `rg -n 'struct RuntimeHelperSpec' src/target/shared/runtime/mod.rs` | **MET** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> before continuing and again before deciding to stop; report every row if you stop.

## 1. Goal

- A worker-side **shadow tree**: `window`/`add*`/`remove`/`attach`/`slot` create and mutate
  nodes with parent/child links, per-node dirty + structure-dirty flags, and per-node
  property shadows. The window's one-root-child rule raises `ErrInvalidArgument`.
- `_mfb_rt_app_layout`: **allocation-free, re-entrant, main-thread-callable**; walks a flat
  node array plus an indirect `host_measure` fn-ptr; produces one `Rect` per node for
  Row / Column / Stack across the full `Justification × Align × Size`(`<0` = fill) matrix,
  honoring padding and margin and skipping `display:none` nodes.
- Its `RuntimeHelperSpec`, catalog entry and usage gating alongside the other 124 helpers.
- A `headless` host + `mfb build -app --app-host headless`: synthetic deterministic
  `host_measure`, `host_set_frame` printing `id kind x y w h`, immediate-FALSE
  `host_wait_events`, window closes after the first `sync`.

### Non-goals (explicit constraints)

- **No native backend.** macOS is 13-E, GTK is 13-F.
- **No events, no Input.** 13-G.
- **No caching of measured sizes.** The solver *calls* the measure fn-ptr (single-pass per
  leaf in v1) rather than baking a size, so a future multi-pass solver is a change of
  strategy, not of contract.
- **No allocation inside the solver, ever.** It runs on the main thread during present and
  resize; an allocation there is a UI stall at best.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| Closest emitted-helper precedent | `term_grid.rs` **1202 lines** | `wc -l src/target/shared/code/term_grid.rs` |
| The draft's stated comparator | `float_format.rs` **596 lines** | `wc -l src/target/shared/code/float_format.rs` |
| Runtime helpers today | **124** | `rg -oh '_mfb_rt_[a-z0-9_]*' src/ \| sort -u \| wc -l` |
| Layout combinations to cover | `3 Direction × Justification × Align` + fill + padding/margin + hidden-sibling | the matrix in §1 |
| The solver's budget in the draft | "~1500–2500 lines" | **UNMEASURED — no derivation given** |

### 2.2 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| An emitted helper of this scale has a precedent | **CONFIRMED** | `term_grid.rs`, 1202 lines |
| That precedent solves a *simpler* problem | **CONFIRMED** | fixed cell grid; no measure callback, no nested flex, no fill distribution |
| The solver needs a registry entry | **CONFIRMED** | 124 helpers each have a `RuntimeHelperSpec`; the draft never mentions the registry |
| The solver must be re-entrant | **CONFIRMED, and the reason is 13-I** | plan-13-I: its scroll handler is a *third* caller, and it says "'re-entrant on the main thread' is a hard constraint that **this plan, not plan-13-C, is the reason for**" |
| No atomic instruction encoders exist (and none are needed) | **CONFIRMED** | `rg -n 'ldaxr\|stlxr\|cmpxchg' src/arch/` → one comment only. The design uses a pipe + idle-post, no shared mutable memory |
| The solver is 1500–2500 lines | **UNMEASURED** | Phase 0 |

**On re-entrancy:** this is a **reverse dependency**. 13-D must build a property whose only
justification lives in 13-I. Build it anyway and record why — dropping it because "nothing
here needs it" would silently break 13-I much later, and re-entrancy is not something that
can be retrofitted into an emitted helper cheaply.

## 3. Design Overview

Three pieces, deliberately ordered so the unmeasured one is measured first.

### 3.1 The measurement contract

The solver takes an indirect `host_measure` fn-ptr rather than a baked size. That keeps it
**multi-pass-ready**: v1 calls it once per leaf, but nothing in the contract says it may
only be called once. Baking sizes would make a future multi-pass solver a rewrite.

### 3.2 The registry entry the draft forgot

`_mfb_rt_app_layout` is not just an emitter — it is the **125th** entry in the runtime
helper registry and needs a `RuntimeHelperSpec`, a catalog entry, and usage gating so it is
only emitted for programs that call `app::`. No plan-13 document mentions the registry.
Without gating, every program pays for the solver.

**Where design uncertainty concentrates: the size, and it is the whole point of Phase 0.**
The solver decides whether plan-13 is `huge` or worse, and its only stated number has no
derivation. Everything downstream — 13-E, 13-F, 13-H, 13-I — waits on it.

**Where correctness risk concentrates:** the fill (`Size < 0`) distribution across a
`Justification × Align` matrix. It is the case with the most interacting rules, it is
where an off-by-one produces a *plausible* layout rather than an obviously broken one, and
it is exercised by every real UI. The golden matrix under the headless host is the guard,
and it must be driven through the **real emitted solver**, never a Rust-side model of it.

**Rejected alternative:** *write the solver in Rust and call it from emitted code.*
Rejected: it would run on the compiler's side of the boundary and could not be called from
the native resize handler, which is the whole point of native-owned layout.

**Rejected alternative:** *cache measured sizes per node.* Rejected — §3.1.

**Rejected alternative:** *prove layout on a real backend first and add headless later.*
Rejected: layout correctness is then hostage to a display server and to AppKit's own
behavior, and the macOS/GTK/headless byte-identity claim has no referent.

## Compatibility / Format Impact

- **New:** `_mfb_rt_app_layout` + its registry entry; the shadow tree; a `headless` host
  backend; the `--app-host headless` CLI flag.
- **Unchanged:** every existing runtime helper; app mode's existing behavior; every
  program that does not call `app::` (usage gating, §3.2).

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 0 — measure the solver (before anything is scheduled behind it)

No production code. This phase exists because §2.1's last row is the only unmeasured
number in plan-13 and it sizes the feature.

- [ ] Implement **Row-only, single-axis, no fill** as a throwaway spike and count its
      emitter lines.
- [ ] Extrapolate to the full matrix; compare against `term_grid.rs` (1202) and
      `float_format.rs` (596).
- [ ] Record the derived estimate in this document. **If it lands at `large` or above,
      split this sub-plan by axis before continuing** — Row/Column, then Stack + z-order,
      then the `Justification × Align` matrix + fill/padding/margin.

Acceptance: a derived, written-down estimate with the spike that produced it. The
2026-07-09 "~1500–2500" is replaced by a number with a derivation, or this sub-plan is
split.
Commit: —

### Phase 1 — the shadow tree (pure worker-side data structure)

Deliberately separable from the solver: this phase has no emitter and the solver has no
shadow-tree knowledge. They share only the flat node array's layout.

- [ ] Node kinds, parent/child links, per-node dirty + structure-dirty flags, property
      shadows.
- [ ] `window`/`add*`/`remove`/`attach`/`slot` mutations; the one-root-child rule →
      `ErrInvalidArgument`.
- [ ] Tests: mutations update links and dirty flags as specified.

Acceptance: shadow-tree mutations behave as specified, headless, with no solver. **Landing
this alone gives 13-H and 13-I something to build nodes against before the solver exists.**
Commit: —

### Phase 2 — the headless host

Before the solver, so the solver has somewhere to run the moment it exists.

- [ ] `src/target/shared/widgets/headless` + `--app-host headless`.
- [ ] Synthetic deterministic `host_measure`; `host_set_frame` printing `id kind x y w h`;
      immediate-FALSE `host_wait_events`; window closes after the first `sync`.

Acceptance: a program builds with `--app-host headless`, runs with no display, and prints
a deterministic frame line per node.
Commit: —

### Phase 3 — the solver

- [ ] `_mfb_rt_app_layout`: allocation-free, **re-entrant** (§2.2 — the reason is 13-I),
      main-thread-callable, walking a flat node array + an indirect `host_measure`.
- [ ] Row / Column / Stack across the full `Justification × Align × Size`(`<0` fill)
      matrix, honoring padding/margin, skipping `display:none`.
- [ ] Its `RuntimeHelperSpec`, catalog entry and usage gating (§3.2).
- [ ] Tests: `tests/rt-behavior/app/layout-*` goldens driving the **real emitted solver**
      through the headless host — one per `Direction × Justification × Align`, plus
      `<0` fill, padding/margin, and hidden-sibling reflow.

Acceptance: the full matrix is correct under the headless host on macOS **and** Linux with
no display server, byte-identical between them. A Rust-side model of the solver does not
satisfy this — the goldens must run the emitted code.
Commit: —

## Validation Plan

- Tests: the golden layout matrix, through the emitted solver, under the headless host.
- Coverage check: `tests/rt-behavior/app/` is new — confirm its goldens land in the gate's
  denominator before relying on them. A green gate over a directory with no goldens proves
  nothing.
- Runtime proof: the headless host *is* the runtime proof for this sub-plan; a display is
  not required and deliberately not used. On-device proof arrives in 13-E/13-F, and its
  acceptance is that it matches these frames.
- Doc sync: none — the solver is internal. `--app-host headless` needs a CLI mention.
- Acceptance: the project's full suite.

## Open Decisions

1. **Whether to split by axis** (§Phase 0). Recommended: decide from the spike, not in
   advance. If Row-only lands near `term_grid.rs`'s per-feature density, the full matrix
   is large and must split.
2. **Whether the headless host ships or is test-only.** Recommended: ship it. It is the
   only way to test layout in CI without a display, and it costs one flag.
3. **Single-pass vs multi-pass measurement in v1.** Recommended single-pass, with the
   fn-ptr contract preserved (§3.1) so multi-pass is a later strategy change rather than a
   rewrite.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **The runtime-helper registry was never mentioned** in any plan-13
  document. `_mfb_rt_app_layout` is the 125th helper and needs a `RuntimeHelperSpec`,
  catalog entry and usage gating, or every program pays for the solver.
- 2026-07-20 — **The solver's size is the only unmeasured number in plan-13**, and it sizes
  the feature. `term_grid.rs` does a simpler job in 1202 lines, so "~1500–2500" is a floor
  with no derivation. Phase 0 now measures it before anything is scheduled behind it.
- 2026-07-20 — **Re-entrancy is a reverse dependency on 13-I**, which the draft did not
  record on A's side. Building it here is correct; dropping it as unneeded would break
  13-I much later and cannot be retrofitted cheaply.
- 2026-07-20 — **The shadow tree and the solver are separable** and are now separate
  phases: the tree has no emitter, the solver has no tree knowledge. Landing the tree alone
  unblocks 13-H and 13-I early.

## Summary

The engineering risk here is a number: the solver's size has no derivation, it is the
largest item in the feature, and four other units wait on it. Phase 0 measures it and is
allowed to conclude that this sub-plan must split.

The correctness risk is fill distribution across the `Justification × Align` matrix, where
a wrong answer looks plausible rather than broken — which is why the goldens drive the real
emitted solver through a headless host rather than a Rust model of it.

The structural improvement is separating the shadow tree from the solver. They share only
a node-array layout, and landing the tree first gives TextArea and Table something to build
against months before the emitter is finished.

What is left untouched: every existing runtime helper, app mode's current behavior, and
every program that does not call `app::`.
