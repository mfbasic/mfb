# plan-67-A: Drive the golden/acceptance harness from a release build

Last updated: 2026-07-26
Overall Effort (AI): x-large   (section A only — sizes the whole plan-67 feature)
Effort (Human): medium
Effort (AI): small
Depends on: nothing
Produces: a golden/acceptance test path that always runs a **release-built**
`mfb` compiler (`debug_assertions` off), so the debug-gated perf injection added
in B–F never fires during golden generation. This is the safety foundation every
later letter relies on.

Plan-67 adds internal runtime performance tracking (`perf_init` / `perf_start` /
`perf_end` / `perf_done`) that the compiler injects into generated code **only
when the compiler itself is built in debug mode** (`cfg!(debug_assertions)`). The
four helpers maintain two runtime-internal hashes in system memory (outside the
arena) — hash **B** (name → start-nanos) and hash **A** (name → sample array) —
and print a `name count avg median min max sum` table as the last thing before
process exit. For now only arena-related code regions are instrumented.

This sub-plan changes **no runtime behavior**. It exists because the acceptance
and artifact-gate suites currently drive the **debug** compiler
(`CARGO_BIN_EXE_mfb` resolves to `target/debug/mfb`; fallback literal
`target/debug/mfb` at `tests/common/mod.rs:580`) and fold **stderr into the
diffed golden** (`test-accept.sh:269` `2>&1`, `test-accept.sh:376`
`} >"$log_path" 2>&1`). If B lands debug-gated injection while the suite still
runs the debug binary, every `build.log` golden gains a perf table and every
`.ncode` byte-identity golden gains injected calls — the suite goes red
wholesale. Switching the harness to a release binary removes that collision at
the root: release has `debug_assertions` off, so injection is inert there.

References:

- `.ai/compiler.md` — runtime completion gate, validation & function tests,
  register lifetimes. Read before touching any B–F codegen.
- `.ai/remote_systems.md` — remote build/run boxes (only box 2229 has a Rust
  toolchain; Linux/riscv verification ships cross-compiled binaries).
- Memory: *Fast codegen gate*, *Acceptance golden harness*, *Build time baseline*
  — artifact-baseline already builds `--release`; full acceptance is minutes, not
  the "20+ min" VM-contention myth.

## Prerequisites

These are a precondition on the **whole plan-67 feature** (A–F). Every other
letter points here rather than restating them.

| Must be true | Command | Status |
|---|---|---|
| Working tree clean / on a branch you may commit to | `git status --porcelain` (empty) | UNVERIFIED |
| Full test suite green at HEAD (baseline before any change) | `cargo test` → `0 failed` | UNVERIFIED |
| Artifact byte-identity gate green at HEAD with a **release** exe | `cargo build --release && scripts/artifact-gate.sh target/release/mfb` → `diffs=0` | UNVERIFIED |
| No pre-existing build-mode-gated codegen to conflict with | `rg -n 'debug_assertions' src/target src/arch` shows only internal asserts (`codegen_utils.rs:446,755`, `regalloc/linear_scan.rs:338`, `riscv64/v128.rs:248`, `rules/mod.rs:205`) | MET (verified during planning) |

Everything in A–F is written against the world where these hold. There are no
fallbacks for the world where they don't.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before continuing, and again
> before deciding to stop. If you stop, report the status of *all* prerequisites,
> not just the row that blocked you.

## Dependency graph  (whole feature)

```
A ← nothing        (this sub-plan — release harness; must land first)
B ← A              (perf family scaffold, gating, region, entry/exit injection)
C ← B              (name-keyed table B, linear scan + perf_start + print B rows)
D ← C              (table A + growable samples + perf_end + print count)
E ← D              (full statistics: avg / median / min / max / sum)
F ← D              (arena-region instrumentation — needs start/end from D, not
                    the stats from E; scheduled last for blast radius)
```

Execution: topological order A → B → C → D → E → F, re-checking each letter's
stated preconditions before it starts.

## 1. Goal

- The golden/acceptance/artifact-gate test path invokes a **release-built** `mfb`
  binary, and the full suite is green when driven that way, with **zero golden
  changes** from the switch itself.

### Non-goals (explicit constraints)

- No change to compiler output, runtime behavior, or any golden **content**. This
  letter only changes *which build of the compiler* the harness runs.
- Do not delete or weaken any test. Pure-Rust library unit tests (`cargo test`
  unit tests that do **not** shell out to the compiler binary) keep running under
  debug so the compiler's internal `debug_assertions` invariant checks
  (`codegen_utils.rs:446` etc.) still execute.
- No perf-tracking code is added in this letter.

## 2. Current State

- Rust integration tests shell out to `env!("CARGO_BIN_EXE_mfb")`
  (`tests/common/mod.rs:28,50,74`), which Cargo points at the **debug** binary
  during a normal `cargo test`; the helper `mfb_exe()` falls back to the literal
  `"target/debug/mfb"` (`tests/common/mod.rs:580`).
- `scripts/test-accept.sh` takes the compiler exe as `$1` and captures combined
  stdout+stderr into the diffed `build.log` golden (`:269` `2>&1`, `:376`
  `} >"$log_path" 2>&1`).
- `scripts/artifact-gate.sh` and `scripts/sync-goldens.sh` also take the exe as an
  argument; artifact baselines are already generated with `target/release/mfb`
  (memory: *Fast codegen gate*, *Linux boxes have no Rust toolchain*).

So the shell harnesses already accept whatever exe they are handed — the only
path hard-wired to debug is the Cargo-driven integration-test layer in
`tests/common/mod.rs`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Integration-test call sites using `CARGO_BIN_EXE_mfb` | UNMEASURED — run first task | `rg -n 'CARGO_BIN_EXE_mfb' tests/` |
| Shell harnesses taking an exe arg (already release-capable) | 3 | `test-accept.sh`, `artifact-gate.sh`, `sync-goldens.sh` (each reads exe from `$1`) |

### Verified properties

- **stderr is folded into the golden** — VERIFIED by reading `test-accept.sh:269`
  and `:376` (`2>&1`). This is why routing the perf table to stderr does not by
  itself protect the goldens, and why the harness must run a build where
  injection is off entirely.
- **release has `debug_assertions` off** — standard Cargo profile behavior;
  confirmed there is no override in `Cargo.toml` profiles (task: `rg -n
  'debug-assertions' Cargo.toml` should show nothing forcing it on in release).

## 3. Design Overview

Two mechanisms, pick per the Open Decision below:

1. **Prefer a release binary in `mfb_exe()`** — change the fallback/selection so
   the acceptance path resolves `target/release/mfb`, building it first if
   absent. Keeps `cargo test` as the entry command; only the acceptance layer
   switches build. Lowest blast radius; recommended.
2. **Run the acceptance suite under `cargo test --release`** — then
   `CARGO_BIN_EXE_mfb` already points at the release binary. Simpler code change,
   but flips the *whole* test binary (including pure-Rust unit tests) to release,
   dropping the compiler's internal `debug_assertions` coverage during those runs.

**Correctness risk** is low — no output changes. The only real risk is a *hidden*
golden that was silently depending on a debug-only diagnostic; the acceptance run
after the switch is what falsifies that.

**Design uncertainty** is whether any committed golden currently reflects
debug-binary output (it should not, since baselines are release). Phase 1 is the
experiment that settles it: run the suite both ways and diff.

## Phases

> Keep checkboxes current in the same commit as the work. An unticked box means
> NOT DONE.

### Phase 1 — Measure the current gap

Cheap experiment that falsifies the premise "committed goldens are release-built".

- [ ] Enumerate the debug-wired sites: `rg -n 'CARGO_BIN_EXE_mfb|target/debug'
      tests/` — record the count in Measured populations above.
- [ ] Build both compilers: `cargo build && cargo build --release`.
- [ ] Run acceptance with each and diff: `scripts/test-accept.sh target/debug/mfb
      /tmp/acc-dbg` vs `scripts/test-accept.sh target/release/mfb /tmp/acc-rel`;
      compare pass/fail sets. Any golden that only passes under debug is a latent
      dependency to record here before proceeding.

Acceptance: a written finding — either "all goldens pass identically under
release" (expected) or an itemized list of goldens that diverge, each triaged.
Commit: —

### Phase 2 — Switch the acceptance path to release

- [ ] Implement the chosen mechanism (Open Decision). If option 1: make
      `mfb_exe()` (`tests/common/mod.rs:580`) resolve `target/release/mfb` for the
      compiler-driving helpers `build_ok`/run helpers (`:28,:50,:74`), building it
      on demand, and document why. Keep pure-Rust unit tests on the default
      profile.
- [ ] Update `scripts/sync-goldens.sh` / `scripts/test-accept.sh` / CI invocations
      and any docs so the canonical golden-producing command passes
      `target/release/mfb`. If they already do, note it moot with evidence.
- [ ] Add a one-line comment at each switched site explaining that release is
      load-bearing for plan-67's debug-gated injection.

Acceptance: `cargo build --release` then the full acceptance suite driven by
`target/release/mfb` is green with **zero** golden content changes
(`git status --porcelain tests/` empty after the run); `cargo test` remains green.
Commit: —

## Validation Plan

- Tests: no new tests; this is a harness change. The existing acceptance suite is
  the test.
- Coverage check: confirm the acceptance harness now executes the release binary
  (`ps`/log line shows `target/release/mfb`), not debug.
- Runtime proof: full acceptance run green under release; `git diff --stat tests/`
  empty.
- Doc sync: update any contributor doc that names the golden-regen command
  (`.ai/compiler.md`, memory *Acceptance golden harness* if it hard-codes debug).
- Acceptance: `cargo test` + `cargo build --release && scripts/artifact-gate.sh
  target/release/mfb` → `diffs=0`.

## Open Decisions

- **Switch mechanism** — *(recommended)* option 1 (resolve `target/release/mfb`
  inside `mfb_exe()`, keep unit tests on debug) vs. option 2 (`cargo test
  --release` for the acceptance layer). Recommend option 1: it preserves the
  compiler's internal `debug_assertions` invariant checks during unit tests while
  still driving acceptance from release. (§3)
  Decision: resolve `target/release/mfb` inside `mfb_exe()`, keep unit tests on debug

## Corrections

<Filled in during execution.>

## Summary

Zero-behavior-change foundation. The only risk is discovering a golden that
secretly depended on debug output; Phase 1 surfaces it before Phase 2 relies on
its absence. Once release drives the golden path, B–F can gate on
`cfg!(debug_assertions)` safely.
