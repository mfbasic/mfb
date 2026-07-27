# plan-67-F: Instrument the arena-related code regions

Last updated: 2026-07-26
Effort (Human): medium
Effort (AI): medium

**Platform scope:** macOS only (see plan-67-B). The arena-region wrapping is
emitted only on the macOS backend; Linux and Windows are untouched (their perf
helpers are no-op stubs and are not injected). This removes the cross-backend
register-preservation proof that an all-platform version would have needed.
Depends on: plan-67-D (working `perf_start` / `perf_end`). E is **not** a
dependency — F needs the timing calls, not the statistics — but F is scheduled
**after** E because F has the largest blast radius (it edits the macOS arena hot
path) and must land behind everything else.
Produces:
- Debug-gated `perf_start(name)` / `perf_end(name)` pairs wrapping the
  arena-related emitted code regions, with correct live-register preservation and
  a guaranteed non-recursion property.
- The real multi-row profile the whole feature was for: `system_alloc`,
  `mfb_alloc`, `mfb_free`, `post_alloc`, `post_free`, and any additional
  arena regions the census turns up.

References: `.ai/compiler.md` (register lifetimes — this is the letter where they
bite: a `bl _mfb_*` clobbers x0–x17). Prerequisites: plan-67-A gate. (macOS-only,
so no remote-box verification is needed this round.)

## 1. Goal

- A **debug macOS** build compiles+runs a non-trivial program and `perf_done`
  prints correct per-region timing rows for the arena operations (`mfb_alloc`,
  `mfb_free`, `system_alloc`, `post_alloc`, `post_free`, …), each with a plausible
  count matching the program's allocation behavior. Release output — and all
  Linux/Windows output — byte-identical to pre-plan-67 HEAD.

### Non-goals

- **Not** instrumenting every function — only arena-related regions, per the
  user's scope ("a full system alloc, mfb alloc, mfb free, random bits after
  alloc, random bits after free, etc.").
- No arena semantics change; no release-build change; no Linux/Windows change
  (wrapping is emitted only on the macOS backend).
- The perf helpers themselves must remain arena-free (established in B) so that
  wrapping arena code cannot recurse into perf → arena → perf.

## 2. Current State

- Arena allocation body: `_mfb_arena_alloc` (symbol `error_constants.rs:652`;
  scaffolding `native_helpers.rs:113`). Raw system memory for the arena comes from
  the `emit_arena_map` / `emit_arena_unmap` platform seam (macOS
  `macos_aarch64/code.rs:764,793`; Windows `win_x86_64/code.rs:620,645`; Linux
  `linux_common/code.rs:117`). Post-alloc "random bits" corresponds to the
  fill-RNG region (`ARENA_FILL_RNG_LO/HI_OFFSET` at `error_constants.rs:412-413`).
  Arena free / free-list head is at offset 48 (`error_constants.rs` map).
- A runtime-helper call is emitted with `emit_symbol_call`
  (`builder_emit_helpers.rs:4`) / an `internal_branch` relocation
  (`entry.rs:599-600` precedent). Each such call clobbers x0–x17 (modelled at
  `runtime/mod.rs:70-78`).
- `perf_start`/`perf_end` (from C/D) take a name string-block pointer in the arg
  register and are arena-free.

### Measured populations

| What | Count | Command |
|---|---|---|
| Distinct arena regions to wrap | UNMEASURED — **census is F's first task** | `rg -n 'ARENA_ALLOC_SYMBOL\|emit_arena_map\|emit_arena_unmap\|arena_free\|FILL_RNG' src/target` + read `native_helpers.rs`, `error_constants.rs`, the platform `code.rs` files |

The candidate set from research is `system_alloc` (around `emit_arena_map`),
`mfb_alloc` (the `_mfb_arena_alloc` body), `mfb_free` (arena free path),
`post_alloc` / `post_free` (fill-RNG regions). The census fixes the exact list and
names; do not hard-code the count until it is run.

### Verified properties

- **Perf helpers are arena-free** — VERIFIED by construction in B (region via the
  platform mmap seam, not `_mfb_arena_alloc`; no `bl` to any arena symbol). This is
  the property that makes wrapping arena code safe. F must **not** break it and
  must add a check that no wrapped region reaches a perf helper that reaches the
  arena.
- **Injected calls clobber x0–x17** — VERIFIED (`runtime/mod.rs:70-78`). Wrapping
  arena internals therefore requires spilling any live caller-saved value across
  the `perf_start`/`perf_end` calls. (Cross-ref memory: *arena_alloc clobbers all
  caller-saved*.)

## 3. Design Overview

- **Wrapping shape:** at each region, emit `perf_start(nameLit)` before the region
  and `perf_end(nameLit)` after, gated by `perf_injection_enabled()`. Names are
  fixed literals emitted as string data objects.
- **Register preservation (the core risk):** the arena bodies are hand-emitted and
  hold live values in caller-saved registers across the region being wrapped.
  Because `perf_start`/`perf_end` clobber x0–x17, F must spill the region's live
  registers to stack slots around each pair (the *arena_alloc clobbers all
  caller-saved* pattern already used in the codebase). Prefer wrapping at region
  boundaries where the live set is smallest (e.g. wrap the whole `_mfb_arena_alloc`
  body at its entry/return, where the ABI already defines what is live) rather than
  mid-body.
- **Non-recursion guarantee:** since perf helpers never call the arena, wrapping
  arena code cannot recurse. F adds a test/assert that the perf symbols' relocation
  sets contain no arena symbol.
- **Blast radius (why F is last):** this edits the allocation hot path emitted for
  every program on the **macOS** backend. A mistake corrupts allocation. Verify on
  the macOS aarch64 host; Linux/Windows are untouched (no injection there) so a
  release-driven acceptance run on those platforms proves non-regression by
  construction (`diffs=0`).

**Design uncertainty** is low (B–E proved the helpers); **correctness risk** is
maximal and concentrated in register preservation on the arena hot path. That is
the entire reason for the letter ordering.

## 4. Detailed Design

- Add a small helper in the shared code layer that, given a region name and a
  closure emitting the region, brackets it with gated `perf_start`/`perf_end` plus
  the necessary spill/reload of the live caller-saved set. Apply it at each site
  the census identifies.
- Wrap at the coarsest safe boundary per region (helper entry/exit) to minimize
  the live set and the spill burden.
- Emit one string data object per region name.

## Compatibility / Format Impact

Debug-only: the arena hot path gains bracketed perf calls; `perf_done` prints
multiple region rows. Release byte-identical.

## Phases

> Checkboxes current in the same commit. Unticked = NOT DONE.

### Phase 1 — Census + names

- [ ] Enumerate the arena regions to wrap (command in Measured populations); write
      the final list + chosen literal names into this section. This fixes F's scope.

Acceptance: an itemized region→name table in this file, each row citing the emit
site it wraps.
Commit: —

### Phase 2 — Wrapping helper + register preservation

- [ ] Implement the gated bracket helper (spill/reload live caller-saved around
      `perf_start`/`perf_end`).
- [ ] Add the non-recursion check: assert the perf helpers' relocations reference
      no arena symbol.

Acceptance: assembles/encodes on host; `artifact-gate.sh target/release/mfb`
`diffs=0`; the non-recursion assert passes.
Commit: —

### Phase 3 — Apply to each region + host proof

- [ ] Wrap each census region with its named bracket (macOS backend only).
- [ ] Runtime-proof on the macOS host (debug): a program with known alloc/free
      behavior prints correct region rows; the program's own output and exit code
      are unchanged from a release run of the same program.

Acceptance: a **debug macOS** build prints correct arena-region timing rows on the
host with no allocation regression; a **release** build is byte-identical to
pre-plan-67 HEAD across all artifact-gate targets
(`scripts/artifact-gate.sh target/release/mfb` → `diffs=0`), and the full
acceptance suite is green when a release build drives it (plan-67-A). Linux/Windows
are unaffected by construction (no injection emitted there).
Commit: —

## Validation Plan

- Tests: runtime-proof programs with known allocation patterns; the non-recursion
  assert; existing allocation-heavy fixtures must not regress (release-driven).
- Coverage check: debug macOS `.ncode` shows the bracketed calls at each region;
  release `.ncode` (and all Linux/Windows `.ncode`) shows none — `diffs=0` proves
  this across every artifact-gate target.
- Runtime proof: multi-region table under a debug macOS build on the host.
- Doc sync: document the instrumented region set in the perf-helper spec section.
- Acceptance: `cargo test`; `scripts/artifact-gate.sh target/release/mfb`
  (`diffs=0`, all targets); full acceptance under release (plan-67-A) green.

## Open Decisions

- **Wrap granularity** — *(recommended)* wrap at helper entry/exit boundaries
  (smallest live set, least spill) vs. mid-body around sub-regions (more precise
  "post_alloc"/"post_free" timing but larger live set to preserve). Recommend
  starting at boundaries; refine to sub-regions only for the specific
  "random bits after alloc/free" spans the user named, and only if the live-set
  spill is provably correct. (§3)
  Decision: follow recommended
- **Future non-macOS support** — out of scope now (no-op stubs). When enabled
  later, each backend needs its own live-register-preservation review; do not
  assume the macOS spill set transfers. (§3)

## Corrections

<Filled in during execution.>

## Summary

The payoff letter and the dangerous one: it edits the macOS arena hot path, so it
lands last, behind all tests and every other letter. The single
property that keeps it safe — perf helpers never touch the arena — was established
in B and is re-checked here. All real risk is register preservation across the
injected calls; the census (Phase 1) is what turns the user's "etc." into a fixed,
reviewable scope.
