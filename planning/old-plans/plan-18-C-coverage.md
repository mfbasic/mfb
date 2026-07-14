# plan-18-C: Coverage instrumentation + HTML report

Last updated: 2026-07-02
Effort: large

Adds `mfb test --coverage`: per-statement instrumentation injected at AST→IR lowering (a counter
increment keyed to a compile-time `slot → (file, line)` map), a runtime counter array flushed at
`_mfb_shutdown`, and an HTML generator that produces `coverage.html` — a tree of the program's source
files with per-file line-coverage stats, where clicking a file shows its source, color-coded
covered/uncovered, with the lines of failed cases annotated.

Depends on: plan-18-A (mode plumbing, `--coverage` flag parsed) and plan-18-B (the driver that owns
the run and knows per-case failure lines). Read first: overview [plan-18-testing.md](plan-18-testing.md)
§ Layout/ABI and D4; the Explore finding that statement `line` survives only to AST→IR lowering
(overview §2).

## 1. Goal

- Line/statement coverage of the user's `.mfb` source (not the compiler, not the stdlib packages).
- `mfb test --coverage` builds an instrumented test binary; running it dumps hit counts; `mfb test`
  post-processes counts + the compile-time slot map into `coverage.html`.
- `coverage.html`: file tree with per-file `covered/total` line stats; per-file view = source,
  green (covered) / red (uncovered) per line, with failed-case lines annotated (from the driver's
  results).

### Non-goals

- **Branch coverage** — deferred to a future plan; V1 is line/statement only.
- **No effect on any non-`--coverage` build.** Normal `mfb build` and plain `mfb test` are unchanged
  and their goldens byte-identical.
- No coverage of stdlib/`src/builtins/*.mfb` package source — only the program's own files.

## 2. Current State

- Every AST `Statement` carries `line` (`src/ast/types.rs:449`), but it is dropped below the AST
  (only `IrOp::For` keeps `loc`) — so instrumentation must hook at AST→IR lowering, the last place the
  line exists (Explore finding, overview §2).
- Statement lowering: `src/ir/lower.rs` (`lower_statement` / `lower_statement_block`). `IrFunction::file`
  (`src/ir/mod.rs:93`) gives the source file for slot keying.
- `_mfb_shutdown` teardown hook (memory: shutdown-and-signal-handlers) — where counts flush at exit.
- The driver (plan-18-B) holds each failed case's `(file, line)` — the source of the failed-line
  annotations.
- `src/doc.rs` (HTML for `mfb doc`) is the precedent for compiler-side HTML generation.

## 3. Design

- **Instrumentation (compile time, `--coverage` only)**: in `lower_statement`, when the coverage flag
  is set and the statement belongs to a user source file, allocate a slot `s`, record
  `slot_map[s] = (file, line, relpath)`, and prepend a counter-increment IR op writing
  `__mfb_cov[s] += 1`. The increment is an ordinary memory op that flows through IR→NIR→codegen
  unchanged — **no new fields on any IR/NIR/plan op**, so normal-build goldens are untouched.
- **Counter storage**: a BSS array `__mfb_cov[N]` (N = slot count) in the `--coverage` plan only.
- **Slot map delivery (D4)**: the compiler writes `slot_map` to a sidecar (e.g.
  `<out>.covmap.json`) during the instrumented build; the runtime writes raw `__mfb_cov` to a known
  file (e.g. `<out>.covdata`) at `_mfb_shutdown`; `mfb test` reads both.
- **Runtime dump**: extend `_mfb_shutdown` to write the counter array to `<out>.covdata` (only present
  in `--coverage` binaries).
- **HTML generation (`mfb test` side)**: a new `src/coverage.rs` (modeled on `src/doc.rs`) folds
  counts by file, builds the file tree with per-file stats, and renders per-file annotated source
  (covered/uncovered coloring from `__mfb_cov`; failed-case line markers from the driver's results).
  Emits `coverage.html`.

## Phases

### Phase 1 — Lower-time instrumentation + slot map

- [ ] Thread the `--coverage` flag from CLI (plan-18-A parsed it) into lowering.
- [ ] In `src/ir/lower.rs` `lower_statement`, emit a `__mfb_cov[s] += 1` op per user statement and
      populate `slot_map[s] = (file, line, relpath)` (keyed via `IrFunction::file`).
- [ ] Allocate the `__mfb_cov` BSS array (size = slot count) in the `--coverage` plan.
- [ ] Write `slot_map` to `<out>.covmap.json` at build time.
- [ ] Tests: assert a `--coverage` build produces a covmap with one slot per user statement, and — the
      critical guard — assert a **non-`--coverage`** build is byte-identical to today (no `__mfb_cov`,
      no covmap).

Acceptance: instrumented build emits the covmap + counter array; non-instrumented build unchanged
(byte-identical golden). Commit: —

### Phase 2 — Runtime count dump at shutdown

- [ ] Extend `_mfb_shutdown` to write `__mfb_cov` to `<out>.covdata` (guarded to `--coverage`
      binaries).
- [ ] Tests: run an instrumented fixture; assert `.covdata` counts match the statements actually
      executed (e.g. a branch not taken → its slot is 0).

Acceptance: after a run, `.covdata` reflects real execution (taken lines > 0, untaken lines = 0).
Commit: —

### Phase 3 — `coverage.html` generator (file tree + annotated source)

- [ ] `src/coverage.rs`: fold `.covdata` by file via `.covmap.json`; build the file tree with
      per-file `covered/total` stats; render per-file annotated, color-coded source; annotate
      failed-case lines from the driver's results. Model on `src/doc.rs`.
- [ ] Wire `mfb test --coverage` to invoke the generator after the run and write `coverage.html`.
- [ ] Tests: a fixture with deliberately unexercised lines and one failing case → assert
      `coverage.html` marks exactly those lines uncovered and annotates the failed line.

Acceptance: `mfb test --coverage fixture.mfb` writes `coverage.html` whose tree stats and per-line
coloring match the known-executed set, with the failed case's line annotated. Commit: —

## Validation Plan

- Byte-identical guard: non-`--coverage` builds unchanged (Phase 1 acceptance) — this is the gate that
  keeps coverage from touching normal codegen.
- Execution proof: `.covdata` reflects real taken/untaken lines (Phase 2).
- HTML proof: coloring + stats + failed-line annotation match a known fixture (Phase 3).
- Doc sync: document `mfb test --coverage` and the report in `src/docs/spec/**` (tooling/language as
  appropriate).
- Acceptance: `scripts/test-accept.sh` green.

## Open Decisions

- D4 (sidecar map + runtime count file vs. embedding the map) — resolved to sidecar per overview.

## Summary

Additive and fully mode-gated. The single risk is the instrumentation leaking into non-`--coverage`
builds — pinned by the Phase-1 byte-identical guard. By hooking at AST→IR lowering (where statement
`line` still lives) and emitting only an ordinary counter-increment op, no IR/NIR/plan op definitions
change, so existing goldens and ABI are untouched. Branch coverage is explicitly left for a follow-up.
