# plan-131-C: scripts/ cleanup — move generators, probes, oracles and benchmarks out

Last updated: 2026-09-12
Effort: large (3h–1d)
Depends on: plan-131-B

Fourteen top-level files and `bench-probes/` are not helpers you run against the compiler. They are
generators tied to vendored data, measuring probes, an oracle, and a benchmark. Each moves next to
the data or package it serves, with a README. The rule for what belongs where is in
`plan-131-A-scripts-fix-and-delete.md` §3.

References:

- `plan-131-A-scripts-fix-and-delete.md` — prerequisites gate, inventory, Open Decisions (the
  vector generator decision is resolved before Phase 3 here).
- `scripts/check-generated.sh` — the CI gate (`coverage.yml:47`) that runs every generator.
- `tools/codepage-index/README.md`, `tools/math-kernels/README.md` — the existing precedents.
- `.ai/specifications.md` — `src/docs/spec/stdlib/08_encoding.md` cites the codepage generator path.

## Prerequisites

See plan-131-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-131-B complete | `ls planning/plan-131-B-* 2>/dev/null` → no match | NOT MET |
| Vector-generator Open Decision answered by the user | recorded in plan-131-A Open Decisions | NOT MET |

## 1. Goal

- None of the 14 files, nor `bench-probes/`, is under `scripts/`.
- Every mover runs from its new location.
- `check-generated.sh` stays green, and every generated file's header names the generator's new path.
- The bug-470 lock census still covers `bench-lowering.sh` after it moves.

### Non-goals

- Generated file **content** does not change. Only the header line naming the generator path may
  change (verified by `git diff` in each phase).
- No `.ast/.ir/.ncode` golden changes. Any golden diff is a bug to root-cause (see Phase 2).
- The generators' logic is not refactored.

## 2. Current State (audit 2026-09-12)

**Path depth.** These files find the repo root as `dirname(dirname(__file__))`, so a move into
`tools/<dir>/` adds one level:
- `gen_codepage_tables.py`
- `fetch_codepage_index.py`
- `audit_codepage_index.py`
- `check_vector_bodies.py`
- `yaml_oracle_diff.py`

Two more depend on location in other ways:
- `gen_regex_scripts.py` uses `dirname(__file__)/../third_party`.
- `gen_unicode_script_table.py` imports `gen_regex_scripts` through `sys.path`, so the two must
  move together.

**Generated-file headers that name the generator path:**
- `src/codegen/string/unicode/unicode_gencat_ranges.txt:2`
- `unicode_script_ranges.txt:2`
- `unicode_script_names.mfb:2` (a `REM` line in MFBASIC source)
- `src/codegen/builtins/encoding/helper_codepage_table.rs` (several comment lines:
  `grep -n gen_codepage_tables src/codegen/builtins/encoding/helper_codepage_table.rs`)

**Live references per mover** (`git grep -l -F <name> -- ':!planning/completed' ':!bugs/completed'`):

| File | Referenced by |
|---|---|
| `gen_codepage_tables.py` | `check-generated.sh`, `scripts/README.md`, `helper_codepage_table.rs`, `encoding/mod.rs`, `src/docs/spec/stdlib/08_encoding.md` |
| `audit_codepage_index.py` | `gen_codepage_tables.py`, `helper_codepage_table.rs`, `scripts/README.md` |
| `fetch_codepage_index.py` | `scripts/README.md` (and `tools/codepage-index/README.md` after A) |
| `gen_unicode_gencat_table.py` | `check-generated.sh`, `unicode_gencat_ranges.txt`, `src/unicode/range_tables.rs`, `scripts/README.md` (and `tools/math-kernels/capture.sh`, `coverage.yml` after A) |
| `gen_unicode_script_table.py` | `check-generated.sh`, `gen_regex_scripts.py`, `unicode_script_ranges.txt`, `range_tables.rs`, `.gitignore`, `scripts/README.md` |
| `gen_regex_scripts.py` | `check-generated.sh`, `unicode_script_names.mfb`, `scripts/README.md` |
| `gen_vector_package.py` / `check_vector_bodies.py` | `.gitattributes`, `check-generated.sh`, each other, `scripts/README.md` |
| `icmp-capability-probe.c` | `src/codegen/builtins/net/gen_ping.rs` (comment) |
| `icmp-constants-probe.c` | `src/target/linux_common/code.rs`, `src/target/macos_aarch64/code.rs` (comments) |
| `rvv-qemu-runner.sh` / `rvv-ulp-two-profile.sh` | each other, `.ai/remote_systems.md`, `scripts/README.md`. They drive `tools/math-kernels/runtime_ulp.py` |
| `bench-lowering.sh` / `bench-probes/` | census row, `scripts/README.md`, `src/codegen/engine/operand/operand.rs` (comment) |
| `yaml_oracle_diff.py` | `packages/yaml/README.md`, `packages/yaml/oracle/README.md`, `examples/yaml-json/smoke.sh`, `scripts/README.md` |

**The census sees only `scripts/*.sh`.** `tests/gate/gate_lock_covers_every_writer.rs` calls
`read_dir(repo_root().join("scripts"))`. `bench-lowering.sh` takes the lock and emits dumps; moved
as-is, it would silently leave the census.

**UNVERIFIED:** whether the `REM` header line in `unicode_script_names.mfb` reaches any golden
(the file is embedded MFBASIC source). Phase 2 checks it.

## 3. Design

| Destination | Contents | README |
|---|---|---|
| `tools/codepage-index/` | the 3 codepage scripts, next to their index files | existing, updated |
| `tools/unicode-tables/` | the 3 unicode generators | new: what each writes, the Python 3.14 / Unicode 16.0.0 pin, `third_party/unicode` input |
| `tools/vector-gen/` | the generator + body checker (or deleted, per the Open Decision) | new |
| `tools/net-probes/` | the 2 ICMP probes | new: how to build each and which codegen constants they justify |
| `tools/math-kernels/` | the 2 rvv scripts, next to `run-remote-x86.sh` and `runtime_ulp.py` | existing, updated |
| `tools/bench-lowering/` | `bench-lowering.sh` + `probes/` | new |
| `packages/yaml/oracle/pyyaml_diff.py` | the second YAML oracle, next to the Node one | existing oracle README, updated |

The census test gains a second scan root: every `tools/*/*.sh`. `CLASSIFICATION` keys become
repo-relative paths (`scripts/artifact-gate.sh`, `tools/bench-lowering/bench-lowering.sh`), so a
future script under `tools/` cannot escape the lock census either.

Gate class: **provably neutral** for the compiler. The checks are `check-generated.sh` green,
artifact-gate green, and generated-file diffs limited to header path lines. A diff anywhere else
means root-cause it (for a golden, inspect one fixture), then fix it. It is never a reason to
abandon the move.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit; `- [~]` partial; moot
> tasks struck through with evidence; fill `Commit:`. **An unticked box means NOT DONE.**

### Phase 1 — Probes, rvv, yaml oracle (no generated output)

- [ ] `git mv scripts/icmp-{capability,constants}-probe.c tools/net-probes/`. Fix the build
      command in each file's header. Build each once with that command (the cc line from the
      header) → exit 0. Write `tools/net-probes/README.md`. Update the comments in `gen_ping.rs`,
      `linux_common/code.rs` and `macos_aarch64/code.rs`.
- [ ] `git mv scripts/rvv-qemu-runner.sh scripts/rvv-ulp-two-profile.sh tools/math-kernels/`. Fix
      the `RUNNER`/`ULP` paths and the header. Update `.ai/remote_systems.md`,
      `tools/math-kernels/README.md` and `scripts/README.md`.
- [ ] Prove the rvv move: run `tools/math-kernels/rvv-ulp-two-profile.sh` on box 2232 with the
      smallest `--limit` it accepts (read the script) → the same `primary` summary under both
      profiles. Record it.
- [ ] `git mv scripts/yaml_oracle_diff.py packages/yaml/oracle/pyyaml_diff.py` and fix `ROOT`
      depth. Before the move, run `python3 scripts/yaml_oracle_diff.py corpus > /tmp/y-old`; after,
      run the new path `> /tmp/y-new`; `diff` → empty. (Needs `pyyaml` and `examples/yaml-json`
      built. If `pyyaml` is absent, install it into a venv under `/tmp`; do not skip.) Update
      `packages/yaml/README.md`, `packages/yaml/oracle/README.md`, `examples/yaml-json/smoke.sh`
      and `scripts/README.md`.

Acceptance:
- Both probes build from their new paths.
- The rvv run matches across profiles.
- The yaml corpus output is identical.
- `bash scripts/artifact-gate.sh target/release/mfb all` → 0 diffs (only comments in `src/` changed).

Commit: —

### Phase 2 — Generators (codepage, unicode)

- [ ] `git mv` the 3 codepage scripts into `tools/codepage-index/`:
      - fix the root depth and each generator's header string;
      - run `python3 tools/codepage-index/gen_codepage_tables.py`, then
        `git diff src/codegen/builtins/encoding/helper_codepage_table.rs` → only lines naming the
        generator path change;
      - run `audit_codepage_index.py` from the new path → exit 0;
      - update `check-generated.sh`, `encoding/mod.rs` comments, `08_encoding.md` (spec sync),
        `tools/codepage-index/README.md` and `scripts/README.md`.
- [ ] `git mv` the 3 unicode generators into `tools/unicode-tables/` together:
      - fix the `third_party` path and the `sys.path` import;
      - regenerate all three outputs under Python 3.14 (per the header pin), then
        `git diff src/codegen/string/unicode/` → only header path lines change;
      - update `check-generated.sh`, `src/unicode/range_tables.rs` comments, `.gitignore` comment
        and `scripts/README.md`;
      - write `tools/unicode-tables/README.md`.
- [ ] Resolve the UNVERIFIED `REM` question: `bash scripts/artifact-gate.sh target/release/mfb all`
      and `bash scripts/test-accept.sh target/release/mfb /tmp/accept-131c '*regex*' '*unicode*' '*genCat*'`
      → 0 diffs. If a golden diffs, inspect one fixture to see whether the `REM` text or its span
      reaches the output, and record the finding in Corrections. Then decide: keep the old
      header wording, or fix the leak.

Acceptance:
- `sh scripts/check-generated.sh` exits 0.
- Generated-file diffs are header-only.
- artifact-gate plus the scoped acceptance run show 0 diffs.

Commit: —

### Phase 3 — Vector generator (per the user's decision)

- [ ] **If move:** `git mv` both files to `tools/vector-gen/`. Fix `check_vector_bodies.py`'s root
      depth and its subprocess path to the generator. Fix the stale "Source companion … GENERATED"
      header text in `gen_vector_package.py` that describes the deleted `src/builtins/*.mfb`.
      Update `check-generated.sh`, `.gitattributes` and `scripts/README.md`. Write a README.
- [ ] **If delete:** `git rm` both. Remove their block from `check-generated.sh` and `.gitattributes`.
      Add a line to `src/codegen/builtins/vector/mod.rs`'s module doc saying the `BODY*` consts are
      the only source.
- [ ] Mutation check (move case): change one character inside one `BODY*` const, run
      `sh scripts/check-generated.sh` → it must fail; revert.

Acceptance:
- `sh scripts/check-generated.sh` exits 0.
- In the move case, the mutation makes it fail.

Commit: —

### Phase 4 — bench-lowering and the census scan root

- [ ] `tests/gate/gate_lock_covers_every_writer.rs`:
      - scan `scripts/*.sh` and `tools/*/*.sh`;
      - key `CLASSIFICATION` by repo-relative path;
      - keep the `gate-lock.sh`/`artifact-kinds.sh` skip.
      Run it before the move → green.
- [ ] `git mv scripts/bench-lowering.sh tools/bench-lowering/bench-lowering.sh` and
      `git mv scripts/bench-probes tools/bench-lowering/probes`. Fix the `gate-lock.sh` source path
      to `$ROOT/scripts/gate-lock.sh`, plus `GATE_LOCK_TREE`, the `cd` and `PROBES_DIR`. Update the
      census row key, the `operand.rs` comment and `scripts/README.md`. Write the README.
- [ ] Census mutation: in the working tree only, delete the `gate_lock_acquire` line from the moved
      script → `cargo test --test gate_lock_covers_every_writer` must fail naming it. Revert.
- [ ] Run `tools/bench-lowering/bench-lowering.sh` once from `/tmp` → it completes and prints its
      table. Record the wall time.

Acceptance:
- The census tests pass.
- The mutation makes the census fail.
- The benchmark runs from its new home.
- `ls scripts` shows none of the 14 files and no `bench-probes`.

Commit: —

## Validation Plan

- Tests: `tests/gate` census and lock tests, `check-generated.sh`, and
  `codepage_tables_match_the_vendored_index_files`
  (`cargo test codepage_tables_match_the_vendored_index_files --no-fail-fast`).
- Neutrality: artifact-gate `all` → 0 diffs after each phase.
- Doc sync: spec `08_encoding.md` (path only), `.ai/remote_systems.md`, the package and tools READMEs.
- Fresh-worktree check (memory `pin-over-a-gitignored-file-is-not-a-pin`): run
  `sh scripts/check-generated.sh` once from `git worktree add --detach /tmp/wt-131c`, so no
  untracked local file props the moved generators up.

## Open Decisions

See plan-131-A (the vector generator decision gates Phase 3).

## Corrections

## Summary

The risk is in the generator header lines, which must be regenerated in the same commit or CI's
`check-generated.sh` goes red, and in the census scan root, without which `bench-lowering.sh`
would silently leave the bug-470 lock census. The generators' logic and output data do not change.
