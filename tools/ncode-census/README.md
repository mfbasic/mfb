# ncode-census

The measuring instrument of plan-171 (`planning/plan-171-A-aarch64-load-store-pairs.md`):
per-pattern instruction counts over the `mfb build -ncode` dumps of a fixed corpus, so
every plan-171 letter quotes its before/after numbers from one tool. Not a gate; nothing
in `cargo test` runs it. Python 3 and bash, stdlib only.

## Usage

```
tools/ncode-census/ncode-census.sh <mfb> <target> <out.tsv> [extra mfb build flags...]
python3 -I tools/ncode-census/census.py --diff before.tsv after.tsv [--all-scopes]
python3 -I tools/ncode-census/census.py --pairs DUMP...          # the plan-171-A §2.2 table
python3 -I tools/ncode-census/census.py --top METRIC [N] DUMP... # the N functions highest in METRIC
python3 -I tools/ncode-census/census.py [--label NAME] DUMP...   # TSV for given dumps
```

- `ncode-census.sh` copies every corpus project into a scratch directory (never builds
  in-tree), builds the local packages the corpus imports, builds each project with
  `mfb build -q -ncode -target <target>` (`JOBS` in parallel, default 6), and writes the
  TSV. Extra arguments go to every build (`-O0`, `--app`, …). `KEEP=<dir>` keeps the
  scratch copies and their dumps (`<dir>/projects/<path with / as __>/<name>.ncode`) for
  `--top` afterwards. A project that fails to build gets a `build_failed 1` row and is
  named on stderr; it is never silently dropped.
- `--diff` prints `project metric before after delta` for every metric that moved, at the
  total scope (`*`), or per project with `--all-scopes`. The pair metrics and the
  `stp`/`ldp` op counts always print.
- `--pairs` reproduces the prototype that produced plan-171-A §2.2 exactly (whole-dump
  instruction count including labels; store pairs; load pairs whose first destination is
  neither the base nor the second destination; no offset-range filter).

## Corpus

Every `project.json` under `examples/`, `benchmark/language/mfb` and `tests/byte-identity/`
except `kind: package` projects (they are dependencies) and `examples/audio`, which imports
`jzaun#libsnd` from the registry, which a hermetic sweep cannot fetch. 56 projects at
`89a4dce2c`. Local `file:packages/*.mfp` dependencies are built first in the scratch copy,
as `scripts/build-examples.sh` and `benchmark/language/run.sh` do: `bench_workers`
(benchmark), `dom` → `fetch`/`display` → `browser/app`, `packages/yaml` → `yaml-json`.

## TSV and metrics

`project function metric value`, one row per (project, metric) with `function` = `*`, then
the totals with `project` = `*`. Every count is over adjacent instructions within one
function's stream (a label is an instruction, so nothing adjacent spans a label).

| Metric | Counts |
|---|---|
| `functions`, `instructions` | functions; instruction entries, labels included |
| `op:<mnemonic>` | occurrences of each op (`op:stp`, `op:ldp`, … are the plan-171 forms) |
| `str_pair_candidates`, `ldr_pair_candidates` | adjacent `str_u64`/`ldr_u64` pairs, same `base`, numeric offsets 8 apart (overlapping: three stores at 0/8/16 are two candidates) |
| `str_pair_legal`, `ldr_pair_legal` | candidates `optimizer::opt2::pairing` may merge: lower offset in −512…504 and a multiple of 8; for loads, the first destination is not the base and the two destinations differ |
| `pair_reject:<reason>` | candidates failing a rule: `range`, `align`, `dst_is_base`, `same_dst` |
| `cmp0_eq_branch` | `cmp_imm r, 0` then `b.eq`/`b.ne` (`cbz`/`cbnz` shape, plan-171-B) |
| `cmp0_sign_branch` | `cmp_imm r, 0` then `b.lt`/`b.ge` (sign-bit `tbz`/`tbnz` shape) |
| `movimm_feeds_logical` | `mov_imm R` then `and`/`orr`/`eor` reading `R` (logical-immediate shape, plan-171-C) |
| `single_bit_test` | `mov_imm R, 2^k; and D, X, R; cmp_imm D, 0; b.eq/b.ne` (`tbz`/`tbnz` shape) |
| `bool_diamond` | `b.cc L1; mov_imm R, 0|1; b L2; L1: mov_imm R, 1|0; L2:` (`cset` shape, plan-171-D) |
| `match_chains`, `match_chain_compares` | runs of ≥ 3 consecutive `cmp_imm r, k; b.eq` on one register, and the compares in them (jump-table shape, plan-171-F) |

The `.ncode` dump carries no instruction `source`: `CodeInstruction.source` is the
compiler's own emitting call site (`#[track_caller]` audit metadata, never serialized), so
no metric can split candidates by it.

## Baselines

`baseline-<target>.tsv`, produced at `89a4dce2c` (main, after call-graph DCE) by the
release `mfb` of that commit:

```
tools/ncode-census/ncode-census.sh target/release/mfb <target> tools/ncode-census/baseline-<target>.tsv
```

for `macos-aarch64`, `linux-aarch64`, `linux-x86_64` and `linux-riscv64`. All 56 projects
build for the first three. `linux-riscv64` carries 43: twelve are app-mode projects (the
compiler refuses app mode on riscv64), and `benchmark/language/mfb` fails with the
compiler panic `rv64 v128: function needs 172 concurrent vector slots, exceeding the
126-slot region` (`src/arch/riscv64/select.rs:select_riscv64`, pre-existing at
`89a4dce2c`). A later letter compares its own run against these with `--diff` (or
re-baselines at its own starting commit and says which).
