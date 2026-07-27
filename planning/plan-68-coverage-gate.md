# Coverage Gate Restoration Plan (overview)

Last updated: 2026-07-27
Overall Effort (Human): x-large (1d–3d)
Overall Effort (AI): large (3h–1d)

The per-file coverage gate (`scripts/coverage-check.sh`, mirrored by the CI
`coverage` job's per-file step and `--fail-under-lines 95`) is currently RED: 55
in-scope source files sit below the 95% line-coverage floor set by plan-12. This
feature restores the gate to green — every non-excepted in-scope file back at
≥95% — by two distinct kinds of work the triage in sub-plan A separates:

1. **Exception-list rot repair.** Two refactors moved integration-only,
   subprocess/network-driven orchestration to new paths the exception list
   (`scripts/coverage-exceptions.txt`) still names by their *old* paths, so that
   code now (correctly) fails the per-file gate under names nobody excepted.
   `bug-327 T2-10` (19a95d448) split `src/cli/build.rs` → `src/cli/build/{mod,
   native_libs,options,packages,resources,signing,test_mode}.rs`; `bug-340 B1`
   (c565abda8) split `src/main.rs` → `src/cli/help.rs` + `src/cli/dispatch.rs`.
   The excepted-then-moved orchestration must be re-triaged and re-excepted at
   its new path; the genuinely unit-coverable logic that rode along gets tests.
2. **Genuine coverage backfill.** Front-end, IR, and leaf modules that grew new
   code without matching tests and drifted below 95%. These get `#[cfg(test)]`
   unit tests exercising the uncovered branches.

The single checkable outcome: `sh scripts/coverage-check.sh` prints
`All non-excepted files >= 95% line coverage.` and exits 0, and
`cargo llvm-cov report … --fail-under-lines 95` passes — with every exception
carrying a fresh, specific in-list justification for the syscall / network /
subprocess / GUI boundary that blocks unit coverage.

References:

- `scripts/coverage-check.sh` — the per-file gate (FLOOR=95).
- `scripts/coverage.sh`, `scripts/coverage-common.sh` — how the profile and the
  `IGNORE` denominator regex + `PKG_FLAGS` are produced.
- `scripts/coverage-exceptions.txt` — the documented per-file exception list; the
  format and the standing rule ("each entry must name the specific
  syscall/network/subprocess boundary that blocks unit coverage").
- `.github/workflows/coverage.yml` — the CI job this plan must turn green.
- `planning/old-plans/plan-12-*.md` — the plan that established the 95% bar, the
  tiered strategy (A tooling/frontend, B builtins/codegen, C OS, D CLI/repo), and
  the exception mechanism. The precedent this plan mirrors.
- `AGENTS.md` "Never edit a test/golden to pass" — coverage tests must exercise
  real behavior; an exception entry is not a way to dodge a coverable gap.

## Prerequisites

Stated once here; every lettered sub-plan points back to this section.

| Must be true | Command | Status |
|---|---|---|
| `cargo-llvm-cov` is installed (the gate's engine) | `cargo llvm-cov --version` | UNVERIFIED — check before starting A |
| The test suite is green at HEAD (a red suite zeroes a binary's whole profile and produces phantom coverage gaps — see `scripts/coverage.sh` comment) | `cargo test` → `0 failed` | UNVERIFIED — check before starting A |
| A full instrumented coverage report can be produced locally on this host (macOS aarch64) | `sh scripts/coverage.sh` exits 0 and writes `target/coverage/coverage.json` | UNVERIFIED — this run IS sub-plan A's first task |

Everything below is written against the world where these hold. There is no
fallback path for a missing coverage toolchain or a red suite — fix those first.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before continuing, and again
> before deciding to stop. If you stop, report the status of *all* prerequisites.

## Dependency graph

```
A ← nothing                                  (triage + exception repair + fresh line-level report)
B ← A;  C ← A;  D ← A;  E ← A;  F ← A;  G ← A;  H ← A
```

`A` is the only ordering edge that matters: it produces (1) the authoritative
**worklist** — which of the 55 files each later letter must *unit-test* versus
which A itself removed from the gate by excepting or by denominator repair — and
(2) a **fresh `target/coverage/coverage.json`** giving line-level uncovered
regions, without which the backfill tasks cannot name their targets. B–H are
mutually independent (each touches a disjoint set of files and its own test
modules) and may run in any order or in parallel once A lands.

Execution: A first, then B–H in topological order (all after A). B–H touch
disjoint files, so parallel worktrees are safe; each still re-runs the gate for
its own file glob before claiming done.

## 1. Goal

- `sh scripts/coverage-check.sh` exits 0 with `All non-excepted files >= 95% line
  coverage.`
- The CI `coverage` job's three steps all pass: per-file gate, and
  `cargo llvm-cov report $PKG_FLAGS --ignore-filename-regex "$IGNORE"
  --fail-under-lines 95`.
- Every file added to `scripts/coverage-exceptions.txt` names the specific
  syscall / network / subprocess / GUI boundary that blocks unit coverage, in the
  same style as the existing entries. No file is excepted merely because covering
  it is tedious.

### Non-goals (explicit constraints)

- **No production behavior change.** This is test-only + exception-list +
  (at most) the `IGNORE`/exception denominator config. No `src/**` runtime logic
  changes except where a test uncovers a genuine bug (which is then fixed per
  `AGENTS.md` "Never leave a bug you found", not worked around).
- **Do not weaken the gate.** FLOOR stays 95; `--fail-under-lines` stays 95. The
  per-file mechanism is not relaxed. Lowering the bar to pass is out of scope.
- **Do not except a unit-coverable file.** An exception is only for code whose
  uncovered remainder is unreachable from a unit test (live I/O, subprocess, GUI
  loop). "Hard to test" is not "impossible to test."
- **No golden/behavioral test edits to pass.** Follow `AGENTS.md`: a golden is
  touched only when PROVEN wrong. This plan adds tests; it does not re-baseline.

## 2. Current State

The gate is enforced two ways, both keyed off one instrumented profile:

- `scripts/coverage.sh` runs `cargo llvm-cov --workspace --all-targets` once,
  leaving merged profdata, then emits HTML/lcov/cobertura reports.
- `scripts/coverage-check.sh` regenerates a JSON summary from that cached
  profdata (no re-run) and prints every in-scope file below `FLOOR` (default 95),
  exiting non-zero if any non-excepted file falls short. `src/**` and
  `repository/src/**` are in scope; `target/`, `tests/`, `*_runtime_tables.rs`,
  `code/private/unicode.rs`, `src/testutil.rs` are excluded via `IGNORE`
  (`scripts/coverage-common.sh:26`).
- The CI job (`.github/workflows/coverage.yml`) runs both, plus a global
  `--fail-under-lines 95`.

The exception list (`scripts/coverage-exceptions.txt`) currently names six
Tier-D orchestration files + one defensive-arm file. **Two of those paths no
longer exist / no longer hold the code they excepted:**

- `src/cli/build.rs` (excepted) — DELETED by 19a95d448; its `build_project`
  linker-spawn and `load_build_signing_info` registry-attestation logic now live
  under `src/cli/build/` (mod.rs, signing.rs, …), none excepted.
- `src/main.rs` (excepted, "top-level command dispatch spawns subprocess
  build/link pipelines") — its dispatch body moved to `src/cli/dispatch.rs`
  (c565abda8), now 0% and un-excepted. `src/dispatch.rs`… i.e. `src/cli/
  dispatch.rs:27 run()` is the ex-`main.rs` router; it has no `#[cfg(test)]`.

### Measured populations

The failing set, from the gate output the user supplied
(`sh scripts/coverage-check.sh`). Column `uncov` = total − covered = lines a
letter must bring into coverage (by testing) or A must remove from the gate (by
excepting / denominator repair). Letter = the sub-plan that owns the file.

| Ltr | File | covered/total | pct | uncov |
|---|---|---|---|---|
| A | src/cli/dispatch.rs | 0/195 | 0.00 | 195 |
| A | src/os/windows/mod.rs | 0/38 | 0.00 | 38 |
| B | src/cli/build/signing.rs | 52/144 | 36.11 | 92 |
| B | src/cli/build/test_mode.rs | 17/42 | 40.48 | 25 |
| B | src/cli/build/packages.rs | 88/143 | 61.54 | 55 |
| B | src/cli/build/native_libs.rs | 180/251 | 71.71 | 71 |
| B | src/cli/build/resources.rs | 72/97 | 74.23 | 25 |
| B | src/cli/build/mod.rs | 1024/1233 | 83.05 | 209 |
| B | src/cli/mod.rs | 224/269 | 83.27 | 45 |
| C | src/os/linux/flavor.rs | 5/10 | 50.00 | 5 |
| C | src/os/windows/link/mod.rs | 437/513 | 85.19 | 76 |
| C | src/os/linux/appimage/mod.rs | 262/296 | 88.51 | 34 |
| C | src/os/link_encode.rs | 154/172 | 89.53 | 18 |
| C | src/os/linux/mod.rs | 168/184 | 91.30 | 16 |
| C | src/os/linux/appimage/squashfs/mod.rs | 321/351 | 91.45 | 30 |
| C | src/os/macos/icon.rs | 74/78 | 94.87 | 4 |
| C | src/binary_repr/mod.rs | 124/205 | 60.49 | 81 |
| C | src/binary_repr/writer.rs | 949/1017 | 93.31 | 68 |
| C | src/binary_repr/builder.rs | 231/247 | 93.52 | 16 |
| C | src/binary_repr/sections.rs | 956/1008 | 94.84 | 52 |
| D | src/arch/aarch64/backend.rs | 6/9 | 66.67 | 3 |
| D | src/arch/x86_64/encode/emitter.rs | 1471/1554 | 94.66 | 83 |
| D | src/ir/types.rs | 5/8 | 62.50 | 3 |
| D | src/ir/verify/calls.rs | 139/207 | 67.15 | 68 |
| D | src/ir/verify/link.rs | 434/571 | 76.01 | 137 |
| D | src/ir/verify/values.rs | 479/580 | 82.59 | 101 |
| D | src/ir/verify/resources.rs | 262/294 | 89.12 | 32 |
| D | src/ir/verify/compat.rs | 479/512 | 93.55 | 33 |
| D | src/ir/verify/mod.rs | 705/744 | 94.76 | 39 |
| E | src/ir/lower_link.rs | 246/275 | 89.45 | 29 |
| E | src/ir/lower.rs | 2692/2911 | 92.48 | 219 |
| E | src/ir/docs.rs | 128/138 | 92.75 | 10 |
| E | src/ir/link.rs | 456/490 | 93.06 | 34 |
| E | src/ir/binary.rs | 1180/1253 | 94.17 | 73 |
| E | src/ir/package.rs | 197/208 | 94.71 | 11 |
| F | src/syntaxcheck/link.rs | 350/686 | 51.02 | 336 |
| F | src/syntaxcheck/inference.rs | 1709/1824 | 93.70 | 115 |
| F | src/syntaxcheck/helpers.rs | 624/664 | 93.98 | 40 |
| F | src/syntaxcheck/mod.rs | 1558/1654 | 94.20 | 96 |
| G | src/ast/link_items.rs | 487/568 | 85.74 | 81 |
| G | src/ast/pipeline.rs | 146/166 | 87.95 | 20 |
| G | src/ast/scope_privates.rs | 434/480 | 90.42 | 46 |
| G | src/ast/expr.rs | 653/706 | 92.49 | 53 |
| G | src/ast/serialize.rs | 764/813 | 93.97 | 49 |
| H | src/monomorph/lower.rs | 2230/2358 | 94.57 | 128 |
| H | src/builtins/money.rs | 79/89 | 88.76 | 10 |
| H | src/builtins/strings.rs | 460/494 | 93.12 | 34 |
| H | src/docs/man/mod.rs | 216/247 | 87.45 | 31 |
| H | src/docs/spec/mod.rs | 129/139 | 92.81 | 10 |
| H | src/unicode/runtime_tables.rs | 309/385 | 80.26 | 76 |
| H | src/testing/desugar/coverage.rs | 177/214 | 82.71 | 37 |
| H | src/manifest/json_edit.rs | 381/406 | 93.84 | 25 |
| H | src/json.rs | 13/14 | 92.86 | 1 |
| H | src/audit/collect/project.rs | 310/342 | 90.64 | 32 |
| H | repository/src/main.rs | 648/815 | 79.51 | 167 |
| H | repository/src/backfill.rs | 340/358 | 94.57 | 18 |

Totals: 55 files. `A grep`-free cross-check of file count:
`sh scripts/coverage-check.sh 2>/dev/null | grep -c '%'` should equal the row
count above (minus the header) at the start of A.

### Verified properties

- **`src/cli/dispatch.rs` is integration-only orchestration** (VERIFIED — read
  `src/cli/dispatch.rs:1-241`): it is the ex-`main.rs` router (`run()` at :27)
  that dispatches to `build_project` (spawns the linker), `run_pkg_command` /
  `run_repo_command` (live HTTP registry), etc. Its pure parts (`is_help_flag`,
  the USAGE/`--version` arms) are unit-coverable; the command bodies are not. This
  is the same boundary `src/main.rs` was excepted for. → A re-triages it.
- **The `src/cli/build.rs` and `src/main.rs` exceptions are stale** (VERIFIED):
  `src/cli/build.rs` no longer exists (`ls` → GONE); the deleting commit is
  19a95d448. → A repoints.
- **`src/unicode/runtime_tables.rs` is NOT excluded by `IGNORE` and is NOT a pure
  generated table** (VERIFIED — the `IGNORE` alternative is `_runtime_tables\.rs$`
  which requires a `_` before "runtime"; `python3` re match on
  `src/unicode/runtime_tables.rs` → False; and the file head at
  `src/unicode/runtime_tables.rs:1-6` is hand-written `OnceLock` accessor logic
  over utf8proc/`unicode_casefold`, not a data array). It is legitimately in the
  denominator and is genuine backfill (→ H), NOT a denominator-regex bug.
- **The uncovered *line* set within each file is UNMEASURED** at authoring time.
  The on-disk `target/coverage/coverage.json` is from an earlier run (mtime
  Jul 21) and predates current `src/**`; it must not be trusted for line targets.
  A regenerates it. Every backfill task in B–H names its target *functions/
  regions* against A's fresh report, not against a guessed line list.

## 3. Design Overview

**Design uncertainty is concentrated in A** and is scheduled first, cheaply: the
premise "all 55 files need unit tests" is false — an unknown fraction are
integration-only and belong on the exception list, and the split of B–H assumes
A has already removed those. A's triage is the experiment that falsifies the
naive premise before any test is written against a file that should be excepted.

**Blast radius is low throughout** (test-only + a config file), so ordering is
driven purely by the A→{B..H} producer edge, not by risk-of-corruption.

The two work-kinds and how A separates them:

- A file is **excepted** iff its uncovered remainder is unreachable from a unit
  test — it spawns a subprocess (linker/codesign), performs live network I/O
  (registry HTTP), drives a real TTY/socket syscall, or runs a GUI event loop —
  AND its pure arg-parsing/validation/formatting is already covered or covered by
  A's split. Same rule the existing list documents.
- Otherwise the file is **backfill**: A leaves it on the worklist for its letter,
  which adds `#[cfg(test)]` tests. Where a file is *mostly* coverable with a thin
  integration-only tail, the letter covers the body and the tail's specific lines
  are added to the exception rationale only if they cannot be reached — but a
  whole-file exception is never used to skip a coverable body.

**Rejected alternatives:**

- *Lower FLOOR / add `#[allow]`-style blanket exclusions.* Rejected — violates
  the non-goals and `AGENTS.md`; it hides drift instead of fixing it.
- *One monolithic backfill sub-plan.* Rejected — ~3,300 uncovered lines across 55
  files is x-large; the split into B–H by test-module cohesion keeps each letter
  medium and independently landable/reviewable.
- *Split by "pipeline stage."* There is no producer→consumer pipeline among
  coverage targets; the only real edge is A→rest (A produces the worklist +
  report). Given that, B–H are cut by **test-module cohesion** (files that share a
  `#[cfg(test)]` module / test-fixture style are tested together): CLI/build,
  OS+object-writer, IR-verify, IR-lower, syntaxcheck, AST, leaf/misc+repository.
  This minimizes duplicated test scaffolding, which is the real cost driver here.

## 4. Standing requirements (folded in)

Per `AGENTS.md` and `.ai/compiler.md`:

- **Tests live beside code** in `#[cfg(test)] mod tests` blocks (precedent: 16
  files under `src/os/`, 7 under `src/syntaxcheck/`, 4 under `src/ast/` already
  carry them — `grep -rl "cfg(test)" <dir>`). Follow the nearest existing test
  module's style for the file being covered.
- **Run the full `cargo test`, never one module** (AGENTS.md). A green targeted
  test does not prove the suite is green.
- **A found bug is fixed, not worked around** (AGENTS.md "Never leave a bug you
  found"). Coverage tests routinely surface real defects; each is fixed on its own
  commit with a RED-first test, per `write-bug`.
- **Never edit a test/golden to pass** unless PROVEN wrong (AGENTS.md 4-question
  gate). This plan *adds* tests.
- **Coverage is the denominator check itself** — the whole point. Re-run
  `scripts/coverage-check.sh <glob>` (filter-aware) after each letter.
- **Git discipline** (memory `no-git-branches`): commit per file/cluster as it
  crosses 95%, stage by explicit path, no tree-wide operations. B–H in parallel
  worktrees each touch disjoint files.

## Phases (overview-level)

The feature's phases ARE the lettered sub-plans; each has its own phase list.

- [x] **A** — Triage, exception-list repair, fresh line-level report
      (`plan-68-A-triage-exceptions.md`). Lands first; produces the worklist.
      Done: coverage.json regenerated (suite 0-failed), dispatch.rs + signing.rs
      re-excepted, json.rs dead-arm resolved, worklist frozen (54 backfill files;
      +emitter.rs→D7, windows/mod.rs→C11). Commit: — (recorded next commit).
- [ ] **B** — CLI + build modules (`plan-68-B-cli-build.md`).
- [ ] **C** — OS backends + binary_repr object writers (`plan-68-C-os-binrepr.md`).
- [ ] **D** — IR verify + small IR + arch backend (`plan-68-D-ir-verify.md`).
- [ ] **E** — IR lower / binary / link (`plan-68-E-ir-lower.md`).
- [ ] **F** — syntaxcheck front-end (`plan-68-F-syntaxcheck.md`).
- [ ] **G** — AST front-end (`plan-68-G-ast.md`).
- [ ] **H** — Leaf/misc modules + repository crate (`plan-68-H-leaf-repository.md`).

## Validation Plan

- **Per-letter:** `sh scripts/coverage-check.sh <path-glob…>` (the checker takes
  trailing path-substring filters and reuses cached profdata) shows every file in
  the letter's set ≥95% or documented-excepted. Requires a fresh
  `sh scripts/coverage.sh` run first (the profile the checker reads).
- **Whole feature:** `sh scripts/coverage-check.sh` with no filter →
  `All non-excepted files >= 95% line coverage.`, exit 0; then
  `cargo llvm-cov report $PKG_FLAGS --ignore-filename-regex "$IGNORE"
  --fail-under-lines 95` → pass.
- **Suite:** `cargo test` → `0 failed` (new tests must not regress the suite).
- **Exceptions audit:** every line added to `coverage-exceptions.txt` names a
  concrete syscall/network/subprocess/GUI boundary; a reviewer can map each to a
  function in the file.
- **CI:** the `coverage` workflow job passes on the PR.

## Open Decisions

- **`src/json.rs` (13/14, one line).** RESOLVED during authoring — H's read of
  the file judged the lone uncovered line an **infallible / dead `stringify`
  fallback**, contradicting the earlier "cover it in H" guess. A confirms against
  the fresh report: if dead, **delete the line** (→ 13/13 = 100%, no exception, no
  test); if reachable, H covers it. See A's candidate table.
- **`src/arch/aarch64/backend.rs` (3/9).** RESOLVED during authoring — D verified
  `select_aarch64` (`src/arch/aarch64/select.rs:20`) loops over its input, so
  `AARCH64_BACKEND.select(&[])` is safe and `is_aarch64()`/`register_model()` are
  trivially callable. It is **coverable backfill (D)**, not a codegen-integration
  exception. No Open Decision remains.

## Corrections

- **A1 delta (2026-07-27, `sh scripts/coverage-check.sh` on the fresh profile):**
  the failing set is **56**, not 55 — one new file drifted below the floor:
  `src/arch/x86_64/encode/emitter.rs` (94.66%, 1471/1554, 83 uncov), which no
  letter covered. Assigned **backfill:D** as new phase **D7**; §2 table + D scope
  updated. Two files also moved within the failing set:
  `src/arch/aarch64/backend.rs` improved 3/9→6/9 (still D1);
  `src/os/linux/appimage/squashfs/mod.rs` regressed 327/351→321/351 (still C4).
- **A2:** the stale `src/cli/build.rs` exception (deleted file, matched nothing)
  is replaced by `src/cli/build/signing.rs`; `src/cli/dispatch.rs` added;
  `src/main.rs` kept with reworded boundary. 6→8 documented exceptions.
- **A3:** `src/os/windows/mod.rs` triaged **backfill:C** (new phase **C11**), not
  an exception. `src/json.rs` Open Decision resolved in `src/json.rs` itself
  (dead `.unwrap_or_else` → `.expect`), so it is neither excepted nor an H task.
- Full detail + measuring commands in `plan-68-A-triage-exceptions.md` Corrections.

## Summary

The engineering risk is concentrated in **A's triage judgement** — mis-classing a
coverable file as an exception hides real drift; mis-classing an integration-only
file as backfill sends a letter chasing lines a unit test can never reach. Every
such call must cite the specific boundary. The backfill letters (B–H) are
low-risk, mechanical, precedent-dense test authoring — fast for an AI — gated by a
single objective check (`coverage-check.sh <glob>` at 95%). Nothing in production
`src/**` behavior is intended to change; where a coverage test exposes a real bug,
that bug is fixed on its own commit, not papered over.
