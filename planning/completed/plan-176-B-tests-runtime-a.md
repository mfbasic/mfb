# plan-176-B: Split tests/runtime, part 1 (4 integration-test files)

Last updated: 2026-10-18
Effort: medium (1h–2h)
Depends on: plan-176-A

This sub-plan splits four test-only integration-test files under `tests/runtime/` that are over
the 1000-LOC test-only-file limit: `rt_big_int.rs`, `rt_debug_soak.rs`, `rt_native_term_runtime.rs`
and `rt_inplace_self_update.rs`. Together they hold 6,472 test LOC and 93 `#[test]` fns
(`python3 -I /tmp/loc/census.py "$PWD"` at `ce7a995ca`, field `test`; test counts by
`grep -cE '#\[test\]' <file>`). Each file keeps its root as the `[[test]]` target and gains one to
three parts under `tests/runtime/<name>/`, declared with `#[path]`, the layout plan-176-A verified.

Three of the files are pure code motion: no test is added, removed or renamed, and no assertion
text changes. `rt_big_int.rs` cannot be pure motion. Its test
`powers_gcd_factorial_and_mod_pow_match_an_independent_oracle` is one `#[test]` of 1,271 LOC, and
1,170 of those LOC are an unrolled grid of 390 `modPow`-against-`remainder(pow)` cases. Moving the
program into an `.mfb` under `tests/` would only produce a test-only file of the same size. So the
grid moves verbatim into two new `#[test]`s of 195 cases each. Every assertion is kept,
`rt_big_int`'s test count goes from 17 to 19, and §4.1 gives the byte checks that prove nothing
was dropped.

**Single outcome:** after B lands, `cargo test --test file_size_budget` prints no row for any
path under `tests/runtime/` that this letter owns. `tests/guards/file_size_ceilings.txt` has no
row for them, and every file B creates or leaves is ≤ 900 test LOC.

References:

- `planning/plan-176-A-file-size-guard.md`: the rule (three kinds: `code` ≤ 1000, `inline`
  ≤ 250, `test` ≤ 1000), the counting rules, the guard, the
  `ceiling <code|inline|test> <n> <path>` file format, the `MFB_LOC_TIGHTEN` ratchet, and the
  integration-test layout (`#[path = "<name>/<topic>.rs"] mod <topic>;`, verified there in §2
  "Verified properties", the `rt_demo` probe).
- `.ai/build-tooling.md` § "Splitting an over-budget file (plan-176)" (written by A Phase 3):
  the split conventions this letter applies.
- `.ai/testing-gates.md`: `:95` names `rt_inplace_self_update`'s ignored grid tests, and `:883`
  names `rt_debug_soak.rs`'s `BLOCK_BOUND` (`grep -n 'rt_inplace_self_update\|rt_debug_soak' .ai/testing-gates.md`).
  Both stay valid (§4.0). Line 95 also has a pre-existing miscount that B fixes (§4.4).
- `.ai/collections.md:92` names the `rt_inplace_self_update.rs` root path, which stays valid.
- Spec topics: none. No `[[tests/runtime/<file>.rs:…]]` citation names a B file, and no spec or
  man prose names one (`grep -rn 'rt_big_int\.rs\|rt_debug_soak\|rt_native_term_runtime\|rt_inplace_self_update' src/docs` → no output).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-A is complete and archived | `ls planning/plan-176-A-* planning/completed/plan-176-A-*` → only `planning/completed/plan-176-A-file-size-guard.md` | MET (2026-10-18: `ls planning/completed/` lists `plan-176-A-file-size-guard.md`; `planning/plan-176-A-*` matches nothing) |
| The guard and the oracle exist (A Phases 1–2) | `cargo test --test file_size_budget` → passes; `python3 -I tools/loc-census/census.py --file tests/runtime/rt_big_int.rs` → `0 2584` | MET (2026-10-18: `test result: ok. 35 passed`; oracle → `0 2584`) |
| This letter's files have no uncommitted changes | `git status --porcelain -- tests/runtime .ai/testing-gates.md tests/guards/file_size_ceilings.txt` → empty | MET (2026-10-18: empty output) |

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

- Every file of this letter, and every file it creates, is ≤ 1000 test LOC by
  `python3 -I tools/loc-census/census.py --file <path>`, and each new part is ≤ 900 (the headroom
  target). No exemption exists.
- For `rt_debug_soak`, `rt_native_term_runtime` and `rt_inplace_self_update`, the sorted list of
  test names from `cargo test --test <t> -- --list` is identical before and after, once module
  prefixes are stripped. For `rt_big_int`, the after list is the before list plus exactly
  `mod_pow_matches_remainder_of_pow_for_bases_up_to_zero` and
  `mod_pow_matches_remainder_of_pow_for_positive_bases`.
- Every target passes `cargo test --test <t>`, and `rt_inplace_self_update`'s ignored grids pass
  `-- --ignored`.

### Non-goals (explicit constraints)

- No behavior change. No assertion, expected-output literal, MFBASIC program line or
  `#[ignore]` reason is altered or removed. Only these text edits are allowed inside moved code:
  - `pub(super)` on the support items of `rt_inplace_self_update/cases.rs` (§4.4 "Surface");
  - the `rt_big_int` grid split (§4.1). Its 1,170 case lines move verbatim into two programs, and
    the total "0 of 390" becomes "0 of 195" in each half. The oracle test's doc comment stops
    mentioning the grid.
- No `[[test]]` target is added, renamed or re-pathed. `Cargo.toml` is not edited.
- No existing test fn is renamed. A test path may gain a module prefix
  (`console::native_term_on_resets_state_to_defaults`). Substring filters such as
  `cargo test --test rt_native_term_runtime mouse` and `--ignored` selection keep working.
- No test moves between targets, and no helper is promoted into `tests/common/`.

## 2. Current State

### Files and measured sizes

Test LOC per file are from the planning oracle at `ce7a995ca`
(`python3 -I /tmp/loc/census.py "$PWD"` → `/tmp/loc/census.json`, field `test`; after A it is
`python3 -I tools/loc-census/census.py --file <path>`). Test counts are
`grep -cE '#\[test\]' <file>`. No file uses another test attribute.

| File | Test LOC | Tests | New files | Largest file after |
|---|---|---|---|---|
| `tests/runtime/rt_big_int.rs` | 2584 | 17 → 19 | 3 | 809 (root) |
| `tests/runtime/rt_debug_soak.rs` | 1164 | 49 | 1 | 723 (root) |
| `tests/runtime/rt_inplace_self_update.rs` | 1572 | 4 | 1 | 813 (`cases.rs`) |
| `tests/runtime/rt_native_term_runtime.rs` | 1152 | 23 | 1 | 724 (root) |
| **Total (4 files)** | **6,472** | **93 → 95** | **6** | |

Part sizes come from `python3 -I /tmp/loc/span.py <file> <a> <b>`, summed over each part's line
ranges, plus the lines the split adds (§3). The `rt_big_int` powers parts were measured by building
them in `/tmp/loc/rw-B/` (`powers.py`) and counting with `loclib.rust_count`. Line ranges are as
of `ce7a995ca`. `rt_big_int.rs` and `rt_inplace_self_update.rs` are unchanged since `17a9b5799`.
`rt_debug_soak.rs` changed one line in place (`9f378fcbe`). `rt_native_term_runtime.rs` gained
lines 1300–1437 (three tests, `d7a0a1f08` and `054062ce3`), and its lines 1–1299 are unchanged
(`git diff 17a9b5799 HEAD -- <file> | grep '^@@'` → one hunk, `@@ -1297,3 +1297,141 @@`).

### Verified properties

- **The `#[path]` layout works for integration-test targets and keeps name filters.**
  plan-176-A §2 probe: `cargo test --test rt_demo -- --list` listed `lists::list_a: test`, and
  `cargo test --test rt_demo list_a` ran 1 test.
- **A part reaches the root's imports, `common` helpers, private root items and support globs
  through `use super::*;` with no warnings.** Probe `/tmp/loc_bcd_probe`: a `[[test]]` root with
  `mod common; use common::*; use std::collections::HashMap;`, a topic using `HashMap`, a bare
  `helper()` from `common` and `CASES` from a sibling support file, and a root `use data::*;`.
  `cargo test --test t1` → `test result: ok. 1 passed`, with no warning. A root import that only a
  topic uses is not reported unused, and neither is a root support glob that only a topic uses.
- **Glob cycles compile without warnings.** Probe `/tmp/loc/probe2`
  (`mod.rs: use a::*; use b::*;`, `a.rs`/`b.rs: use super::*;`) printed `6` with no warning.
- **Over-broad `pub(super)` does not warn.** The workspace `[lints]` table holds only
  `clippy::items_after_test_module` (`grep -n -A2 '^\[lints' Cargo.toml`), and `unreachable_pub` is
  allow-by-default. A missing `pub(super)` is a compile error (E0603/E0616), so it cannot pass
  silently.
- **`tests/guards/test_targets_registered.rs` ignores `tests/runtime/<name>/*.rs`.**
  `test_files_in_subdirectories` reads only the direct `.rs` children of each `tests/<dir>/`
  (plan-176-A §2). No part file is named `main.rs`.
- **Every B test compiles on the macOS development host,** so `-- --list` there counts every
  test. `grep -nE '#!?\[cfg' <the 4 files>` finds only these:
  - `#![cfg(unix)]` (rt_debug_soak, rt_inplace_self_update);
  - `#[cfg(target_os = "macos")]` (rt_debug_soak:906, which stays in the root);
  - `#[cfg(unix)]` / `#[cfg(not(unix))]` statements inside rt_native_term_runtime test bodies
    (:73, :620, :782, :787), which move verbatim inside their fns;
  - `#[cfg(unix)]` on `app_term_poll_mouse_decodes_without_a_prior_read_and_keeps_the_keys`
    (:1408), which stays in the root.

  No remote box is needed.
- **Only rt_debug_soak and rt_inplace_self_update read files by path.**
  `grep -nE 'include_str|include_bytes|CARGO_MANIFEST_DIR' <the 4 files>` finds:
  - rt_debug_soak:35 `env!("CARGO_MANIFEST_DIR")`, which is absolute and stays in the root;
  - rt_inplace_self_update:103–105, the three `include_str!("inplace_self_update/*.tsv")`;
  - rt_inplace_self_update:696, `include_str!("../../src/codegen/collection/assign/self_update.rs")`.

  All four rt_inplace_self_update includes stay in the root (§4.4), so no path changes.
- **Ignored tests:** `rt_inplace_self_update` has three `#[ignore = "slow: run by coverage.yml; …"]`
  grid tests (`grep -c '^#\[ignore' tests/runtime/rt_inplace_self_update.rs` → `3`). The other three
  files have none (`grep -c '^#\[ignore' …` → `0`).
  `.github/workflows/coverage.yml:180,185` and `scripts/test-full.sh:114` run
  `cargo test [--release] --test rt_inplace_self_update -- --ignored`, and the target name and the
  `--ignored` selection are unchanged. `grep -rn -- --exact .github scripts` finds no use on any
  B test.
- **No B file is in `scripts/coverage-exceptions.txt` or a `tests/guards` allowlist.**
  `grep -n <name> scripts/coverage-exceptions.txt tests/guards/{architecture_guards,no_type_strings,no_operator_strings,inplace_self_update_census}.rs`
  → only the `//!` doc of `inplace_self_update_census.rs:6`, which names the root path that stays.
- **No new directory collides.** `ls -d tests/runtime/{rt_big_int,rt_debug_soak,rt_native_term_runtime,rt_inplace_self_update}`
  → `No such file or directory` for all four. The existing `tests/runtime/inplace_self_update/`
  (TSV data) is not the new `tests/runtime/rt_inplace_self_update/`.
- **The rt_big_int grid is regular.** The 390 cases are 6 bases {-9, -2, 0, 1, 3, 250} × 13
  exponents 0..12 × 5 moduli {-17, -1, 2, 97, 256}, in base-major order. Each case is the same
  3-line `IF NOT big::equals(big::modPow(…), big::remainder(big::pow(…), …)) THEN` /
  `modPowMismatches = modPowMismatches + 1` / `END IF`, and the two sides of every case use the
  same triple (parsed in `/tmp/loc/rw-B/` → `390`, all `True`). Bases ≤ 0 are lines 1077–1661
  (195 cases), and bases > 0 are lines 1662–2246 (195 cases).

## 3. Design Overview

**Conventions (every file).**

- The root `tests/runtime/<name>.rs` stays the `[[test]]` target. It keeps its module doc,
  `#![cfg(…)]`, `#[path = "../common/mod.rs"] mod common;` (unchanged: the root does not move),
  its `use` lines, and everything §4 does not move out. It gains one
  `#[path = "<name>/<m>.rs"] mod <m>;` pair per new file (2 LOC each).
- Parts are regrouped for the ≤ 900 headroom target with as few files as possible. The root
  keeps the shared helpers and the topics nearest them, and the rest goes to one or more parts cut
  at topic boundaries.
- Every part starts with `use super::*;`. Through that glob it reaches `common`, the root's
  imports and helpers, and support globs. Private parent items are visible to child modules, so no
  root item changes visibility.
- The single support file (`rt_inplace_self_update/cases.rs`) gets `pub(super)` on the items,
  fields and methods named under its "Surface" line. The root adds `use cases::*;` (1 LOC).
- Moved code is copied verbatim, including doc comments and attributes (`#[ignore = "…"]`,
  `#[cfg(…)]`), in source order within each file. Item and test NAMES are authoritative if lines
  drift.

**Gate class: test-motion,** plus a byte-check for the one designed split. The proof that nothing
was lost has three parts: the per-target list of test names, stripped of module prefixes and
sorted, compared before and after; a passing run of exactly the moved targets; and, for
`rt_big_int`, the §4.1 byte checks. Byte-identity of compiler output does not apply, because no
compiler code changes. The gate kit used by every phase follows. `$T` is the phase's target list
and `$RE` its path regex, both given in each phase.

- **K1 list before** (run on the tree before the phase's first edit):
  `for t in $T; do cargo test -q --test $t -- --list 2>/dev/null | grep ': test$' | sed -E 's/^([a-z0-9_]+::)+//' | sort > /tmp/176B-$t.list; done; cat $(for t in $T; do echo /tmp/176B-$t.list; done) | wc -l`
  → the phase's test total.
- **K2 list after**:
  `for t in $T; do cargo test -q --test $t -- --list 2>/dev/null | grep ': test$' | sed -E 's/^([a-z0-9_]+::)+//' | sort | diff -q /tmp/176B-$t.list - >/dev/null || echo "DIFF $t"; done`
  → no output. (Phase 2 replaces this with K2′, §4.1.)
- **K3 run**: `cargo test $(for t in $T; do printf -- '--test %s ' $t; done)` → every target
  `test result: ok`, `0 failed`.
- **K4 warnings**: `cargo check --all-targets 2>&1 | grep -E -- "--> tests/runtime/$RE(\.rs|/)"` → no output.
- **K5 budget**: `cargo test --test file_size_budget 2>&1 | grep -E "tests/runtime/$RE(\.rs|/)"` → only
  `TIGHTEN` rows (one per root in the phase, each reporting its `ceiling test` entry as removable),
  and no `NEW`/`GREW`/`STALE` row. Then `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`.
  Then `git diff -U0 tests/guards/file_size_ceilings.txt | grep -E '^[-+]ceiling' | grep -vE "tests/runtime/$RE(\.rs|/)"`
  → no output (the rewrite touched only this phase's rows). Re-running the first grep → no output.

**Where the risk concentrates.**

- *A test that silently stops running.* This happens if a `#[path]` is missing, because a file
  that is not declared is not compiled. K1/K2 catch it.
- *A test that silently stops checking something.* This is the rt_big_int grid split, where a
  dropped case line would still pass. §4.1's byte checks compare all 1,170 case lines and the
  oracle body against `HEAD`, so a lost line is a diff.
- *A test that reads a file by path.* rt_inplace_self_update does. Its four `include_str!`s stay
  in the root, so their paths do not change.
- *The largest surface.* rt_inplace_self_update's split (one support file, 22 `pub(super)` items)
  comes last.

**Rejected alternatives.**

- *Promote shared helpers into `tests/common/`*. That changes 338 includers' compile surface
  (letter E owns `tests/common/`) and is not motion.
- *One `[[test]]` target per topic*. That changes target names, breaks `cargo test --test <name>`
  in docs, CI (`coverage.yml`) and scripts, and adds link time per binary.
- *`foo/mod.rs` with the root moved into the directory*. Integration-test roots must stay
  `tests/<dir>/<name>.rs` for `test_targets_registered.rs`
  (`a_registered_target_is_named_after_its_file`) and for the unchanged `[[test]]` stanza.
- *Move the powers program into an `.mfb` file.* A `.mfb` under `tests/` is a test-only file, so
  the 1,271-LOC unit stays over 1000.
- *Split the powers program across two `.mfb` fragments joined by
  `concat!(include_str!(…), include_str!(…))`.* Each fragment would be a file that is not a
  program on its own, and the grid would still run as one test that reports one count. Two
  self-contained programs keep every file a valid program and say which half failed.
- *Generate the 390 cases from a Rust table of triples.* The program text would no longer be
  reviewable as written, and the generator would be new logic in the test. The verbatim split
  keeps every case line byte-identical, and a `diff` checks that.
- *One small file per topic.* Each extra file adds a `#[path]` pair and a `use super::*;` and
  buys nothing under a 1000-LOC limit.

## 4. Detailed Design

### 4.0 Sweeps common to every file

- Spec citations: `grep -rnoE "\[\[tests/runtime/(rt_big_int|rt_debug_soak|rt_native_term_runtime|rt_inplace_self_update)\.rs[^]]*\]\]" src/docs .ai`
  → 0. `spec_citations_resolve` is not affected, because no cited path moves.
- `.ai/`, `scripts/`, `.github/` and `Cargo.toml` references name root paths, target names or
  root-resident symbols only. Each is listed under its file's "Sweeps" line, and all stay valid
  because roots do not move.
- Format of each entry: the layout (new path, LOC, contents, source line ranges as of
  `ce7a995ca`), then "Surface" (visibility edits and root globs), then "Sweeps".

#### 4.1 `tests/runtime/rt_big_int.rs` — 2584 test LOC, 17 → 19 tests

- `tests/runtime/rt_big_int.rs` (809): header, helpers `run` and `SHOW_BYTES`, and 12 tests:
  a_defaulted_int_is_canonical_zero, integer_round_trip_and_overflow_edges,
  byte_round_trips_in_both_orders, compare_is_a_total_order_and_the_predicates_agree,
  negate_abs_and_the_total_decoder, additive_carry_and_borrow_chains,
  additive_identities_and_integer_oracle, multiply_matches_an_independent_oracle, bit_operations,
  text_matches_an_independent_oracle, a_thousand_term_sum_prints_the_independent_total,
  crypto_random_int_big_overload_is_in_range_and_uniform [lines 1-777, 2296-2387]; mods
  division_and_word_boundaries, powers, mod_pow_grid.
- `tests/runtime/rt_big_int/division_and_word_boundaries.rs` (511): tests (4)
  division_targeted_cases, division_property_over_ten_thousand_seeded_pairs,
  division_members_agree_and_raise, word_boundaries_match_an_independent_oracle [lines 778-1018, 2389-2686].
  The word-boundaries program stays inline. Its test is 288 LOC
  (`python3 -I /tmp/loc/span.py tests/runtime/rt_big_int.rs 2389 2686` → 288), well within the
  limit, so it needs no extraction.
- `tests/runtime/rt_big_int/powers.rs` (699), with two tests:
  - (98) `powers_gcd_factorial_and_mod_pow_match_an_independent_oracle` [lines 1020-2294]. It
    keeps its name, project name `big_c_powers` and every line of its program and expected
    vector, except four deletions that move to the grid tests:
    - line 1076 `MUT modPowMismatches AS Integer = 0`;
    - the 1,170 case lines 1077–2246;
    - line 2247, the `io::print("modPow vs remainder(pow): " … " of 390")`;
    - line 2289, the expected entry `"modPow vs remainder(pow): 0 of 390",`.

    The doc comment's clause "`modPow` against `remainder(pow)` over 390 small signed cases;"
    becomes "the 390-case `modPow`-against-`remainder(pow)` grid is the two
    `mod_pow_matches_remainder_of_pow_*` tests;".
  - (600) the new `mod_pow_matches_remainder_of_pow_for_bases_up_to_zero` (project name
    `big_c_mod_pow_grid_up_to_zero`). It calls `run` on this program:
    - `IMPORT io` / `IMPORT big` / blank / `FUNC main() AS Nothing`;
    - `  MUT modPowMismatches AS Integer = 0`;
    - lines 1077–1661 verbatim (bases -9, -2, 0: 195 cases);
    - `  io::print("modPow vs remainder(pow): " & toString(modPowMismatches) & " of 195")`;
    - `END FUNC`.

    It then runs `assert_eq!(lines, vec!["modPow vs remainder(pow): 0 of 195"]);`. Its doc comment
    names plan-127-C Phase 3 and the bases.
- `tests/runtime/rt_big_int/mod_pow_grid.rs` (601): test (1)
  `mod_pow_matches_remainder_of_pow_for_positive_bases` (project name
  `big_c_mod_pow_grid_positive`). It has the same shape as the test above, with lines 1662–2246
  verbatim (bases 1, 3, 250: 195 cases). The two grid tests cannot share a file (600 + 600 > 900).

Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).
Indivisible-unit disposition: the powers test (one 1,271-LOC `#[test]`) is split into three
`#[test]`s (98 + 600 + 600 LOC), with no exemption. The test count of `rt_big_int` goes from 17
to 19. All 390 cases are kept, in the same order,
byte for byte. "0 of 390" becomes two "0 of 195" assertions, which pass exactly when the old one
did.

**Byte checks** (run after the split, `HEAD` = the tree before Phase 2):
- Case lines: `diff <(git show HEAD:tests/runtime/rt_big_int.rs | grep -A2 '^  IF NOT big::equals(big::modPow' | grep -v '^--$') <(cat tests/runtime/rt_big_int/powers.rs tests/runtime/rt_big_int/mod_pow_grid.rs | grep -A2 '^  IF NOT big::equals(big::modPow' | grep -v '^--$')`
  → no output, and the right side piped to `wc -l` → `1170`.
- Oracle body: `diff <(git show HEAD:tests/runtime/rt_big_int.rs | sed -n '/^fn powers_gcd_factorial/,/^}/p' | grep -vE 'modPowMismatches|modPow vs remainder|^  IF NOT big::equals\(big::modPow|^  END IF$') <(sed -n '/^fn powers_gcd_factorial/,/^}/p' tests/runtime/rt_big_int/powers.rs)`
  → no output. Probed on the planned text in `/tmp/loc/rw-B/oracle.rs`: the only difference was
  the scratch file's missing trailing newline.
- **K2′** (replaces K2 for `rt_big_int`):
  `diff /tmp/176B-rt_big_int.list <(cargo test -q --test rt_big_int -- --list 2>/dev/null | grep ': test$' | sed -E 's/^([a-z0-9_]+::)+//' | sort) | grep '^[<>]'`
  → exactly two lines,
  `> mod_pow_matches_remainder_of_pow_for_bases_up_to_zero: test` and
  `> mod_pow_matches_remainder_of_pow_for_positive_bases: test` (no `<` line: no old test is
  lost or renamed).

#### 4.2 `tests/runtime/rt_debug_soak.rs` — 1164 test LOC, 49 tests

- `tests/runtime/rt_debug_soak.rs` (723): header; helpers FLAT_BOUND, BLOCK_BOUND,
  main_live_bytes, assert_live_bytes_flat, assert_live_bytes_within, main_arena_calls, debug_run,
  assert_block_flat; and 20 tests with their items, as follows:
  - browser (3): a_dom_parse_loop_keeps_live_bytes_constant, a_resolve_styles_loop_keeps_live_bytes_constant, a_paint_loop_keeps_live_bytes_constant; items repo_root, copy_tree, install_browser_package, browser_project.
  - text parsing (3): a_flat_split_loop_keeps_live_bytes_constant, a_json_parse_loop_keeps_live_bytes_constant, a_thread_copy_back_loop_keeps_live_bytes_constant; items write_split_input, write_json_input.
  - network (5): an_http_read_loop_keeps_live_bytes_constant, a_tcp_read_loop_keeps_live_bytes_constant, a_tcp_connect_close_loop_keeps_live_bytes_constant, a_udp_bind_close_loop_keeps_live_bytes_constant, an_http_read_loop_leaves_no_block_behind; items serve_http, serve_tcp.
  - TLS (2): a_tls_connect_close_loop_keeps_live_bytes_constant, a_tls_socket_closed_on_another_thread_keeps_live_bytes_constant; items have_openssl_peer, serve_tls.
  - attributed strings (5): a_bound_attributed_string_keeps_live_bytes_constant, a_list_of_attributed_strings_keeps_live_bytes_constant, a_record_of_attributed_strings_keeps_live_bytes_constant, an_attributed_string_with_an_attribute_keeps_live_bytes_constant, a_defaulted_record_keeps_live_bytes_constant; item astrings_project.
  - crypto (2): a_native_ec_generate_loop_keeps_live_bytes_constant, a_software_curve_generate_loop_keeps_live_bytes_constant; item crypto_generate_project.

  The root keeps lines 1-764, 806-829, 897-935 and 991-1022; everything outside the part below
  stays. Mod: resource_loops.
- `tests/runtime/rt_debug_soak/resource_loops.rs` (444): 29 tests, as follows:
  - resource ownership (6): a_resource_union_bound_from_a_producer_keeps_live_bytes_constant, a_resource_union_aliasing_a_binding_keeps_live_bytes_constant, a_resource_passed_through_a_function_is_freed_once, a_returned_union_wrapping_an_owned_local_stays_open_in_the_caller, a_union_alias_in_an_inner_scope_leaves_the_outer_handle_open, a_resource_not_returned_on_a_sibling_path_is_still_closed; item run_with_fd_limit.
  - trap and state (10): a_trap_bound_resource_union_keeps_live_bytes_constant, a_trap_bound_concrete_resource_keeps_live_bytes_constant, a_trap_bind_whose_producer_fails_keeps_live_bytes_constant, a_trapped_borrowed_get_keeps_live_bytes_constant, a_trapped_borrowed_poll_with_an_owned_recover_keeps_live_bytes_constant, a_stateful_resource_union_keeps_live_bytes_constant, a_stateful_concrete_resource_keeps_live_bytes_constant, an_in_place_state_scalar_update_keeps_live_bytes_constant, an_owned_list_of_resource_unions_keeps_live_bytes_constant, an_owned_list_of_concrete_resources_keeps_live_bytes_constant.
  - threads (10): a_thread_resource_transfer_loop_keeps_live_bytes_constant, a_thread_string_send_loop_keeps_live_bytes_constant, a_thread_send_of_a_computed_message_keeps_live_bytes_constant, a_worker_send_of_a_computed_message_keeps_live_bytes_constant, an_undelivered_queued_message_is_freed_when_the_thread_is_released, a_stateful_resource_transfer_frees_its_record_and_its_state, a_thread_start_seed_is_freed_when_the_thread_is_released, a_thread_start_with_a_literal_seed_frees_nothing, a_thread_start_with_a_local_seed_leaves_it_to_its_binding, a_failed_thread_start_frees_its_seed.
  - record frees (3): an_fs_file_binding_frees_its_resource_record, an_fs_file_bound_through_an_inline_trap_frees_both_records, an_unfloated_list_of_res_frees_its_collection_block.

  Lines: 765-805, 830-896, 936-990, 1023-1520. The four blocks are contiguous apart from blank
  lines, and `span.py` over them → 443.

Surface: none (no support file; root keeps every shared helper).
Sweeps:
- `.ai/testing-gates.md:883` cites `tests/runtime/rt_debug_soak.rs`'s `BLOCK_BOUND`. It stays in
  the root, so the citation stays valid.
- `repo_root()` (`env!("CARGO_MANIFEST_DIR")`, line 35) and its use for
  `tests/runtime/data/rt_debug_soak_basic.html` stay in the root and are absolute.
- The module-doc command `cargo test --release --test rt_debug_soak -- --include-ignored` is
  unaffected.

#### 4.3 `tests/runtime/rt_native_term_runtime.rs` — 1152 test LOC, 23 tests

- `tests/runtime/rt_native_term_runtime.rs` (724): header; helpers mouse_decoder_source,
  mouse_poll_then_read_source, run_with_held_stdin, POLL_BEFORE_READ_SOURCE, POLL_ONLY_SOURCE; and
  14 tests:
  - native_term_mouse_is_opt_in_and_silent_until_asked
  - native_term_mouse_decodes_every_kind_and_modifier
  - native_term_mouse_leaves_other_input_untouched
  - native_term_mouse_poll_input_keeps_the_byte_after_an_escape
  - native_term_mouse_escape_at_end_of_input_is_delivered
  - native_term_mouse_lone_escape_is_reported_without_a_following_key
  - native_term_mouse_ring_overwrites_oldest_and_expires_stale
  - native_term_mouse_keeps_immediate_reads_honest
  - native_term_mouse_line_read_keeps_the_bytes_after_an_escape
  - native_term_mouse_does_not_echo_during_a_line_read
  - native_term_mouse_is_per_thread_and_needs_stdin
  - native_term_poll_mouse_decodes_without_a_prior_read_and_keeps_the_keys
  - native_term_poll_mouse_needs_no_io_reader_in_the_program
  - app_term_poll_mouse_decodes_without_a_prior_read_and_keeps_the_keys (`#[cfg(unix)]`, kept
    on the fn)

  Lines: 1-512 and 1029-1437. Mod: console.
- `tests/runtime/rt_native_term_runtime/console.rs` (431): tests (9)
  native_term_off_withdraws_mouse_tracking,
  native_term_size_reports_unsupported_off_and_size_when_active,
  native_term_console_emits_expected_escape_sequences, native_term_gate_no_ops_while_inactive,
  native_term_did_resize_reports_false_without_a_resize, native_term_on_resets_state_to_defaults,
  native_term_draw_text_attributed_applies_bold_underline,
  native_term_draw_text_attributed_applies_colors,
  native_term_draw_text_attributed_ignores_alpha [lines 513-1028, one contiguous block]. The
  in-body `#[cfg(unix)]`/`#[cfg(not(unix))]` statements at :620, :782 and :787 move with their fns.

Surface: none (no support file). Every helper the console tests use comes from `common` or the
root's `use` lines through `use super::*;`. The mouse helpers stay in the root beside their only
users.
Sweeps: none (§4.0 commands: 0 hits).

#### 4.4 `tests/runtime/rt_inplace_self_update.rs` — 1572 test LOC, 4 tests

- `tests/runtime/rt_inplace_self_update.rs` (763): header; consts DEFAULT_N, EXEMPT_M, VALUE_M,
  CASES, FIELD_EXPECT, FIELD_KINDS [lines 1-106]; the test scratch_arms_match_the_compiler
  [lines 689-717]. Then the plan-145-A field-site machinery [lines 1222-1964]:
  - FieldSite, FIELD_SITES, impl FieldSite, FieldExpect, impl FieldExpect, field_expectations;
  - FieldCase, impl FieldCase, field_kinds, kind_helpers;
  - field_frame, at_field_site, field_statement_lines, field_prologue, field_program,
    field_result_program, field_extra;
  - check_field, site_filter, label_filter, Work, run_work.

  Then the three ignored grid tests, every_field_kind_meets_its_expectation,
  every_self_update_case_meets_its_allocation_bound and
  every_via_let_field_cell_meets_its_expectation, with item VIA_LET_SAMPLE [lines 1966-2075].
  Mod: cases, plus root `use cases::*;`.
- `tests/runtime/rt_inplace_self_update/cases.rs` (support, 813; starts with `use super::*;`
  because it reads root consts and `common`). It holds the self-update case grid, its program
  builders and its build/run/check harness [lines 107-688, 718-1221]:
  - PRELUDE, Status, Case, impl Case, len_of, extra_imports, prelude_for, cases;
  - STDIN_LINES, output_with_stdin;
  - Site, ENABLED_SITES, impl Site, frame, at_site, sized, return_program;
  - SCRATCH_ARMS, statement_target, case_needs_scratch, owned_param_program, program,
    replace_ident, result_program;
  - exempt_program, exempt_check, run_peak, build_debug, run, Build, build_debug_as, run_lines,
    check.

  ("exempt" here is the harness's own case status, unrelated to file-size exemptions.)

Surface: `pub(super)` on 22 items of `cases.rs`:
- Status, Case, len_of, prelude_for, cases;
- Site, ENABLED_SITES, frame, at_site, sized, return_program;
- SCRATCH_ARMS, case_needs_scratch, program, replace_ident, result_program;
- run_peak, run, Build, build_debug_as, run_lines, check.

Also `pub(super)` on struct Case's fields signature, status, decl, setup, statements, check and
n, on impl Case's fn ty, and on impl Site's fn applies. The root adds `use cases::*;`.

This is the subset of the 11-part layout's measured 36-item surface that lands in `cases.rs`.
Some of these items are now used only inside `cases.rs`. Their extra `pub(super)` is harmless
(§2: no lint), and a missing one is a compile error. The field-site items stay in the root and
need no visibility edit, because children see private root items.

Sweeps:
1. The three `include_str!("inplace_self_update/*.tsv")` consts (lines 103-105) stay in the root,
   so their relative paths stay valid.
2. `scratch_arms_match_the_compiler` stays in the root, so its
   `include_str!("../../src/codegen/collection/assign/self_update.rs")` (line 696) is unchanged.
3. References (`grep -rn rt_inplace_self_update tests/guards .ai .github scripts Cargo.toml`):
   - the Cargo.toml `[[test]]` stanza (path unchanged);
   - `tests/guards/inplace_self_update_census.rs:6` doc (root path, valid);
   - `.ai/collections.md:92`;
   - `.github/workflows/coverage.yml:174-185` and `scripts/test-full.sh:114`
     (`cargo test [--release] --test rt_inplace_self_update -- --ignored`: target name unchanged,
     grids still in the root);
   - the module-doc filter `… rt_inplace_self_update via_let -- --ignored`, which still matches
     `every_via_let_field_cell_meets_its_expectation`.
4. `.ai/testing-gates.md:95` says "`rt_inplace_self_update`'s **two** grid tests are
   `#[ignore]`d". There are three (`grep -c '^#\[ignore' tests/runtime/rt_inplace_self_update.rs`
   → `3`). The third, `every_via_let_field_cell_meets_its_expectation`, came with `efa38f1d0`
   (plan-149-C2, `git log -S every_via_let_field_cell_meets_its_expectation`). The miscount is
   pre-existing, so Phase 3 corrects "two" → "three" (AGENTS.md: never leave a bug found).

Cross-letter note: letter Q turns `src/codegen/collection/assign/self_update.rs` into a
directory and repoints this `include_str!`. Under this layout the include is in the root
`tests/runtime/rt_inplace_self_update.rs` with the path `../../src/…`, not in a
`rt_inplace_self_update/scratch_arms.rs` part. A missing path is a compile error, and a moved
`SCRATCH_ARMS` declaration fails the test's `expect("the compiler's SCRATCH_ARMS declaration")`,
so neither can pass silently. The repoint is Q's scope. Letter J's comment edit ("`RETURN_NEVER`
in `self_update.rs`", line 379) lands in `rt_inplace_self_update/cases.rs` (inside `impl Site`)
once B has run.

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

Every phase follows the same order:
1. Run K1.
2. Split each file per its §4 entry: create the files, move the items verbatim, and add the
   `#[path]` pairs, the root glob and the `pub(super)` edits.
3. Run K2–K5.
4. Commit the phase: the new files, the edited roots, the ceiling file, and any `.ai` line the
   phase corrects.

Phases are ordered lowest risk first: pure two-way motion, then the designed big_int split, then
the support-file split.

### Phase 1 — rt_debug_soak and rt_native_term_runtime (§4.2, §4.3)

Two root + one part splits, with no support module and no path hazard.
`T="rt_debug_soak rt_native_term_runtime"`, `RE="rt_(debug_soak|native_term_runtime)"`.

- [x] K1 on the unsplit tree → 72 tests.
      Result (2026-10-18): K1 → `72` (49 rt_debug_soak + 23 rt_native_term_runtime).
- [x] `tests/runtime/rt_debug_soak.rs` → root (keeps `FLAT_BOUND`, `BLOCK_BOUND` and the library
      topics) + `resource_loops.rs` (§4.2).
      Moved lines 765-805, 830-896, 936-990, 1023-1520 verbatim; a line-coverage check of the
      move script reported no non-blank source line missing or duplicated.
- [x] `tests/runtime/rt_native_term_runtime.rs` → root (keeps the mouse topics and helpers) +
      `console.rs` (§4.3).
      Moved lines 513-1028 verbatim; same coverage check → none missing, none duplicated.
- [x] Prose-reference check: `grep -n 'const BLOCK_BOUND' tests/runtime/rt_debug_soak.rs` → one hit
      (the const is still declared in the root).
      Result: `35:const BLOCK_BOUND: u64 = 4096;` (one hit).
- [x] Size check: `for f in tests/runtime/rt_debug_soak.rs tests/runtime/rt_debug_soak/resource_loops.rs tests/runtime/rt_native_term_runtime.rs tests/runtime/rt_native_term_runtime/console.rs; do python3 -I tools/loc-census/census.py --file $f; done`
      → `0 723`, `0 444`, `0 724`, `0 431`.
      Result: `0 723`, `0 444`, `0 724`, `0 431`. `rustfmt --edition 2021 --check` on both roots → exit 0.
- [x] Tighten: K5, then commit `tests/guards/file_size_ceilings.txt` with the phase.
      Result: guard → `TIGHTEN test 723 (ceiling 1164) tests/runtime/rt_debug_soak.rs` and
      `TIGHTEN test 724 (ceiling 1152) tests/runtime/rt_native_term_runtime.rs`, nothing else;
      `MFB_LOC_TIGHTEN=1` removed exactly those two `ceiling test` rows (foreign-row diff → no
      output); re-run grep → no output.

Acceptance: 72 tests are listed under the same names and all pass, and the two paths leave the
budget banner.
  Check: K2 → no output; K3 → 2 × `test result: ok`, 0 failed (ignored soak tests stay ignored;
  K1/K2 list and compare them); K4 → no output; K5 → 2 TIGHTEN rows, then none (est. 15 min:
  rt_debug_soak builds the browser package and runs 49 programs).
  Result (2026-10-18): K2 → no output; K3 → `test result: ok. 49 passed; 0 failed` and
  `test result: ok. 23 passed; 0 failed`; K4 → no output; K5 → 2 TIGHTEN rows, then none. MET.
Commit: fdefb2fe9

### Phase 2 — rt_big_int (§4.1)

The one designed split: the powers grid becomes two tests.
`T="rt_big_int"`, `RE="rt_(big_int)"`.

- [x] K1 → 17 tests.
      Result (2026-10-18): K1 → `17`.
- [x] `rt_big_int.rs` → root (12 tests, `run`, `SHOW_BYTES`) + `division_and_word_boundaries.rs`
      (§4.1).
      Root keeps lines 1-777 and 2296-2387; the part holds 778-1018 and 2389-2686. rustfmt
      sorted the three `#[path]` mod pairs alphabetically (`mod_pow_grid` before `powers`).
- [x] `powers.rs`: the oracle test minus lines 1076, 1077–2246, 2247 and 2289, with its doc
      comment updated, + `mod_pow_matches_remainder_of_pow_for_bases_up_to_zero` built from
      lines 1077–1661 (§4.1).
- [x] `mod_pow_grid.rs`: `mod_pow_matches_remainder_of_pow_for_positive_bases` built from lines
      1662–2246 (§4.1).
- [x] Byte checks (§4.1): the case-line `diff` → no output, with `1170` lines; the oracle-body
      `diff` → no output.
      Result: case-line diff → no output, `wc -l` → `1170`; oracle-body diff → no output. A
      non-blank line-multiset compare of `HEAD:tests/runtime/rt_big_int.rs` against the four new
      files shows only the designed edits: gone are the two old doc-comment lines, the `of 390`
      print and the `0 of 390` expectation; added are the mod pairs, part headers, the reworded doc
      lines and the two grid tests' scaffolding.
- [x] Size check: `for f in tests/runtime/rt_big_int.rs tests/runtime/rt_big_int/{division_and_word_boundaries,powers,mod_pow_grid}.rs; do python3 -I tools/loc-census/census.py --file $f; done`
      → `0 809`, `0 511`, `0 699`, `0 601` (each ±3 for the hand-written grid scaffolding, all
      ≤ 900).
      Result: `0 809`, `0 511`, `0 699`, `0 601`.
- [x] Tighten: K5 and commit the ceiling file.
      Result: guard → `TIGHTEN test 809 (ceiling 2584) tests/runtime/rt_big_int.rs` only;
      `MFB_LOC_TIGHTEN=1` removed that one row (foreign-row diff → no output); re-run → no output.

Acceptance: the 17 old tests keep their names and pass, the two new grid tests pass, and every
grid case line is byte-identical to `HEAD`.
  Check: K2′ → exactly the two `>` lines of §4.1; K3 → ok, `19 passed`; K4 → no output; K5 →
  1 TIGHTEN row, then none; both byte-check diffs → no output (est. 10 min: three release builds
  replace one for the powers programs).
  Result (2026-10-18): K2′ → exactly `> mod_pow_matches_remainder_of_pow_for_bases_up_to_zero: test`
  and `> mod_pow_matches_remainder_of_pow_for_positive_bases: test`; K3 → `test result: ok. 19
  passed; 0 failed`; K4 → no output; K5 → 1 TIGHTEN row, then none; both byte checks → no output.
  MET.
Commit: 5518d73fa

### Phase 3 — rt_inplace_self_update (§4.4)

The support-file split (22 `pub(super)` items) and the `.ai` miscount fix.
`T="rt_inplace_self_update"`, `RE="rt_(inplace_self_update)"`.

- [x] K1 → 4 tests (three of them `#[ignore]`d grids; `--list` includes them).
      Result (2026-10-18): K1 → `4`.
- [x] `rt_inplace_self_update.rs` → root (keeps the six consts, the three
      `include_str!("inplace_self_update/*.tsv")` consts, `scratch_arms_match_the_compiler` with
      its unchanged `include_str!("../../src/…/self_update.rs")`, the field-site machinery and
      the three grids) + support `cases.rs` with the §4.4 `pub(super)` surface and root
      `use cases::*;`.
      Moved lines 107-688 and 719-1221; the move script asserted all 22 items, 7 `Case` fields,
      `Case::ty` and `Site::applies` got `pub(super)` (no more, no fewer). A non-blank
      line-multiset compare against `HEAD` shows only those `pub(super)` edits, the mod/glob lines,
      the part header, the doc fix below, and rustfmt's rewrap of the two signatures that
      `pub(super)` pushed past 100 columns (`build_debug_as`, `run_lines`).
- [x] `.ai/testing-gates.md:95`: "two grid tests" → "three grid tests" (§4.4 sweep 4). Check:
      `grep -c "rt_inplace_self_update\`'s three grid tests" .ai/testing-gates.md` → `1`.
      Result: `1`.
- [x] (discovered) The root's own module doc had the same miscount: "The two grid tests build
      thousands of programs" (`grep -n 'The two grid tests' tests/runtime/rt_inplace_self_update.rs`
      → `82:`). Corrected to "three" (a `//!` line, not moved code or an assertion). Check: the
      same grep → no output.
- [~] Run the ignored grid tests once, because CI runs them (`coverage.yml:180`):
      `cargo test --release --test rt_inplace_self_update -- --ignored` → ok.
      Result (2026-10-18, split tree, 5296 s): `every_self_update_case_meets_its_allocation_bound
      ... ok`, `every_via_let_field_cell_meets_its_expectation ... ok`,
      `every_field_kind_meets_its_expectation ... FAILED` — `19 of 1320 case/site pair(s) failed`.
      The 19 are **pre-existing, not caused by the split**: the same test with
      `MFB_SELF_UPDATE_FILTER='canvas::Text|http::Route'` in a detached worktree at `5518d73fa`
      (the commit before Phase 3, file unsplit) → `19 of 45 case/site pair(s) failed`, and the
      sorted `<kind> at <site>` failure labels of the two logs are identical (19 each). They are
      `http::Route` at S4/S5/S9/S10 (open `bugs/bug-1014-field-kinds-tsv-disagrees-with-its-generator.md`,
      which reports exactly these four) and 15 `canvas::Text` rows that `field_kinds.tsv` marks
      `na:TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE` but which now compile (or fail with a different
      code at S7/T7); no bug doc names them. Remaining: the field-kind grid passes only once those
      rows are fixed, which is outside this letter (test-data/compiler work, not motion).
- [x] Size check: `python3 -I tools/loc-census/census.py --file tests/runtime/rt_inplace_self_update.rs`
      → `0 763`; `… --file tests/runtime/rt_inplace_self_update/cases.rs` → `0 813`.
      Result: `0 763`; `0 821` (+8 from rustfmt rewrapping two signatures; see Corrections). ≤ 900.
- [x] Tighten: K5 and commit the ceiling file and `.ai/testing-gates.md`.
      Result: guard → `TIGHTEN test 763 (ceiling 1572) tests/runtime/rt_inplace_self_update.rs`
      only; `MFB_LOC_TIGHTEN=1` removed that one row (foreign-row diff → no output); re-run → no
      output.

Acceptance: 4 tests are unchanged, the plain and ignored runs pass, and the compiler-source
include still resolves from the root.
  Check: K2 → no output; K3 → ok (`scratch_arms_match_the_compiler` passes); the `--ignored` run
  → ok; K4 → no output; K5 → 1 TIGHTEN row, then none (est. 25 min: the ignored grids build every
  case program, and this run is the only check of the moved support items' run-time behavior).
  Result (2026-10-18): K2 → no output; K3 → `test result: ok. 1 passed; 0 failed; 3 ignored`
  (`scratch_arms_match_the_compiler` passes, so the root include resolves); K4 → no output; K5 →
  1 TIGHTEN row, then none. The `--ignored` run is NOT ok: 2 of 3 grids pass, and the third fails
  on 19 pairs that fail identically on the unsplit tree (see the task above). So the split itself
  changed no run-time result. The criterion "the ignored runs pass" is NOT MET at this tree, for a
  reason outside B.
Commit: a07439d9e

## Validation Plan

- Tests: `rt_big_int` gains two tests that carry the 390 grid cases its oracle test used to hold.
  No other test is added or removed. Every moved test runs in its phase's K3. The ignored ones are
  compared by K1/K2, and the self-update grids run in Phase 3.
- Coverage check: `tests/` is outside the coverage denominator by design
  (`scripts/coverage-common.sh` IGNORE `(^|/)tests/`), and no compiler code changes, so coverage
  cannot move.
- Runtime proof: K3 per phase is the end-to-end proof. Each target compiles and runs its MFBASIC
  programs. For `rt_big_int`, the byte checks prove that the split programs hold every original
  case line.
- Doc sync: `.ai/testing-gates.md:95` (the ignored-grid count, Phase 3). No spec or man topic
  changes meaning, because roots, target names and the cited `BLOCK_BOUND` stay put (§4.0). No
  `mfb spec` contract changes.
- Final gate (once, after Phase 3):
  `cargo test --test file_size_budget 2>&1 | grep -E 'tests/runtime/rt_(big_int|debug_soak|native_term_runtime|inplace_self_update)(\.rs|/)'`
  → no output. The same regex over `tests/guards/file_size_ceilings.txt` → no output.
  `cargo test --test test_targets_registered` → ok (est. 3 min). The full workspace suite runs
  once, in plan-176-V's Validation Plan.
  Result (2026-10-18, after `2672b3a1b`): guard grep → no output (`test result: ok. 35 passed`);
  ceiling-file grep → no output; `test_targets_registered` → `test result: ok. 4 passed` inside
  `bash scripts/test-fast.sh --test rt_inplace_self_update --test rt_big_int --test rt_debug_soak --test rt_native_term_runtime --test test_targets_registered`
  (check/build/skips/tests ok; B's targets 19 + 49 + 23 + 1 passed). Its `fmt` step failed on
  `tests/guards/file_size_budget/lex_cases.rs` (letter A's file, one `assert!` over 100 columns);
  rustfmt fixed it in `2672b3a1b`, and `cargo fmt --all -- --check` → exit 0 afterwards.

## Decisions

- Rule revised — owner, 2026-10-18. It replaces the 2026-10-14 "code ≤ 1000, test ≤ 250" rule.
  Counting is non-comment, non-blank LOC over `.rs`/`.mfb` under `src/`, `packages/`,
  `examples/`, `repository/` and `tests/`:

  | Kind | Applies to | Limit | Over the limit → |
  |---|---|---|---|
  | `code` | code LOC of a file that is not a test-only file | 1000 | split the file into modules |
  | `inline` | inline test LOC (`#[cfg(test)]` items / MFBASIC `TESTING … END TESTING`) inside a file that is not a test-only file | 250 | move the tests out into their own test-only file(s) |
  | `test` | total LOC of a test-only file (path component `tests`, `test_*.mfb`, `#![cfg(test)]`, or a `#[cfg(test)] mod x;` target and its subtree) | 1000 | split the test-only file into modules |

  There are no exemptions. Generated files (`GENERATED` banner) are the only exclusion. Parts
  target ≤ 900. The guard warns with a banner and never fails. A new module is always an
  acceptable split. Ceiling rows are `ceiling <code|inline|test> <n> <path>`.
- A single indivisible unit over its limit after natural extractions gets a real split that keeps
  every assertion. A test-count change is stated with before/after numbers — owner, 2026-10-18.
  B's instance: the rt_big_int powers grid splits into two 195-case tests, and rt_big_int goes
  from 17 to 19 tests (§4.1) — author, 2026-10-18.
- The powers grid splits by base sign (bases ≤ 0 / bases > 0), cut at a case boundary (line
  1661/1662). The two programs come out equal-sized (600 LOC each), and the order of every case
  is preserved — author, 2026-10-18.
- Parts are regrouped into as few files as keep each ≤ 900, and the root keeps the topics nearest
  its shared helpers — author, 2026-10-18 (brief: merge neighbours, do not invent seams).
- `scratch_arms_match_the_compiler` and the TSV `include_str!` consts stay in the
  rt_inplace_self_update root, so no include path changes — author, 2026-10-18.
- Integration-test splits keep the root as the `[[test]]` target and use `#[path]` children —
  plan-176-A (verified there) and `.ai/build-tooling.md`.
- Glob-cycle imports (`use super::*;` in parts, `use part::*;` in the root) compile without warnings
  (`/tmp/loc/probe2` printed `6`, no warning) — verified, 2026-10-14.
- Pre-existing defects found while mapping are fixed in the letter that touches them: the
  `.ai/testing-gates.md:95` ignored-grid miscount — AGENTS.md "never leave a bug", 2026-10-18.

## Corrections

- 2026-10-18 — Rule revised by the owner: test-only files limit 1000 (was 250), inline tests 250,
  no exemptions. Dropped 34 files, all now ≤ 1000:
  rt_callback_failure_frees, rt_closure_single_owner, rt_debug_arena, rt_debug_report,
  rt_domain_error_frees_locals, rt_error_model_recovery, rt_foreign_entry_callee_saved,
  rt_foreign_type_reexport, rt_global_argument_reassigned_by_callee, rt_hof_transient_bytes,
  rt_imported_package_global, rt_imported_type_name_collision, rt_imported_union_enum_members,
  rt_inline_trap_expression_boundary, rt_inline_trap_nested_call,
  rt_inline_trap_raising_operator, rt_inline_trap_short_circuit, rt_inline_trap_success_alloc,
  rt_inline_trap_union_bind, rt_inplace_failure_atomic, rt_io_key_readers, rt_json_bounds,
  rt_list_append_growth_bounds, rt_list_element_record_borrow, rt_list_take_drop_in_place,
  rt_map_integer_key_probe_cost, rt_math_fixed_trig_accuracy, rt_math_float_trig_large_angles,
  rt_moved_handle_guards, rt_native_io_runtime, rt_nothing_collections,
  rt_overloaded_function_values, rt_owned_argument, rt_package_dependency_closure (all
  `tests/runtime/*.rs`).

  The four kept files go from 37 new files to 6. The `rt_closure_single_owner/cases.rs`
  exemption is gone with its file. The `rt_big_int/powers.rs` exemption is replaced by a real
  split into three tests (17 → 19 tests). The `word_boundaries.mfb` extraction is gone, because
  that test is 288 LOC inline. Six phases became three.

  Re-measured at `ce7a995ca`:
  - rt_native_term_runtime is 1152 LOC / 23 tests (was 1038 / 20, after `d7a0a1f08` and
    `054062ce3`);
  - rt_inplace_self_update has three ignored grids, not two (the old K1 note);
  - `.ai/testing-gates.md` line refs are :95 and :883 (were :77 and :865).
- 2026-10-18 (execution) — `tests/runtime/rt_inplace_self_update/cases.rs` measures `0 821`, not
  the planned `0 813` (`python3 -I tools/loc-census/census.py --file tests/runtime/rt_inplace_self_update/cases.rs`).
  The +8 is rustfmt wrapping `build_debug_as` and `run_lines` over several lines once
  `pub(super)` pushed them past 100 columns (1 line → 5 each). Still ≤ 900.
- 2026-10-18 (execution) — the plan named only `.ai/testing-gates.md:95` for the "two grid tests"
  miscount. The root's own module doc (`//!` line 82 of `tests/runtime/rt_inplace_self_update.rs`,
  `grep -n 'The two grid tests'`) said the same; corrected in Phase 3 too.
- 2026-10-18 (execution) — the plan's `/tmp/loc/census.py` and `/tmp/loc/span.py` are
  `python3 -I tools/loc-census/census.py [--file PATH [--lines A B]]` after plan-176-A. All
  §4 line ranges matched the tree at `8157eb7c2` (anchors asserted by the move scripts), so no range
  was re-mapped.
- 2026-10-18 (execution) — owner instruction 2026-10-18: landing tier per phase; the full suite,
  including the ignored grids, runs once before the plan merges. Phases 1–3 had already run their
  named K-checks (K3 included) and Phase 3's `--release -- --ignored` grid run, which was allowed to
  finish. The final gate's `cargo test --test test_targets_registered` runs inside
  `bash scripts/test-fast.sh --test rt_inplace_self_update --test rt_big_int --test rt_debug_soak --test rt_native_term_runtime --test test_targets_registered`.
- 2026-10-18 (execution) — the Phase 3 `--ignored` run fails `every_field_kind_meets_its_expectation`
  on 19 pairs (`http::Route` S4/S5/S9/S10, which is bug-1014, and 15 `canvas::Text` rows). The
  failures are pre-existing: the same failure labels show on the unsplit tree at `5518d73fa`
  (filtered run in a detached worktree). Phase 3 records this as `[~]` and does not edit
  `field_kinds.tsv` (never edit a test to pass, and these rows are not motion).

## Summary

The risk is a test that silently stops compiling, running or checking. The before/after name list
per target (K1/K2, K2′ for rt_big_int) and one run per phase (K3) cover compiling and running. For
checking, two `diff`s prove that the rt_big_int split kept all 1,170 grid case lines and the rest
of the oracle body byte for byte. The rt_inplace_self_update includes stay in the root, so no path
changes. Outside `tests/runtime/`, only the ceiling file and one corrected count in
`.ai/testing-gates.md` change.
