# plan-176-A: File-size budget guard, oracle and ceilings

Last updated: 2026-10-18
Overall Effort: huge (>3d) — 22 sub-plans, A–V
Effort: large (3h–1d)
Depends on: nothing

plan-176 enforces one rule across the tree:

> **Code and code + test files:** over **1000** lines of code → split the file. Over **250**
> lines of inline tests → move the tests out into their own test-only file(s). (So a mixed
> file holds at most 1000 code + 250 inline test = 1250.)
>
> **Test-only files:** over **1000** lines of tests → split the test-only file.
>
> **All files:** comments (and blank lines) do not count. Splitting files into modules is
> always acceptable.

The guard therefore measures three kinds:

| Kind | What is counted | Limit |
|---|---|---|
| `code` | code LOC of a file that is not a test-only file | 1000 |
| `inline` | inline test LOC (`#[cfg(test)]` items, MFBASIC `TESTING … END TESTING`) inside a file that is not a test-only file | 250 |
| `test` | all LOC of a test-only file | 1000 |

This sub-plan builds the enforcement and the measurement every later letter uses: a guard
test that counts LOC exactly, a checked-in ceiling file that can only go down, an independent
oracle that cross-checks the guard, and the written rule. When it is done, `cargo test --test
file_size_budget` prints a loud banner naming every file that is over its limit and not
covered by a ceiling. It also names every file that grew past its ceiling, and
every ceiling that can be lowered. Sub-plans B–V then bring the ceilings down until the file is
empty. There are no exemptions apart from generated files.

The single outcome of A: on the tree at the commit that lands A, the guard reports **zero**
NEW, GREW, STALE and TIGHTEN rows. That proves its counts equal the oracle's for every
over-limit file, and that it agrees with the oracle that every other in-scope file is within
its limit.

References:

- `AGENTS.md` — "Always" section (where the rule is written) and the test/golden rule.
- `.ai/build-tooling.md` — where the split mechanics are written.
- `.ai/testing-gates.md` § "Splits must sweep man AND spec citations" — every later letter
  depends on this.
- `tests/guards/test_targets_registered.rs` — the guard style this mirrors (filesystem lint,
  prints the fix, hand-rolled parsing, no new crates).
- `src/docs/spec/language/02_lexical-structure.md` §2.4 and §"Comments" — MFBASIC comment
  syntax (`'`, statement-leading `REM`, `DOC … END DOC`).
- `src/docs/spec/language/22_test-framework.md` — `TESTING … END TESTING` blocks.
- `scripts/check-generated.sh` — the generated artifacts.
- Sub-plans `planning/plan-176-B-…` through `planning/plan-176-V-…`.

## Prerequisites

None. A adds files and two doc paragraphs and touches no compiler code.

Every later letter has the same prerequisite shape: **its own files have no uncommitted
changes** (`git status --porcelain -- <its paths>` → empty), so a pure move never captures
someone else's work in progress. On 2026-10-18 no in-scope file is dirty
(`git status --porcelain` at `ce7a995ca` lists only `planning/baremetal.md` and
`debugger-report.md`).

## 1. Goal

- `cargo test --test file_size_budget` exists, always passes, and writes a banner to the
  terminal (raw stderr, so libtest's capture does not swallow it) listing every violation row.
- `tests/guards/file_size_ceilings.txt` holds one ceiling per over-limit (file, kind) at its
  current count. It has no exemption rows.
- `tools/loc-census/` holds the Python oracle that produced the seed. On the A commit, the
  guard's count equals the oracle's for every in-scope file.
- `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget` rewrites the ceiling file. It lowers
  ceilings and removes entries that are no longer needed. It never adds an entry and never raises one.
- `AGENTS.md` states the rule, and `.ai/build-tooling.md` states how to split.

### Non-goals (explicit constraints)

- The guard **never fails** the test (decided by the owner: warn loudly). It must not
  `panic!`, `assert!` or return `Err` on a violation. Its own unit tests (the lexer cases) do
  assert, as ordinary tests.
- No file outside `tests/guards/`, `tools/loc-census/`, `Cargo.toml`, `AGENTS.md` and
  `.ai/build-tooling.md` changes in A. No code is split in A.
- No new crate dependency (mirrors `test_targets_registered.rs::registered_targets`, which
  hand-rolls its TOML scan for the same reason).

## 2. Current State

There is no size rule and no size guard. `grep -rn "1000\|LOC" AGENTS.md .ai/build-tooling.md`
finds no file-size rule.

### The counting rules (decided; §Decisions)

| Language | Not LOC | Test LOC |
|---|---|---|
| Rust `.rs` | blank lines; `//`, `///`, `//!` to end of line; `/* … */` including nested, including `/** */` and `/*! */`. Comment markers inside string, raw-string (`r"…"`, `r#"…"#`, `br…`, `cr…`), byte-string and char literals are NOT comments. | the whole file if a path component is `tests`, or it is a module declared under `#[cfg(test)]` (and that module's subtree), or it begins with `#![cfg(test)]`. Otherwise, the lines of every item under a `#[cfg(test)]` / `#[cfg(all(test, …))]` attribute (extent rule below). |
| MFBASIC `.mfb` | blank lines; lines whose first token is `'`; lines whose first token is `REM` followed by whitespace or end of line; every line from a statement-leading `DOC` line (optionally followed by uppercase attribute words) through its `END DOC` line. | the whole file if a path component is `tests` or the file name starts with `test_`. Otherwise, the lines from `TESTING` through `END TESTING` inclusive. |

Code LOC = LOC − test LOC. For a test-only file (the "whole file" cases in the table), every
LOC is kind `test`, limit **1000**. For any other file, code LOC is kind `code`, limit **1000**,
and its test LOC is kind `inline`, limit **250**.

**Extent of a `#[cfg(test)]` item** (Rust, scanned over code text after comments and literals
are blanked). Starting at the attribute, with `d` = the bracket depth there, counting `(`, `[`
and `{` together, the item ends at the first of:
- a `;` or `,` at depth `d`;
- a `}` that returns depth to `d` after a `{` opened at depth `d + 1`;
- a closer that drops depth below `d`. That closer belongs to the enclosing construct, so a line
  that *starts* with it is not test code.

Every line from the attribute through the ending line is test code. This covers modules,
fns, impls, `const` tables that open with `&[` (the case a brace-only rule cuts off after one
row: `src/codegen/collection/assign/self_update.rs` is 1059 code / 2085 test, not 1930 / 1214),
`use`/`mod x;`, enum variants, struct fields, match arms (`=> { … }` and `=> expr,`) and bare
blocks. The oracle and the guard implement this same rule, and §4.4 pins each shape.

**Scope:** `.rs` and `.mfb` files under `src/`, `packages/`, `examples/`, `repository/`, `tests/`.
Any directory named `target` or `.git` is skipped.

**Generated files are exempt automatically.** A file is generated when one of its first three
lines is a comment containing the uppercase word `GENERATED`. At planning time that matches
exactly the 9 files the generators own (oracle run below):
`src/codegen/builtins/encoding/helper_codepage_table.rs`,
`src/codegen/builtins/vector/simd_kernel_coeffs.rs`,
`src/codegen/string/unicode/unicode_script_names.mfb`, `packages/timezones/src/data.mfb`,
`packages/zip/src/test_fixtures.mfb`, `packages/zip/src/cp437.mfb`,
`packages/tar/src/test_fixtures.mfb`, `packages/currency/src/data.mfb`,
`examples/brogue/src/tables.mfb`. No hand-written file in scope carries the uppercase word in
its first three lines. The only other case-insensitive hits are lowercase prose, such as
`src/html.rs: //! Shared HTML helpers for the generated documentation`.

### Measured populations

All counts come from the planning oracle (`/tmp/loc/census.py`, which becomes
`tools/loc-census/census.py` in Phase 1) run on the tree at `ce7a995ca` (2026-10-18):
`python3 -I /tmp/loc/census.py "$PWD"` (14 s), then summarized from `census.json` by kind.
The letters' line ranges were mapped at `17a9b5799`; item names are authoritative.

| What | Count |
|---|---|
| In-scope `.rs` + `.mfb` files | 4470 |
| Generated (auto-exempt) | 9 |
| `code` > 1000 | 98 files (97 `.rs`, 1 `.mfb`), 76,383 LOC over the limit |
| `inline` > 250 | 71 files (all `.rs`), 60,191 inline test LOC |
| `test` > 1000 (test-only files) | 30 files (all `.rs`), 67,935 test LOC |
| Ceiling rows to seed | 98 + 71 + 30 = 199 |

Largest offenders (same run): `code` `src/codegen/runtime/canvas/vulkan.rs` 7309,
`src/ir/lower.rs` 5222, `src/codegen/engine/control/builder_control.rs` 4466; `test`
`src/ir/verify/tests.rs` 8495, `src/ir/tests.rs` 6386, `tests/runtime/rt_scope_drop_leaks.rs`
5476; `inline` `repository/src/server.rs` 7487.

The seed is re-taken from the oracle when Phase 2 runs (§Phase 2). The numbers above are the
planning snapshot that sized the letters, not values to type in.

### Verified properties

- **A passing test's output is only visible if it bypasses libtest's capture.** Probe in a
  throwaway crate (`/tmp/loc/probe`, a `[[test]]` whose test calls both `eprintln!` and
  `std::io::stderr().write_all`): `cargo test --quiet --test rt_demo 2>&1 | grep VIA` printed
  only `VIA-RAW-STDERR`. The banner therefore writes through `std::io::stderr().lock()`
  with `write_all`, never through `eprintln!`/`println!`.
- **Test-target submodules via `#[path]` work and keep filters working.** Same probe crate:
  root `tests/runtime/rt_demo.rs` with `#[path = "rt_demo/lists.rs"] mod lists;`.
  `cargo test --test rt_demo -- --list` listed `lists::list_a: test` and `maps::map_b: test`,
  and `cargo test --test rt_demo list_a` ran 1 test. This is the layout B–F use. It is why the
  guard itself can be split the same way in Phase 1.
- **`test_targets_registered.rs` tolerates the layout.** `test_files_in_subdirectories` reads
  only the direct `.rs` children of `tests/<dir>/` (it skips directories), so
  `tests/guards/file_size_budget/*.rs` needs no stanza. `a_registered_target_is_named_after_its_file`
  requires the target name to equal the root's stem, and `file_size_budget` = `file_size_budget.rs`.
- **The only self-respawning `--exact` test is out of scope.**
  `grep -rn -- '--exact' scripts .ai AGENTS.md tests src/docs Cargo.toml` → one hit,
  `tests/net/rt_port_gate_is_cross_process.rs:93`. That file is 79 test LOC, under the limit.
- **Moving Rust code cannot change compiler output through source locations.**
  `grep -rnE 'module_path!|\bfile!\(|\bline!\(' src repository/src` → no hits. `#[track_caller]`
  (only in `src/target/shared/abi.rs`) affects panic text, not emitted code. No link-order
  registration: `grep -nE '^(inventory|linkme|ctor)' Cargo.toml repository/Cargo.toml` → no
  hits. This is why byte-identity is the gate for code letters L–V.
- **The coverage gate ignores directories named `tests/`.** `scripts/coverage-common.sh`
  `IGNORE='…|(^|/)tests/|…'`. That is why extracted test modules go into a `tests/` directory
  (`foo/tests/mod.rs`), not `foo/tests.rs`, in letters G–K.
- **The spec citation test is file-level only.** `src/docs/spec/mod.rs::spec_citations_resolve`
  documents this deliberately (bug-338-H2). A split that keeps the cited file
  path alive while moving the symbol out of it would pass silently. Converting `foo.rs` to
  `foo/mod.rs` (the code letters' convention) makes every citation of the old path fail
  `spec_citations_resolve`, which forces the sweep. That is a deliberate forcing function.

## 3. Design Overview

Three pieces:

1. **Oracle** — `tools/loc-census/census.py` (Python, stdlib only). This is the planning census
   made permanent. It emits per-file `(path, code, test, generated, testfile)` JSON and,
   with `--seed`, the ceiling-file body (kind `test` for a test-only file, else `code` and
   `inline`).
2. **Guard** — `tests/guards/file_size_budget.rs` (target root) plus submodules under
   `tests/guards/file_size_budget/`. It walks the scope, counts with its own Rust
   implementation of the same rules, compares against the ceiling file, writes the banner, and
   handles `MFB_LOC_TIGHTEN=1`.
3. **Ceiling file** — `tests/guards/file_size_ceilings.txt`.

Two independent implementations of the counting rules (Python oracle, Rust guard) agreeing on
4470 files is the correctness check for both. The risk is in the Rust lexer's edge cases (raw
strings, lifetimes vs char literals, nested block comments, `REM` as an identifier prefix).
The unit tests in Phase 1 pin each edge case, and the Phase 2 agreement run pins the whole tree.

**Byte-identity is not a gate for A** — A changes no compiler code.

Rejected alternatives:

- *`build.rs` `cargo:warning=`* — shows on every build, but noisy, and it puts a tree walk on
  the build's critical path. The owner chose a banner from the guard.
- *Failing the test* — the owner chose warn-only.
- *A `scripts/` gate* — it would not run in the default `cargo test`, so nobody would see it.
- *`syn`/`proc-macro2` tokenizer* — a new dependency for a guard (contrary to the
  `registered_targets` precedent), and it gives no MFBASIC support.
- *Name heuristics for Rust test files (`*_tests.rs`, `tests.rs`)* — dropped. The path
  component `tests` plus `#[cfg(test)]` module resolution is principled. On the planning tree,
  the union of the heuristic and the principled rule classified every file the same as the
  principled rule plus the `tests` component (oracle diff run, 0 differing files).
- *Ceilings that `MFB_LOC_TIGHTEN` can add or raise* — that would turn the ratchet into a
  rubber stamp. New entries are never added.
- *Hand-written exemptions* — the rule has none; an over-limit file is split. Only generated
  files are exempt, and that is detected, not listed.

## 4. The guard

### 4.1 Files (test-only files, each ≤ 1000 test LOC by its own count)

| File | Contents |
|---|---|
| `tests/guards/file_size_budget.rs` | target root: `//!` doc of the rule, `#[path]` mod declarations, the `#[test] fn file_size_budget()` driver (walk → count → compare → banner → `MFB_LOC_TIGHTEN` rewrite) |
| `tests/guards/file_size_budget/rust_lex.rs` | `fn rust_lines(text) -> Vec<LineKind>` — per line: Blank / Comment / Code, with string/char/raw-string/nested-comment handling; plus `fn rust_test_lines(text, lines) -> Vec<bool>` (inline `#[cfg(test)]` item extents by brace depth over code-only text) |
| `tests/guards/file_size_budget/mfb_lex.rs` | `fn mfb_lines(text) -> Vec<LineKind>` (`'`, `REM`, `DOC … END DOC`) and TESTING-block extents |
| `tests/guards/file_size_budget/classify.rs` | scope walk; whole-file test classification (`tests` component, `test_*.mfb`, `#![cfg(test)]`, `#[cfg(test)] mod x;` and `#[cfg(test)] #[path = "…"] mod x;` resolution to file or directory subtree); generated detection |
| `tests/guards/file_size_budget/ceilings.rs` | parse/write `file_size_ceilings.txt`; compare; produce rows; tighten |
| `tests/guards/file_size_budget/lex_cases.rs` | `#[test]` cases for both lexers and the classifier (§4.4) |

### 4.2 Ceiling file format

```
# tests/guards/file_size_ceilings.txt — plan-176. Limits (non-comment, non-blank LOC):
#   code 1000, inline 250 (inline tests in a code file), test 1000 (test-only file).
# ceiling <code|inline|test> <n> <path>  an over-limit file may not grow past n; MFB_LOC_TIGHTEN=1 lowers n.
ceiling code 7309 src/codegen/runtime/canvas/vulkan.rs
ceiling inline 748 src/codegen/runtime/canvas/vulkan.rs
ceiling test 8495 src/ir/verify/tests.rs
```

Rows are sorted by path, then kind. `#` lines and blank lines are ignored. Any other line
(including an unknown kind) is a parse error, reported as a `STALE` banner row (the guard
still does not fail).

### 4.3 Rows and the banner

For each in-scope, non-generated file and each kind (code, test), with `n` = count and `L` =
limit:

| Condition | Row |
|---|---|
| `n ≤ L`, no entry | — |
| `n ≤ L`, ceiling entry | `TIGHTEN` (entry now removable) |
| `n > L`, ceiling `c`, `n == c` | — |
| `n > L`, ceiling `c`, `n < c` | `TIGHTEN` (lower to `n`) |
| `n > L`, ceiling `c`, `n > c` | **`GREW`** |
| `n > L`, no entry | **`NEW`** |
| entry for a path that no longer exists, or an unparseable line | **`STALE`** |

If any row exists, the driver writes this to `std::io::stderr().lock()` with `write_all`:

```
################################################################################
##  FILE SIZE BUDGET (plan-176): 2 over, 1 grew, 3 can tighten                 ##
##  code ≤ 1000, inline tests ≤ 250, test-only file ≤ 1000 non-comment LOC.    ##
##  Split into a new module; move inline tests out to a test-only file.        ##
################################################################################
  NEW      code 1043 > 1000  src/codegen/foo.rs
  NEW      inline 262 > 250  src/codegen/foo.rs
  GREW     test 1262 > 1000 (ceiling 1255)  tests/runtime/rt_x.rs
  TIGHTEN  code  4100 (ceiling 4466)  src/codegen/engine/control/builder_control.rs
  run `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget` to lower ceilings.
################################################################################
```

The test then returns normally. It never fails.

### 4.4 Lexer cases (`lex_cases.rs`)

One `#[test]` per case, asserting the per-line classification of a short literal input:
`//` inside `"…"`; `"` inside `r#"…"#`; `b"//"`; `'"'` char literal; lifetime `'a` followed by
`"//"` on the same line; nested `/* /* */ */` spanning lines; `/// doc` and `//! doc`;
code then trailing `// comment` (Code); a `#[cfg(test)] mod tests { … }` block
(test lines); `#[cfg(test)] mod tests;` (one test line); `#![cfg(test)]` file (all test);
kind assignment (a `tests/` path → `test` only; a mixed file → `code` and `inline`);
MFBASIC `'` line, `REM x`, `REMAINDER = 1` (Code), `DOC INTERNAL … END DOC`, a `TESTING`
block, a line in a `test_x.mfb` file; a generated banner in line 3 (generated) vs line 4 (not);
and one case per `#[cfg(test)]` extent shape in §2 (module, `&[` const table, `use`, `mod x;`,
enum variant, last struct field without a comma, `=> { }` arm, `=> expr,` arm, bare block,
stacked attributes).

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

### Phase 1 — Oracle and guard (no ceiling file yet)

Lands the measurement. Without a ceiling file the guard reports every over-limit file as NEW,
which is correct and harmless (it never fails).

- [x] `tools/loc-census/census.py`: the planning oracle (`/tmp/loc/census.py` as of this plan)
      with the generated-banner rule and the `#[cfg(test)]` extent rule from §2, an `argparse` CLI
      (`census.py [--root DIR] [--json OUT] [--seed] [--file PATH [--lines A B]]`), stdlib only.
      `--file PATH` prints `code test` for one file. `--file PATH --lines A B` prints the LOC
      of lines A..B (1-based, inclusive). B–V use these modes to re-measure a part before
      and after a move. `--seed` prints the
      `ceiling` rows (sorted, §4.2 format) for every (file, kind) over its limit that is not
      generated. The script has no exemption table.
- [x] `tools/loc-census/README.md`: what it is (the independent oracle for
      `tests/guards/file_size_budget.rs`), the counting rules table from §2, usage, and that
      it is not a gate.
- [x] `tests/guards/file_size_budget.rs` + `tests/guards/file_size_budget/{rust_lex,mfb_lex,classify,ceilings,lex_cases}.rs`
      per §4.1–4.4. A missing `file_size_ceilings.txt` is read as empty.
- [x] `Cargo.toml`: `[[test]] name = "file_size_budget" path = "tests/guards/file_size_budget.rs"`
      in the `# --- tests/guards/ ---` block.

Acceptance: the guard and the oracle agree on exactly which (file, kind) pairs are over the
limit and on each one's count.
  Check: `cargo test --test file_size_budget 2>&1 | awk '$1=="NEW"{print $2, $3, $NF}' | sort > /tmp/g.txt`;
  `python3 -I tools/loc-census/census.py --seed | awk '$1=="ceiling"{print $2, $3, $4}' | sort > /tmp/o.txt`;
  `diff /tmp/g.txt /tmp/o.txt` → no output (est. 2 min). And `cargo test --test file_size_budget`
  → every `lex_cases` test passes, and the banner has 0 GREW, 0 STALE and 0 TIGHTEN rows.
  Result (2026-10-18, base `7fa2dc5dc`): `diff /tmp/g.txt /tmp/o.txt` → no output, 199 rows each;
  `cargo test --test file_size_budget` → `test result: ok. 32 passed`; banner
  `199 over, 0 grew, 0 stale, 0 can tighten`. Stronger whole-tree check: a temporary per-file
  dump from the guard compared with `census.py --json` → `files 4485 4485`, `differ 0`
  (dump code removed before commit). The new oracle also equals the planning oracle
  (`/tmp/loc/census.py`) on all 4479 pre-A files, `differ 0`.
Commit: —

### Phase 2 — Seed the ceilings

- [ ] `python3 -I tools/loc-census/census.py --seed > tests/guards/file_size_ceilings.txt`,
      then prepend the §4.2 header comment.
- [ ] `tests/guards/file_size_budget/ceilings.rs`: `MFB_LOC_TIGHTEN=1` rewrite (lower `n`,
      drop entries for files now within limit or missing, never add, never raise), with a `lex_cases.rs` case that feeds an in-memory
      ceiling table plus counts and asserts the rewritten text.

Acceptance: the seeded tree is clean and the ratchet only goes down.
  Check: `cargo test --test file_size_budget 2>&1 | grep -c '^  \(NEW\|GREW\|STALE\|TIGHTEN\)'`
  → `0`. Then `python3 -c 'print("fn f() {}\n" * 1100)' > src/zz_budget_probe.rs` (an
  undeclared file, which the walk still sees), rerun → exactly one row,
  `NEW      code 1100 > 1000  src/zz_budget_probe.rs`, then `rm src/zz_budget_probe.rs` (est. 2 min).
  `grep -c '^ceiling' tests/guards/file_size_ceilings.txt` equals the oracle's over-limit row
  count printed by `census.py --seed | grep -c '^ceiling'` (est. 30 s).
Commit: —

### Phase 3 — Write the rule down

- [ ] `AGENTS.md`, "Always": one bullet — *File size (non-comment, non-blank LOC; `.rs`/`.mfb`
      under `src/ packages/ examples/ repository/ tests/`): a code file ≤ 1000 code and ≤ 250
      inline tests (`#[cfg(test)]`/`TESTING`); a test-only file ≤ 1000. Over 1000 code → split
      into modules; over 250 inline → move the tests to their own test-only file; test-only
      file over 1000 → split it. `cargo test --test file_size_budget` prints a banner;
      `tests/guards/file_size_ceilings.txt` ceilings only go down (`MFB_LOC_TIGHTEN=1`). No
      exemptions except generated files (`GENERATED` in the first three lines). Mechanics:
      `.ai/build-tooling.md`.*
- [ ] `.ai/build-tooling.md`: new section "## Splitting an over-budget file (plan-176)" with
      the conventions every letter uses. Code: `foo.rs` → `foo/mod.rs` + parts, `mod.rs`
      re-exports the surface, ≤ 900 per part, pure motion, sweep `[[foo.rs:` citations and
      `tests/guards` allowlists, inherit `scripts/coverage-exceptions.txt` entries. Inline tests
      over 250: move to `foo/tests/mod.rs` (one file unless it is over 1000). Test-only files
      over 1000: root stays, `#[path = "<name>/<topic>.rs"] mod <topic>;` (integration) or
      `tests/<topic>.rs` submodules (src), ≤ 900 per part. Gates: `-- --list` count before ==
      after; byte-identity for code. Then tighten.
- [ ] `AGENTS.md` "Read before that kind of work": extend the existing `.ai/build-tooling.md`
      line's description with ", file-size splits (plan-176)".

Acceptance: the rule is findable from AGENTS.md.
  Check: `grep -n 'file_size_budget' AGENTS.md .ai/build-tooling.md` → one or more hits in each (est. 5 s).
Commit: —

## Validation Plan

- Tests: `lex_cases.rs` (every edge case in §4.4, and the tighten rewrite).
- Coverage check: the guard is under `tests/`, which coverage ignores by design
  (`coverage-common.sh` IGNORE). The oracle agreement is the coverage of the counting rules.
- Runtime proof: the Phase 2 scratch-file check (a real over-limit file produces a NEW row).
- Doc sync: `AGENTS.md`, `.ai/build-tooling.md`, `tools/loc-census/README.md`. No `mfb spec`
  or `mfb man` topic is affected (no compiler contract changes).
- Final gate (once, after Phase 3): `cargo test --test file_size_budget --test test_targets_registered`
  → all pass, banner shows 0 rows (est. 2 min). The full suite is not needed: A adds one
  test target and touches no compiled code outside it.

## Decisions

- Scope is `.rs` and `.mfb` under `src/`, `packages/`, `examples/`, `repository/`, `tests/`
  — decided by the owner, 2026-10-14. `benchmark/`, `tools/`, `spikes/` and `scripts/` are out.
- Comments and blank lines are not LOC. Doc comments (`///`, `//!`, `DOC … END DOC`) are
  comments — owner (comments) and author (doc comments are comments), 2026-10-14.
- **The rule (owner, 2026-10-18; replaces the 2026-10-14 "test ≤ 250" rule).** Code and
  code + test files: over 1000 code LOC → split the file; over 250 inline test LOC → move the
  tests out into their own test-only file(s). Test-only files: over 1000 → split the file.
  A mixed file is therefore at most 1000 + 250 = 1250. Kinds `code`/`inline`/`test` (§2).
- **No exemptions** — owner, 2026-10-18. Every over-limit file is split. The five input
  fixtures exempted on 2026-10-14 (the three parser depth `main.mfb` files, `float_fixed.mfb`,
  `frame_skip_nested_lists.mfb`) are test-only files of 253–607 LOC, under the 1000 limit
  (census run above), so they need neither an exemption nor a split. The letters' former
  test-side exemptions are resolved in each letter (Corrections, 2026-10-18).
- Generated files are exempt — owner, 2026-10-14. Detection is the `GENERATED` banner (§2).
  This is detection, not an exemption list.
- Over-limit files produce a loud banner, not a failure — owner, 2026-10-14.
- Splitting by making a new module is always acceptable, for code and tests — owner, 2026-10-14.
- A single **function** over the code limit is decomposed by extract-method refactoring, gated
  by byte-identical `.ncode` on every target — decided by the owner, 2026-10-14. This covers
  `lower_module_for_platform` (1801), canvas `register` (2093), `lower_ops_inner` (1313),
  `lower_value_inner` (1224), `emit_vulkan_draw_scene` (1060) and `lower_tls_listen_macos`
  (1007); the designs are in letters S, T, U and V.
- `src/rules/table.rs` (1623 code LOC, one `const RULES: &[Rule]` literal) is split into
  parts like any other code file — owner rule, 2026-10-18. The design is in M.
- The guard is a set of test-only files and obeys its own 1000 limit (§4.1) — author.
- Per-file coverage is not a plan-176 gate — author, 2026-10-17. Measured at `7a4b09970`:
  `sh scripts/coverage.sh --bins` in a detached worktree (59 min; 4989 unit tests passed), then
  `sh scripts/coverage-check.sh` → exit 1. 241 `src/` files are already below the 98% per-file
  floor, and overall line coverage is 93.67%. A pure move changes no executed line, and moving
  an inline test block out only removes test lines from its host's denominator. That failure
  predates plan-176 and is reported to the owner separately. Letters do repoint
  `scripts/coverage-exceptions.txt` entries to the part files that carry the excepted code (L).
- `MFB_LOC_TIGHTEN` never adds or raises an entry — author (§3, rejected alternatives).

## Corrections

- 2026-10-18 — Rule revised by the owner: inline tests ≤ 250 in a code file, test-only files
  ≤ 1000 (was 250), code ≤ 1000, no exemptions. A now measures three kinds
  (`code`/`inline`/`test`), the ceiling file has no `exempt` rows and the oracle no `EXEMPT`
  table, and the seed is 199 rows (was 370). Re-measured at `ce7a995ca`. Letters B–F shrink
  to the 18 test-only files over 1000 plus `examples/money-demo/src/main.mfb`; G–K keep every
  inline move but split a moved block only when it is over 1000; M splits `src/rules/table.rs`.

## Summary

The engineering risk is in the Rust lexer's edge cases. Two independent implementations
agreeing over 4470 files, plus targeted cases, contain it. Nothing in the compiler, the
goldens or any test's behavior changes in A.
