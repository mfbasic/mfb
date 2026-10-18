# plan-176-F: MFBASIC sources (money-demo)

Last updated: 2026-10-18
Effort: small (<1h)
Depends on: plan-176-E

F splits the one over-limit MFBASIC file, `examples/money-demo/src/main.mfb` (1417 code LOC,
limit 1000), into five sibling files by responsibility: state and simulation, formatting,
ledgers, reports, and screen. The project globs `**/*.mfb` from `src`, so the split is the
motion of whole top-level declarations into new files, plus a copy of the 9-line IMPORT block
at the top of each new file. The program does not change behavior.

The single outcome: after F lands, `cargo test --test file_size_budget` shows no row for any
`examples/money-demo` file, and `tests/guards/file_size_ceilings.txt` has no row for one.

References:

- `planning/plan-176-A-file-size-guard.md`: the rule (code ≤ 1000, inline tests ≤ 250,
  test-only file ≤ 1000), the MFBASIC counting rules (§2), and §Decisions.
- Spec topics that set the motion rules:
  - `src/docs/spec/language/04_types.md` and `06_functions.md`: `PRIVATE` is file-local, and
    IMPORTs are per file.
  - `05_bindings-and-scope.md`: PUBLIC is visible anywhere in the project.
- `scripts/build-examples.sh` `TARGETS`: the five targets every example is built for.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-E is complete and archived | `ls planning/plan-176-E-* planning/completed/plan-176-E-*` → only `planning/completed/…` | NOT MET (2026-10-18) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | NOT MET (2026-10-18) |
| F's files have no uncommitted changes | `git status --porcelain -- examples/money-demo` → empty | MET (2026-10-18) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify — a
> prerequisite recorded NOT MET may have landed since, and one recorded MET may
> have regressed.
>
> **If you stop, report the current status of *all* prerequisites** — not only
> the one that blocked you. The reader needs to know how far off the gate is,
> not just which row you hit first.

## 1. Goal

- Every file in `examples/money-demo/src/` is ≤ 900 code LOC (the part target, under the
  1000 limit): `python3 -I tools/loc-census/census.py --file <path>` → first field ≤ 900.
- `cargo test --test file_size_budget` prints no row naming a `money-demo` path, and the
  ceiling file holds no row for one.
- money-demo builds for every `build-examples.sh` target and runs the same on the host.

### Non-goals (explicit constraints)

- No behavior change. No declaration is renamed or edited; the only edit beyond moving whole
  declarations is copying the IMPORT block into each new file.
- No manifest or README edit: `project.json` already globs `**/*.mfb`, and the README names
  no source file.

## 2. Current State

Counts: `python3 -I /tmp/loc/census.py "$PWD"` at HEAD `ce7a995ca` (2026-10-18) →
`examples/money-demo/src/main.mfb` code 1417, test 0. Per-region counts below come from
`python3 -I /tmp/loc/span.py examples/money-demo/src/main.mfb <a> <b>` on the same tree
(the file is 1649 lines; `git log 17a9b5799..HEAD -- examples/money-demo` → no commits).

| File | Kind | LOC | Tests |
|---|---|---|---|
| `examples/money-demo/src/main.mfb` | code | 1417 | 0 (`grep -c TESTING` → 0) |

money-demo has no goldens and no tests: `ls examples/money-demo` → `README.md`,
`project.json`, `src/`, `build/` (no `golden/`), and `grep -c TESTING examples/money-demo/src/main.mfb`
→ 0, so `mfb test` has nothing to run for it. Its gates are therefore the five-target build and
the host pty run (§Phase 1).

### Verified properties

- **The project globs `**/*.mfb` from `src`.** `cat examples/money-demo/project.json` →
  `"root": "src"`, `"include": ["**/*.mfb"]`. A new sibling file is compiled with no manifest
  edit.
- **Top-level PUBLIC declarations are project-visible, `PRIVATE` is file-local, and IMPORTs
  are per file** (spec topics above). The file has no `PRIVATE`
  (`grep -n PRIVATE examples/money-demo/src/main.mfb` → none). Every new file copies the whole
  IMPORT block (lines 1-9: collections, color, datetime, errorCode, io, math, money, strings,
  term). There is no unused-import diagnostic
  (`grep -in "unused.*import" src/docs/spec/diagnostics/01_rule-codes.md` → none).
- **Splitting an MFBASIC file changes `.ncode`, so byte-identity is not a gate for F.** Probe
  `/tmp/loc_f_probe`: the same program in one file and in two files, built with
  `mfb build -q --ncode`. `shasum` differed. The diff (75 lines) is source-path/line string
  constants (`"src/helpers.mfb"`) and the string-table renumbering they cause. Behavior is
  therefore the gate: the five-target build and the host run.
- **`money-demo` runs headless under a pty.** Probe `/tmp/loc_f_md` (a copy of the project):
  `mfb build -q .` (6.5 s), then
  `(sleep 3; printf q) | script -q ts sh -c 'stty rows 40 cols 120; ./build/money-demo.out'`
  exited 0, and `grep -a -o 'Welcome. Starting balances' ts` matched. At 80×24 it shows the
  "needs at least 80x25" screen instead, so the check pins 120×40.
- **Formatting baseline.** `target/release/mfb fmt --check examples/money-demo/src/main.mfb`
  → `All 1 file(s) already formatted` (2026-10-18).
- **No reference names the file.** `grep -rnoE "\[\[examples/money-demo[^]]*\]\]" src/docs/spec`
  → none; `grep -rn 'money-demo' scripts tests/guards .github .ai` → none. The two example
  guards (`tests/guards/example_readmes.rs`, `example_project_names.rs`) read `project.json`
  and the README only.
- **`build/` is ignored.** `git check-ignore -v examples/money-demo/build` → `.gitignore:24:build/`,
  so `mfb build` in place leaves the tree clean.

## 3. Design Overview

- Every new file begins with the original 9-line IMPORT block, verbatim.
- Declarations move whole, in their original relative order within the file that receives
  them.
- All top-level `LET`/`MUT` stay together in `main.mfb`, so no global initializer reads
  another file's global. `MUT mBook = openingMoney()` and `fBook = openingFloat()` call into
  `ledger.mfb`; they are function calls, not global reads.

The gate class is behavior: a build of money-demo on every `build-examples.sh` target and a
host pty run. Byte-identity is not available (§2 probe).

Rejected alternatives:

- *Fewer, larger files.* Two files would each fit under 900, but the five-way split follows
  the program's responsibilities (state/simulation, formatting, ledgers, reports, screen) and
  costs nothing extra.

## 4. Detailed Design

### 4.1 `examples/money-demo/src/main.mfb` (code 1417)

All files are in `examples/money-demo/src/`. LOC per file = the `span.py` count of its regions
(at `ce7a995ca`) + 9 for the IMPORT block. Line ranges are as of `ce7a995ca` (unchanged since
`17a9b5799`); FUNC and TYPE NAMES are authoritative if lines drift.

- `main.mfb` (418 = 9 + 140 + 159 + 110): the IMPORT block (1-9); the banner, `ENUM Kind`,
  `ENUM Acct`, all `TYPE`s (`Event` … `Row`) and every top-level `LET` constant (10-210); the
  simulation-state `MUT`s, `startDate`, `centsToMoney`, `pushLog`, `mk`, `eventsFor`,
  `record`, `simulateDay` (1121-1299); and `main` (1530-1649).
- `format.mfb` (139 = 9 + 130): `groupDigits`, `withCommas`, `dollars`, `signed`, `fm`, `fx`,
  `fmPlain`, `ff`, `ffPlain`, `fraw`, `fsci`, `thousandths`, `bpText`, `fit`, `fitRight`,
  `modeName`, `line`, `pct`, `cents`, `moneyMax`, `incomeTax` (211-375).
- `ledger.mfb` (413 = 9 + 404): the Money ledger `openingMoney` … `dueText` (376-581), the
  Float ledger `fpct` … `postFloat` (582-816), and `replayMoney`, `replayFloat` (817-838).
- `report.mfb` (272 = 9 + 263): `sumMoney`, `sumFloat`, `stampOf`, `isHalfCent`, `notBinary`,
  `balanceRow`, `roundingReport`, `driftRow`, `floatReport` (839-1120).
- `screen.mfb` (211 = 9 + 202): `toneColor`, `put`, `isMove`, `entryTone`, `fEntryTone`,
  `recentRows`, `expenseRows`, `drawColumn`, `moneyOrFloat`, `drawFrame`, `drawModal`,
  `nextKey` (1300-1529).
- Check: 418 + 139 + 413 + 272 + 211 = 1453 = 1417 + 4 × 9 (the four copied IMPORT blocks).
- Surface: none. No declaration changes visibility.
- Hazards: none. There is no spec citation, no path reference, and the README names no source
  file (§2).

## Phases

> **NOTE — keep the checkboxes current as you go. This is not bookkeeping; it is
> the only way anyone can see where the work actually is.**
>
> - Tick `- [x]` **in the same commit as the work it describes** — never in a
>   batch at the end of a phase, never "I'll update the plan after". If it is a
>   separate step it will be skipped, and the list becomes a lie.
> - Use `- [~]` for partially done and say in one line what remains.
> - Mark a task moot with `- [x] ~~text~~ — moot: <evidence>` rather than
>   deleting it. A vanished task is indistinguishable from a forgotten one.
> - Fill the phase's `Commit:` line with the hash(es) the moment it lands.
> - Add any task you discover you needed; an unlisted task that got done is a
>   task the next reader will do twice.
>
> **An unticked box means NOT DONE.** Someone reading this file — including the
> person who has to decide whether to stop — is entitled to trust that.

`M=target/release/mfb`. F changes no Rust, so one release build serves the letter.

### Phase 1 — money-demo

- [ ] `cargo build --release` (est. 6 min), then record the baseline before any edit:
      `$M build -q examples/money-demo && (sleep 3; printf q) | script -q /tmp/md-before.ts sh -c 'stty rows 40 cols 120; examples/money-demo/build/money-demo.out' >/dev/null; echo $?`
      → `0`, and `grep -a -c 'Welcome. Starting balances' /tmp/md-before.ts` → ≥ 1.
- [ ] Split `examples/money-demo/src/main.mfb` into `main.mfb`, `format.mfb`, `ledger.mfb`,
      `report.mfb`, `screen.mfb` per §4.1, each starting with the 9-line IMPORT block.
- [ ] `$M fmt --check examples/money-demo/src/*.mfb` → exit 0 (est. 10 s).
- [ ] Guard: `cargo test --test file_size_budget 2>&1 | grep -E '^  (NEW|GREW)'` → no output,
      and the only `TIGHTEN` row is `code … examples/money-demo/src/main.mfb`. Then
      `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`, rerun →
      `grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`. Commit `tests/guards/file_size_ceilings.txt`
      with the phase (est. 2 min).

Acceptance: money-demo builds for every example target and runs the same on the host.
  Check: `for t in macos-aarch64 linux-aarch64 linux-x86_64 linux-riscv64 windows-x86_64; do $M build -q --target $t examples/money-demo >/dev/null || echo FAIL $t; done`
  → no output (est. 1 min). Then
  `$M build -q examples/money-demo && (sleep 3; printf q) | script -q /tmp/md.ts sh -c 'stty rows 40 cols 120; examples/money-demo/build/money-demo.out' >/dev/null; echo $?`
  → `0`, and `grep -a -c 'Welcome. Starting balances' /tmp/md.ts` → ≥ 1 (est. 15 s).
  `for f in examples/money-demo/src/*.mfb; do python3 -I tools/loc-census/census.py --file $f; done`
  → five lines whose first fields, in glob order (format, ledger, main, report, screen), are
  139, 413, 418, 272, 211.
Commit: —

### Phase 2 — `money.mfb` keyword spelling (pre-existing defect)

`tests/acceptance/src/money.mfb` spells the keyword `TRUE` in lowercase on 10 lines (109-115,
266, 268, 306 at `ce7a995ca`: `grep -n ', true)' tests/acceptance/src/money.mfb`), so
`mfb fmt --check` fails on it. The old F fixed this while splitting the file. Under the
2026-10-18 rule that file is not split (it is under 1000), but the defect still gets fixed
(AGENTS.md: never leave a bug). The fix does not change behavior, because keyword matching
is case-insensitive (`src/docs/spec/language/02_lexical-structure.md:3`).

- [ ] Before the edit: `$M test tests/acceptance | grep '^Tests:'` → record the line (est. 2 min).
- [ ] `$M fmt tests/acceptance/src/money.mfb`, then `git diff --stat tests/acceptance/src/money.mfb`
      → 10 lines changed, every one only `true` → `TRUE`.

Acceptance: the file is formatter-clean and the acceptance suite is unchanged.
  Check: `$M fmt --check tests/acceptance/src/money.mfb` → exit 0;
  `$M test tests/acceptance | grep '^Tests:'` → the same line as before the edit (est. 2 min).
Commit: —

## Validation Plan

- Tests: money-demo has no tests and no goldens (§2), so nothing is added or removed.
- Coverage check: `.mfb` sources are not in the Rust coverage denominator, and F changes no
  Rust.
- Runtime proof: the host pty run (`Welcome. Starting balances` on screen, exit 0) before and
  after the split, and the build on all five example targets.
- Doc sync: none. No spec citation, script, guard or `.ai` doc names the file (§2).
- Final gate: `cargo test --test file_size_budget 2>&1 | grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'`
  → 0, and `grep -c 'examples/money-demo/' tests/guards/file_size_ceilings.txt` → 0
  (est. 2 min). The Rust workspace suite runs once, in plan-176-V.

## Decisions

- Rule revised — owner, 2026-10-18 (replaces the 2026-10-14 rule):

  | Kind | Applies to | Limit | Over the limit → |
  |---|---|---|---|
  | `code` | code LOC of a file that is not a test-only file | 1000 | split the file into modules |
  | `inline` | inline test LOC (`#[cfg(test)]` items / MFBASIC `TESTING … END TESTING`) inside a file that is not a test-only file | 250 | move the tests out into their own test-only file(s) |
  | `test` | total LOC of a test-only file (path component `tests`, `test_*.mfb`, `#![cfg(test)]`, or a `#[cfg(test)] mod x;` target and its subtree) | 1000 | split the test-only file into modules |

  Comments and blank lines do not count. There are no exemptions; generated files
  (`GENERATED` banner) are skipped automatically. Ceiling rows are
  `ceiling <code|inline|test> <n> <path>`. Parts target ≤ 900 LOC of their kind. The guard
  warns with a banner and never fails. A new module (here a new `.mfb` file in the project)
  is always an acceptable split.
- Scope: `.rs`/`.mfb` under `src/` `packages/` `examples/` `repository/` `tests/` — owner,
  2026-10-14.
- Every new file copies the original's whole IMPORT block. There is no unused-import rule, and
  per-file import pruning is churn — author.
- Five files by responsibility rather than the minimum two — author (§3).

## Corrections

- 2026-10-18 — Rule revised by the owner: test-only files limit 1000 (was 250), inline tests
  250, no exemptions. Dropped 21 files: `packages/jwt/src/test_verify.mfb`,
  `packages/xml/src/test_write.mfb`, `packages/xml/src/test_xpath_edges.mfb`; the 15
  `tests/acceptance/src/` suites `arithmetic`, `astrings`, `bits`, `collections`, `crypto`,
  `csv`, `encoding`, `general`, `json`, `math`, `money`, `primitives`, `regex`, `strings`,
  `vector`; and the rt-behavior fixtures `datetime-arith-errors-rt`,
  `union-variant-expression-wrap` and `literal-spans-lines-valid` (each ≤ 1000 test LOC, so
  not split). The `literal-spans-lines-valid` exempt row is gone with no replacement (523 ≤
  1000). Five phases became two; no golden is re-baselined any more. The `money.mfb`
  lowercase-`true` fmt fix stays, as its own Phase 2, without the split. money-demo's design is unchanged, its counts
  re-measured at `ce7a995ca`.

## Summary

F is motion of whole MFBASIC declarations from `examples/money-demo/src/main.mfb` (1417 code
LOC) into five sibling files of ≤ 418 LOC each, in a project that already globs `**/*.mfb`.
Byte-identity cannot gate it, because file layout reaches `.ncode` through source-location
constants (probe). The gates are behavior: a money-demo build on five targets plus a host pty
run before and after.
