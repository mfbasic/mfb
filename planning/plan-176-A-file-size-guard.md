# plan-176-A: File-size budget guard, oracle and ceilings

Last updated: 2026-10-14
Overall Effort: huge (>3d) — 22 sub-plans, A–V
Effort: large (3h–1d)
Depends on: nothing

plan-176 enforces one rule across the tree:

> No code file is over **1000 LOC** and no test file is over **250 LOC**, unless there
> is no logical place to split it. Comments and blank lines are not LOC. A split is done
> by creating a new module.

This sub-plan builds the enforcement and the measurement every later letter uses: a guard
test that counts LOC exactly, a checked-in ceiling file that can only go down, an independent
oracle that cross-checks the guard, and the written rule. When it is done, `cargo test --test
file_size_budget` prints a loud banner naming every file that is over its limit and not
covered by a ceiling or exemption. It also names every file that grew past its ceiling, and
every ceiling that can be lowered. Sub-plans B–V then bring the ceilings down until the file is
empty apart from its exemptions.

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
someone else's work in progress. At planning time, four letters have a file in the dirty tree.
Each of those letters lists the file in its own Prerequisites:
`src/codegen/registry/mod.rs` (H for its inline tests, M for its code),
`src/ir/verify/resources.rs` (N) and `tests/canvas/rt_canvas_group_ownership.rs` (D).
(`git status --porcelain` on 2026-10-14, intersected with `/tmp/loc/groups.json`.)

## 1. Goal

- `cargo test --test file_size_budget` exists, always passes, and writes a banner to the
  terminal (raw stderr, so libtest's capture does not swallow it) listing every violation row.
- `tests/guards/file_size_ceilings.txt` holds one ceiling per over-limit (file, kind) at its
  current count, plus the exemptions decided in this plan, each with its reason.
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

Code LOC = LOC − test LOC. Limits: code **1000**, test **250**.

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
`tools/loc-census/census.py` in Phase 1) run on the working tree of `17a9b5799`:
`python3 -I /tmp/loc/census.py "$PWD"` (22 s), then summarized from `census.json`.

| What | Count |
|---|---|
| In-scope `.rs` + `.mfb` files | 4438 |
| Generated (auto-exempt) | 9 |
| Files with code LOC > 1000 | 98 (97 `.rs`, 1 `.mfb`), 75,345 LOC over the limit |
| Test files with test LOC > 250 | 206 (180 `.rs`, 26 `.mfb`), 144,713 test LOC |
| Non-test files whose inline test LOC > 250 | 71 (all `.rs`), 59,868 inline test LOC |
| Ceiling rows to seed (code + test rows; 5 of the 206 test files are exemptions, see §Decisions) | 98 + 201 + 71 = 370 |

Largest offenders (same run): code `src/codegen/runtime/canvas/vulkan.rs` 7300,
`src/ir/lower.rs` 5140, `src/codegen/engine/control/builder_control.rs` 4392; test files
`src/ir/verify/tests.rs` 8465, `tests/runtime/rt_scope_drop_leaks.rs` 5476,
`tests/canvas/rt_canvas_rasteriser.rs` 4575; inline `repository/src/server.rs` 7487.

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
   with `--seed`, the ceiling-file body.
2. **Guard** — `tests/guards/file_size_budget.rs` (target root) plus submodules under
   `tests/guards/file_size_budget/`. It walks the scope, counts with its own Rust
   implementation of the same rules, compares against the ceiling file, writes the banner, and
   handles `MFB_LOC_TIGHTEN=1`.
3. **Ceiling file** — `tests/guards/file_size_ceilings.txt`.

Two independent implementations of the counting rules (Python oracle, Rust guard) agreeing on
4438 files is the correctness check for both. The risk is in the Rust lexer's edge cases (raw
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
  rubber stamp. New entries are added only by hand in review: an exemption with a reason, or
  never.

## 4. The guard

### 4.1 Files (each ≤ 250 test LOC, by its own count)

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
# tests/guards/file_size_ceilings.txt — plan-176. Limits: code 1000, test 250 (non-comment, non-blank LOC).
# ceiling <code|test> <n> <path>        an over-limit file may not grow past n; MFB_LOC_TIGHTEN=1 lowers n.
# exempt  <code|test> <path> # <reason> no logical place to split; reviewed by hand.
ceiling code 7300 src/codegen/runtime/canvas/vulkan.rs
ceiling test 748 src/codegen/runtime/canvas/vulkan.rs
exempt test tests/syntax/parser/parser_tgroup_depth/src/main.mfb # nests TGROUP to the parser's 256 statement-depth limit; the depth IS the test
```

Rows are sorted by path, then kind. `#` lines and blank lines are ignored. An `exempt` line
without a `# reason` is a parse error, reported as a banner row (the guard still does not fail).

### 4.3 Rows and the banner

For each in-scope, non-generated file and each kind (code, test), with `n` = count and `L` =
limit:

| Condition | Row |
|---|---|
| `n ≤ L`, no entry | — |
| `n ≤ L`, ceiling entry | `TIGHTEN` (entry now removable) |
| `n > L`, exempt entry | — |
| `n > L`, ceiling `c`, `n == c` | — |
| `n > L`, ceiling `c`, `n < c` | `TIGHTEN` (lower to `n`) |
| `n > L`, ceiling `c`, `n > c` | **`GREW`** |
| `n > L`, no entry | **`NEW`** |
| entry for a path that no longer exists, or an unparseable line | **`STALE`** |

If any row exists, the driver writes this to `std::io::stderr().lock()` with `write_all`:

```
################################################################################
##  FILE SIZE BUDGET (plan-176): 2 over, 1 grew, 3 can tighten                 ##
##  code ≤ 1000, test ≤ 250 non-comment LOC. Split into a new module.         ##
################################################################################
  NEW      code 1043 > 1000  src/codegen/foo.rs
  GREW     test  262 > 250 (ceiling 255)  tests/runtime/rt_x.rs
  TIGHTEN  code  4100 (ceiling 4392)  src/codegen/engine/control/builder_control.rs
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

- [ ] `tools/loc-census/census.py`: the planning oracle (`/tmp/loc/census.py` as of this plan)
      with the generated-banner rule and the `#[cfg(test)]` extent rule from §2, an `argparse` CLI
      (`census.py [--root DIR] [--json OUT] [--seed] [--file PATH [--lines A B]]`), stdlib only.
      `--file PATH` prints `code test` for one file. `--file PATH --lines A B` prints the LOC
      of lines A..B (1-based, inclusive). B–V use these modes to re-measure a part before
      and after a move. `--seed` prints the
      `ceiling` rows (sorted, §4.2 format) for every (file, kind) over its limit that is not
      generated and not in the `EXEMPT` table at the top of the script. That table holds
      exactly the §Decisions exemptions with their reasons, and they are printed as `exempt` rows.
- [ ] `tools/loc-census/README.md`: what it is (the independent oracle for
      `tests/guards/file_size_budget.rs`), the counting rules table from §2, usage, and that
      it is not a gate.
- [ ] `tests/guards/file_size_budget.rs` + `tests/guards/file_size_budget/{rust_lex,mfb_lex,classify,ceilings,lex_cases}.rs`
      per §4.1–4.4. A missing `file_size_ceilings.txt` is read as empty.
- [ ] `Cargo.toml`: `[[test]] name = "file_size_budget" path = "tests/guards/file_size_budget.rs"`
      in the `# --- tests/guards/ ---` block.

Acceptance: the guard and the oracle agree on exactly which (file, kind) pairs are over the
limit and on each one's count.
  Check: `cargo test --test file_size_budget 2>&1 | awk '$1=="NEW"{print $2, $3, $NF}' | sort > /tmp/g.txt`;
  `python3 -I tools/loc-census/census.py --seed | awk '$1=="ceiling"{print $2, $3, $4}' | sort > /tmp/o.txt`;
  `diff /tmp/g.txt /tmp/o.txt` → no output (est. 2 min). And `cargo test --test file_size_budget`
  → every `lex_cases` test passes, and the banner has 0 GREW, 0 STALE and 0 TIGHTEN rows.
Commit: —

### Phase 2 — Seed the ceilings

- [ ] `python3 -I tools/loc-census/census.py --seed > tests/guards/file_size_ceilings.txt`,
      then prepend the §4.2 header comment.
- [ ] `tests/guards/file_size_budget/ceilings.rs`: `MFB_LOC_TIGHTEN=1` rewrite (lower `n`,
      drop entries for files now within limit or missing, keep `exempt` rows of existing files
      verbatim, never add, never raise), with a `lex_cases.rs` case that feeds an in-memory
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

- [ ] `AGENTS.md`, "Always": one bullet — *File size: code ≤ 1000, test ≤ 250 non-comment
      LOC (`.rs`/`.mfb` under `src/ packages/ examples/ repository/ tests/`; inline
      `#[cfg(test)]`/`TESTING` count as test). Over → split into a new module by
      responsibility. `cargo test --test file_size_budget` prints a banner;
      `tests/guards/file_size_ceilings.txt` ceilings only go down
      (`MFB_LOC_TIGHTEN=1`). Exempt only when no logical split exists, by hand, with a
      reason. Generated files (`GENERATED` in the first three lines) are exempt. Mechanics:
      `.ai/build-tooling.md`.*
- [ ] `.ai/build-tooling.md`: new section "## Splitting an over-budget file (plan-176)" with
      the conventions every letter uses. Code: `foo.rs` → `foo/mod.rs` + parts, `mod.rs`
      re-exports the surface, ≤ 900 per part, pure motion, sweep `[[foo.rs:` citations and
      `tests/guards` allowlists, inherit `scripts/coverage-exceptions.txt` entries. Integration
      tests: root stays, `#[path = "<name>/<topic>.rs"] mod <topic>;`. src tests:
      `foo/tests/mod.rs`. Gates: `-- --list` count before == after; byte-identity for code.
      Then tighten.
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
- Inline test code counts against the 250 test budget, not the file's 1000. Over 250, it moves
  out to its own module, and that module splits further as needed — owner, 2026-10-14.
- Generated files are exempt — owner, 2026-10-14. Detection is the `GENERATED` banner (§2).
- Over-limit files produce a loud banner, not a failure — owner, 2026-10-14.
- Splitting by making a new module is always acceptable, for code and tests — owner, 2026-10-14.
- Exemptions (author, 2026-10-14). The input text itself is the subject of each of these
  tests, so there is no logical split:
  - `tests/syntax/parser/parser_tgroup_depth/src/main.mfb`,
    `tests/syntax/parser/parser_inline_trap_depth/src/main.mfb`,
    `tests/syntax/parser/parser_statement_block_depth/src/main.mfb` — each nests one
    construct to the parser's 256 statement-depth limit; a split file cannot nest across files.
  - `tests/codegen/regalloc_eviction/float_fixed.mfb` — `include_str!`'d whole by
    `tests/codegen/codegen_regalloc_eviction.rs:FLOAT_FIXED` as a single `main.mfb`. It is the
    benchmark body verbatim (bug-760 C), compiled for its whole-function register pressure.
  - `tests/canvas/scenes/frame_skip_nested_lists.mfb` — a single-source scene shared with
    `scripts/test-canvas-gpu-rows.sh`, which compiles each `scenes/*.mfb` as one file.
  - Each further exemption below is recorded in the Decisions of the letter that creates the
    file, *and* added to `file_size_ceilings.txt` and the oracle's `EXEMPT` table in that
    letter's commit.
- A single **function** over the code limit is decomposed by extract-method refactoring, gated
  by byte-identical `.ncode` on every target — decided by the owner, 2026-10-14. This covers
  `lower_module_for_platform` (1801), canvas `register` (2093), `lower_ops_inner` (1313),
  `lower_value_inner` (1224), `emit_vulkan_draw_scene` (1060) and `lower_tls_listen_macos`
  (1007); the designs are in letters S, T, U and V.
- A single **test-side unit** over 250 LOC gets its own file and an `exempt` row: an embedded
  MFBASIC program, a const table or fixture list, one `#[test]` fn, or one trait impl — decided
  by the owner, 2026-10-14. The input is what the test is about. Natural extractions are still
  done first: a raw-string program moves to an `include_str!` `.mfb` file (D), and nested helper
  fns are hoisted out of a long test (D). The exempt files, by letter:
  - B: `rt_big_int/powers.rs`, `rt_closure_single_owner/cases.rs` (`rt_big_int/word_boundaries.rs`
    is not exempt: its program moves to an `include_str!` `.mfb` file, which takes both files under 250)
  - C: `rt_resource_last_holder/map_iteration.rs`, `rt_self_update_cost/value_program.rs`
  - E: `cli_repo_install/garbage_collection.rs`, `cli_repo_publish/signing_and_metadata.rs`
  - F: `tests/rt-behavior/lexical/literal-spans-lines-valid/src/main.mfb`
  - H: `builtins/tests/corpus/fixture_list.rs`, `builtins/tests/diagnostics/fixture_list.rs`,
    `builtins/tests/platform_failure/forwarding.rs`
  - J: the `SELF_UPDATE_TABLE` file, the aarch64 decode `table()` file,
    `src/codegen/io/input/corpus.rs`, and the vulkan `layout_tables_are_self_consistent` file
- `src/rules/table.rs` (1617 code LOC) is exempt — author, 2026-10-14. It is one
  `const RULES: &[Rule]` data literal, and `Rule` is not `Copy`, so it cannot be concatenated
  from parts without new code. It is data, not a function, so the extract-method decision does
  not reach it. The exempt row is added in M.
- The guard is a test file and obeys its own 250 limit (§4.1) — author.
- Per-file coverage is not a plan-176 gate — author, 2026-10-17. Measured at `7a4b09970`:
  `sh scripts/coverage.sh --bins` in a detached worktree (59 min; 4989 unit tests passed), then
  `sh scripts/coverage-check.sh` → exit 1. 241 `src/` files are already below the 98% per-file
  floor, and overall line coverage is 93.67%. A pure move changes no executed line, and moving
  an inline test block out only removes test lines from its host's denominator. That failure
  predates plan-176 and is reported to the owner separately. Letters do repoint
  `scripts/coverage-exceptions.txt` entries to the part files that carry the excepted code (L).
- `MFB_LOC_TIGHTEN` never adds or raises an entry — author (§3, rejected alternatives).

## Corrections

## Summary

The engineering risk is in the Rust lexer's edge cases. Two independent implementations
agreeing over 4438 files, plus targeted cases, contain it. Nothing in the compiler, the
goldens or any test's behavior changes in A.
