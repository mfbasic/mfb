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

- [x] Census (`rg ... src/target/shared/code/arena.rs`). The whole-helper arena
      boundaries — the Open Decision's recommended wrap points — are:

      | region name | emit site | ABI | wrapped |
      |---|---|---|---|
      | `mfb_alloc` | `lower_arena_alloc` (`_mfb_arena_alloc`) | args `ARG[0]`=size, `ARG[1]`=align; result `RET[0]`=tag, `RET[1]`=ptr | ✅ |
      | `mfb_free`  | `lower_arena_free` (`_mfb_arena_free`) | args `ARG[0]`=ptr, `ARG[1]`=size; returns Nothing | ✅ |
      | `system_alloc` | inline `emit_arena_map` (mmap) inside `lower_arena_alloc` grow path | — | deferred (inline sub-region) |
      | `post_alloc` / `post_free` | inline fill-RNG region (`arena_fill_random`) inside alloc/free | — | deferred (inline sub-region) |

      Both wrapped helpers are **single-exit** (`arena_alloc_ret` / `arena_free_done`
      — all paths route through one `return_()`), which is what makes the
      one-`perf_start`/one-`perf_end` boundary wrap correct. `system_alloc` /
      `post_alloc` / `post_free` are inline sub-regions; per the Open Decision
      ("start at boundaries; refine to sub-regions only if the live-set spill is
      provably correct") they are left for a follow-up refinement, with
      `emit_perf_arena_call` ready to bracket them.

Acceptance: the region→name table above. Commit: 8c547c9c6

### Phase 2 — Wrapping helper + register preservation

- [x] Implemented the gated bracket helper `emit_perf_arena_call` (`arena.rs`):
      loads the region name into the arg register and `bl`s the perf helper. The
      spill/reload of live caller-saved values is handled **by the register
      allocator** — the injected `bl` carries the standard call-clobber mask, so any
      live vreg is spilled across it automatically (the same mechanism arena_alloc
      already uses for its `arena_fill_random` grow call). `perf_start` goes in
      after the helper captures its args into vregs; `perf_end` at the single exit,
      with arena_alloc's result (`RET[0]`/`RET[1]`) saved into vregs and restored
      across the perf `bl`. All gated on `perf_arena_enabled()` (debug + macOS).
- [x] Non-recursion check: `perf_helpers_reference_no_arena_symbol` (`perf.rs`
      tests) asserts the module names no `ARENA_*_SYMBOL` (the perf region rides the
      `emit_arena_map` mmap seam / `clock_gettime`, never `_mfb_arena_alloc`), so a
      perf helper can never `bl` into the arena.

Acceptance: `cargo build -p mfb` clean; the non-recursion test passes; release
byte-identity verified below.
Commit: 8c547c9c6

### Phase 3 — Apply to each region + host proof

- [x] Wrapped `mfb_alloc` (`lower_arena_alloc`) and `mfb_free` (`lower_arena_free`),
      macOS backend only. Emitted their name data objects (`"mfb_alloc"`,
      `"mfb_free"`).
- [x] Runtime proof on the macOS host (debug): an alloc/free-heavy program
      (`/tmp/p67free`, a 200-iteration string-concat) prints
      `mfb_alloc 7 428 0 0 3000 3000` / `mfb_free 5 0 0 0 0 0` / `program 1 …` —
      correct per-region rows with plausible counts — and its own output (`len 200`)
      + exit code (0) are unchanged, proving the result save/restore preserves the
      allocation. This ALSO validates plan-67-E's stats over real multi-sample data.

Acceptance: **debug macOS** build prints correct arena-region rows with no
allocation regression (program output + exit intact); **release** build
byte-identical — `artifact-gate: … 0 diff(s)` (all targets); `cargo test`
310+20 passed 0 failed. Linux/Windows unaffected by construction (`perf_arena_enabled`
false → the gated blocks emit nothing → identical instruction list).
Commit: 8c547c9c6

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

- **In-body boundary wrap, not a spill-closure helper.** §4 imagined a helper
  taking "a closure emitting the region" plus explicit spill/reload. In practice
  both wrapped helpers are already **single-exit** vreg bodies, so the wrap is two
  gated insertions — `perf_start` right after the args are captured into vregs,
  `perf_end` at the one exit — and the **register allocator does the spilling**: the
  injected `bl` carries the call-clobber mask, so every live vreg is spilled across
  it automatically (exactly as arena_alloc already handles its `arena_fill_random`
  grow call). No manual caller-saved spill list is needed. arena_alloc's result
  (`RET[0]` tag / `RET[1]` ptr) is the only value not already in a vreg at the exit,
  so it is explicitly saved into vregs and restored around the `perf_end` `bl`.
- **Scope = the two helper boundaries (Open Decision "follow recommended").** The
  Open Decision recommended starting at helper entry/exit boundaries and refining to
  the inline `system_alloc` / `post_alloc` / `post_free` sub-regions "only if the
  live-set spill is provably correct." `mfb_alloc` + `mfb_free` are those boundaries
  and are done + proven. The inline sub-regions are a documented follow-up
  (`emit_perf_arena_call` is ready to bracket them); they are the only part of the
  user's named region list not yet wrapped.
- **Non-recursion via a source-structural test.** The macOS `CodegenPlatform`
  (`struct Platform`) is private, so a lowering-based reloc-scan unit test would
  require exposing internals. The invariant is instead enforced structurally
  (`perf.rs` names no `ARENA_*_SYMBOL`; its memory is the `emit_arena_map` mmap
  syscall, never `_mfb_arena_alloc`) and confirmed behaviorally by the runtime proof
  (an instrumented program runs to completion — a recursion would blow the stack).

## Summary

The payoff letter and the dangerous one: it edits the macOS arena hot path, so it
lands last, behind all tests and every other letter. The single
property that keeps it safe — perf helpers never touch the arena — was established
in B and is re-checked here. All real risk is register preservation across the
injected calls; the census (Phase 1) is what turns the user's "etc." into a fixed,
reviewable scope.
