# plan-125-L: spec iteration 2, batch 3 — linker, threading, tooling, app, package-manager, diagnostics (51 files)

Last updated: 2026-09-04
Effort: x-large (1d–3d)
Depends on: plan-125-K (iteration 2 is one ordered pass).

Landing unit: **each Phase below is independently landable and gets its own
commit.** The letter totals x-large; it is never landed as one change, and a
session that lands one phase and stops has left the tree consistent.

Iteration 2, final spec batch. The unit is one file. Six packages, 51 files,
9,835 lines, 795 citations — the platform, tooling and protocol surfaces:

- **`linker`** (14 files) — how native executables are produced.
- **`threading`** (13) — the thread and concurrency contracts.
- **`tooling`** (9) — manifest, source selection, lockfile, audit, fmt, doc,
  CLI. **Directly falsifiable**: every claim is checkable by running `mfb`.
- **`app`** (7) — the `-app` GUI runtime, macOS/Linux/term backends, canvas.
- **`package-manager`** (5) — registry, keys, signing. The highest citation
  density in the spec (193 citations over 5 files) and the one with a
  security consequence if it is wrong.
- **`diagnostics`** (3) — the rule and error-code registries. `02_error-codes.md`
  is **build input**.

References:

- plan-125-A §3.2/§3.3/§4.2/§4.3/§5 (the spec iteration-2 prompt).
- plan-125-J §3 — the citation-first four-bucket triage rule, unchanged.
- `.ai/spec-content.md`; `.ai/specifications.md`.
- `.ai/build-tooling.md` — foil and cross-check for `tooling` and `linker`
  (rustfmt/clippy policy, cross-compile and vendor rebuild mechanics).
- `.ai/canvas-threading.md` — the three-thread model, the scene ring,
  per-thread arena state, the resize handshake, the closed-flag texture-free
  rule: the internals `app`'s canvas topic specifies, and the destination of
  the canvas cuts letter E made.
- `.ai/net-tls.md` — cross-check for `package-manager`'s transport claims.
- Memory `ci-jobs-run-on-linux-debug-not-mac-release`,
  `three-linux-boxes-have-cargo-probe-both-paths`,
  `linux-cargo-test-axis-is-one-slow-box` — CI/platform realities `tooling`
  may describe; a claim about where things run is checkable.
- Memory `tls-trust-relaxation-per-backend`,
  `openssl-cli-version-skew-local-vs-ci` — `package-manager` transport
  specifics.
- Memory `arena-state-is-per-thread`,
  `spawned-thread-entry-must-save-callee-saved`,
  `a-mid-frame-race-needs-both-sides-slowed` — `threading` and `app`
  invariants, each checkable against the code.

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-K complete | `grep -c '^- \[ \]' planning/plan-125-K-spec-iter2-batch2.md` → `0` | — |
| citation surface still clean | `./scripts/spec-census.sh --citations` → 0 `MISS-*` | — |
| the in-flight `term` work has landed | `git status --porcelain src/docs/spec/app` → empty | — |

## 1. Goal

- **All 51 files** through my per-file pass, one `codex exec` each, and apply.
- **Every `tooling` claim verified by running the command it describes.** This
  package is unusual in the spec: nothing about it needs to be inferred from
  code, because every claim is directly executable. A `tooling` claim that was
  not run is not verified.
- **`diagnostics/02_error-codes.md`'s Constant Registry reconciled against the
  generated constants**, with `cargo build` + `cargo test errorcode`
  (`table_matches_registry`) in the same commit as any edit to it.
- **`app`'s canvas topic receives the cuts letter E made** — the
  `belongs-in-spec` entries attributed to `src/docs/spec/app/06_canvas.md` are
  covered here, and `04_term-backend.md` is reconciled against the term work
  that landed.
- **`package-manager`'s signing and transport claims verified against the
  code**, with every cryptographic claim checked against its standard rather
  than against the implementation agreeing with itself.
- Four-bucket triage with per-file bucket counts, as in J and K.
- Every finding has a verdict; every rejection a disproving command.
- `--reconcile` exits 0 over the 51-unit list; `--citations`/`--links` clean
  **across the entire spec surface**.
- `cargo build`, `cargo test --bin mfb spec`, and `cargo test errorcode` (if
  the registry table changed) green.
- **Spec iteration 2 is complete**: 3 files in A's pilot + 47 (J) + 45 (K) +
  51 (L) = **146**, reconciled against the census.

### Non-goals (explicit constraints)

- **No cross-file reconciliation** (letter M owns it).
- The memory-vocabulary ban does not apply on this surface.
- Compiler, linker, CLI and registry behavior are not changed; a defect goes
  through `write-bug`.
- **No change to the error-code registry's values**, only to the table's
  description of them — a value change is a compiler change with its own gates
  and is out of scope here.
- No wording churn on a claim that survives verification.

## 2. Current State

All six packages have been read as wholes (letter I) and their citations
resolve (tooling 4, app 3, linker 2, threading 1, diagnostics 1 broken at
baseline, all repaired there; `package-manager` had 0). No claim in any of
them has been verified against the code or the CLI.

### Measured populations

| Package | Files | Lines | Citations | Citations/file | Broken at baseline |
|---|---|---|---|---|---|
| linker | 14 | 1,837 | 148 | 11 | 2 |
| threading | 13 | 990 | 31 | 2 | 1 |
| tooling | 9 | 2,553 | 241 | 27 | 4 |
| app | 7 | 1,971 | 149 | 21 | 3 |
| package-manager | 5 | 1,759 | 193 | **39** | 0 |
| diagnostics | 3 | 773 | 33 | 11 | 1 |
| **total** | **51** | **9,835** | **795** | 16 | 11 |

Commands: `find src/docs/spec/<pkg> -name '*.md' | wc -l`;
`cat src/docs/spec/<pkg>/*.md | wc -l`;
`grep -rhoE '\[\[[^]]+\]\]' src/docs/spec/<pkg> --include='*.md' | wc -l`.

| What | Count | Command |
|---|---|---|
| units in this letter | **51** | sum of the table |
| `threading` citation density | **2 per file** — the lowest in the spec | 31/13 |
| `package-manager` citation density | **39 per file** — the highest in the spec | 193/5 |
| `app` lines at plan-writing | 1,971, against 1,923 in an earlier run | uncommitted `src/docs/spec/app/04_term-backend.md` edits — re-measure at kickoff |

### Verified properties

- **`threading` is the least-cited package in the spec** (2 citations per
  file, against a spec-wide 13) — VERIFIED by measurement. Its claims are
  therefore the least checkable-by-reading in the whole surface, and the most
  likely to be verified by reasoning rather than by evidence. Phase 2 budgets
  for verification by probe, and adding citations is an explicit output.
- **`package-manager` is the most-cited** (39 per file) — VERIFIED by the same
  measurement. Dense citation is not the same as correct: 5 files carrying 193
  citations means the per-citation reading is where this package's time goes.
- **`tooling` is uniquely cheap to verify** — VERIFIED by inspection of its
  subject matter (manifest, lockfile, audit, fmt, doc, CLI): every claim is
  the output of a command that can be run. No other spec package has an
  oracle this direct.
- **`app` is mid-change at plan-writing** — VERIFIED by `git status`; the
  prerequisite gate covers it.
- UNVERIFIED: the accuracy of any claim in the batch.

## 3. Design Overview

Per file: my pass → one `codex exec` → apply, `N` concurrency, main thread
sole writer. plan-125-J's four-bucket triage rule applies unchanged.

**Order:** `diagnostics` (3, and the one with a build-input gate — do it first
while attention is highest) → `tooling` (9, directly executable, calibrates
fastest) → `package-manager` (5, densest) → `app` (7) → `threading` (13, the
least-cited and so the slowest per claim) → `linker` (14).

**Risk concentration:**
- **`threading` verified by plausibility.** With 2 citations per file, the
  path of least resistance is to read a claim, find it reasonable, and move
  on. Held by requiring bucket-2 claims to be *either* cited *or* probed, and
  by recording the bucket counts per file — a `threading` file that is all
  bucket 1 with 2 citations is arithmetically impossible and the count makes
  that visible.
- **Touching the error-code registry.** It is build input; a careless edit
  breaks the build or trips `table_matches_registry`. The gate is
  `cargo build` + `cargo test errorcode` in the same commit, and value changes
  are out of scope entirely.
- **`package-manager` crypto claims verified against the implementation.**
  Memory `wrong-oracle-is-ratified-not-caught` and
  `hpke-official-vectors-cfrg-json`: where a standard exists, the standard is
  the oracle, and the vectors are fetched, not recalled.
- **`tooling` claims about CI and remote machines.** Memory records that CI is
  RELEASE on five platforms and that the Linux `cargo test` axis is one
  1-core box; a `tooling` claim about where things run is checkable and worth
  checking, because it is exactly the kind of claim that ages silently.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; `- [x] ~~text~~ — moot: <evidence>`
> rather than deleting; fill `Commit:` on landing. **Unticked means NOT DONE.**

### Phase 1 — diagnostics and tooling (12 units)

- [ ] `diagnostics` 3 files: reconcile the Constant Registry against the
      generated constants; **any edit gets `cargo build` +
      `cargo test errorcode` in the same commit**; reconcile the rule registry
      against `src/rules/`.
- [ ] `tooling` 9 files: **every claim verified by running the command**;
      the ledger records the command per claim.
- [ ] Four-bucket triage; per-file bucket counts recorded.

Acceptance: 12 units `exit 0`; every `tooling` claim in the ledger names the
command that verified it; `cargo build`, `cargo test --bin mfb spec` and (if
the registry table changed) `cargo test errorcode` green; `--citations`/
`--links` clean for both packages.
Commit: —

### Phase 2 — package-manager and app (12 units)

- [ ] `package-manager` 5 files: every signing/key/transport claim verified at
      its cited symbol; every cryptographic claim checked against its
      published standard, with the source recorded.
- [ ] `app` 7 files: receive letter E's canvas cuts into `06_canvas.md`;
      reconcile `04_term-backend.md` against the landed term work; verify the
      macOS/Linux runtime claims against `src/target/**`.
- [ ] Four-bucket triage; bucket counts recorded.

Acceptance: 12 units `exit 0`; every `package-manager` crypto claim names an
oracle that is not the implementation; every `belongs-in-spec` entry
attributed to `app` is covered, with the topic and section named;
`--citations`/`--links` clean; cargo gates green.
Commit: —

### Phase 3 — threading and linker (27 units)

- [ ] `threading` 13 files: every bucket-2 claim either given a grep-confirmed
      citation or verified by probe; record which, per claim. Adding citations
      to the least-cited package in the spec is an explicit output of this
      phase, not a side effect.
- [ ] `linker` 14 files: every claim verified at its cited symbol or by
      producing and inspecting a real executable.
- [ ] Four-bucket triage; bucket counts recorded.

Acceptance: 27 units `exit 0`; `threading`'s citation count is recorded before
and after, with the increase attributable to specific claims; `--citations`
and `--links` clean **across the whole spec surface**; cargo gates green.
Commit: —

### Phase 4 — spec iteration 2 completeness

- [ ] Reconcile the union of A's pilot (3), J (47), K (45) and L (51) against
      the census: **146 files, 0 unaccounted**.
- [ ] Reconcile the union of all four letters' bucket tables against the file
      list: 146 files, 0 unaccounted.
- [ ] Run the whole-surface sweeps: `--fill`, `--citations` (0 `MISS-*`),
      `--links` (clean), `--render` (no leaked `[[` in any package).
- [ ] Record the spec-wide citation count before and after iteration 2.

Acceptance: the reconciliation prints 146 and 0; all four sweeps at target;
this letter records the totals as the entry state for letter M.
Commit: —

## Validation Plan

- Tests: `cargo build` and `cargo test --bin mfb spec` at the end of every
  phase; `cargo test errorcode` whenever `diagnostics/02_error-codes.md`
  changes. A `write-bug` fix landing here brings its own gates.
- Coverage check: `--reconcile` over the 51-unit list, then the Phase 4
  cross-letter reconciliation to 146.
- Runtime proof: every `tooling` command run; a real executable produced for
  `linker`; `mfb spec <pkg> --all` renders for all six with no leaked `[[`.
- Doc sync: disagreements with `.ai/build-tooling.md`,
  `.ai/canvas-threading.md` or `.ai/net-tls.md` fixed in both surfaces in the
  same commit.
- Acceptance: 0 `MISS-*` surface-wide, `--links` clean, cargo gates green,
  `--reconcile` 0, 146 files reconciled.

## Open Decisions

- **How many citations should `threading` gain?** — Recommend: enough that
  every non-obvious claim is locatable, with no target number. A count target
  produces decorative citations, which is how a surface ends up with 1,970
  citations and 61 of them broken.
- **Does `tooling` specify CLI output verbatim?** — Recommend specifying the
  contract (fields, exit codes, ordering, stability guarantees) and not the
  exact rendering, which changes for good reasons and would make the spec
  wrong on every cosmetic change. Where the exact rendering *is* the contract
  (machine-readable output), say so explicitly.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

Two packages sit at opposite ends of the same risk. `tooling` is the cheapest
verification in the entire plan — every claim is a command away from proof —
and `threading` is the most expensive, at 2 citations per file, where a claim
can only be checked by probing the runtime. The bucket counts are how this
letter proves it did the expensive one honestly instead of reading it and
nodding.
