# Plan: Built-in Unit Tests + Coverage Verification (grcov & tarpaulin)

This is the plan for raising the `mfb` compiler/toolchain to **95–100%
functional line coverage per source file** using Rust's built-in
(`#[cfg(test)] mod tests`) unit tests, and for wiring up **grcov** and
**cargo-tarpaulin** so that coverage is measured, reported, and gated.

## Goal

For every `src/**.rs` (and `repository/src/**.rs`) file:

- Add or extend an in-file `#[cfg(test)] mod tests` block that exercises the
  file's public and crate-internal behavior.
- Reach **≥95% line coverage** per file (target 95–100%), measured by grcov
  locally and tarpaulin in CI, **excluding documented exclusion regions**
  (see [Exclusions](#what-counts-coverage-targets--exclusions)).
- Keep tests *functional* — assert on observable behavior (return values,
  emitted bytes, JSON, error codes), not on private implementation shape.

Today **17 of 78** `src` files carry unit tests and **6 of 9** non-generated
`repository/src` files do; total ~93.7k LOC in `src`. The bulk of current
verification lives in the `tests/` integration harness
(`repo_acceptance.rs`, `native_io_runtime.rs`, ~700 fixture dirs that compile
and run `.mfb` programs end-to-end). That harness stays — this plan *adds* a
fast, isolated unit layer underneath it and makes coverage measurable.

## Why both grcov and tarpaulin

The dev platform is **macOS aarch64** (Darwin 24.6); CI is expected to run on
**Linux**. The two tools have complementary reach, so we use each where it
works best rather than picking one.

| Tool | Engine | Runs on | Role in this plan |
|------|--------|---------|-------------------|
| **grcov** | LLVM source-based (`-C instrument-coverage`) | macOS aarch64 **and** Linux | Primary **local** loop — accurate per-file/per-line coverage on the dev's machine. Produces HTML + lcov + the gating numbers. |
| **cargo-tarpaulin** | `--engine llvm` (ptrace fallback is Linux/x86-64 only) | Linux (CI) | **CI gate** — per-file `--fail-under`, Cobertura/lcov for the CI dashboard. Authoritative pass/fail on PRs. |

Rationale: tarpaulin's classic ptrace engine does not support
macOS/aarch64, so it cannot be the local tool here; grcov's LLVM
instrumentation does. Both consume the **same** instrumented test run, so
their numbers should agree within rounding. (If we later prefer a single
wrapper, `cargo-llvm-cov` subsumes the grcov setup — out of scope; the user
asked for grcov + tarpaulin specifically.)

---

## Tooling setup

### 1. Toolchain & components

Source-based coverage needs the `llvm-tools-preview` component. Pin the
toolchain so coverage is reproducible:

- Add `rust-toolchain.toml`:

  ```toml
  [toolchain]
  channel = "1.96.0"
  components = ["llvm-tools-preview", "rustfmt", "clippy"]
  ```

- One-time installs (documented in the contributor README, not committed
  binaries):

  ```sh
  cargo install grcov
  cargo install cargo-tarpaulin
  rustup component add llvm-tools-preview
  ```

### 2. grcov — local coverage script

Add `scripts/coverage.sh` (mirrors the existing `scripts/` convention used
for compiler-function listing and man-page updates):

```sh
#!/usr/bin/env sh
# Local coverage via grcov (LLVM source-based). Works on macOS aarch64.
set -eu
export CARGO_INCREMENTAL=0
export RUSTFLAGS="-C instrument-coverage"
export LLVM_PROFILE_FILE="$(pwd)/target/coverage/mfb-%p-%m.profraw"

rm -rf target/coverage
mkdir -p target/coverage

# Run the whole test suite (unit + integration) instrumented.
cargo test --workspace --all-targets

grcov target/coverage \
  --binary-path ./target/debug/ \
  --source-dir . \
  --output-types html,lcov \
  --branch \
  --ignore-not-existing \
  --ignore 'target/*' \
  --ignore 'repository/target/*' \
  --ignore 'tests/*' \
  --excl-line 'GRCOV_EXCL_LINE|unreachable!|todo!|unimplemented!' \
  --excl-start 'GRCOV_EXCL_START' \
  --excl-stop  'GRCOV_EXCL_STOP' \
  -o target/coverage/

echo "HTML: target/coverage/html/index.html"
echo "lcov: target/coverage/lcov"
```

A companion `scripts/coverage-check.sh` parses `lcov` and prints any file
below 95%, exiting non-zero — the local equivalent of the CI gate (so the
gate can be reproduced before pushing).

### 3. tarpaulin — CI config

Add `tarpaulin.toml` at the repo root:

```toml
[coverage]
engine = "llvm"
workspace = true
all-targets = true
out = ["Lcov", "Html", "Xml"]      # Xml = Cobertura for the CI dashboard
output-dir = "target/tarpaulin"
fail-under = 95                     # global floor
exclude-files = [
  "target/*",
  "repository/target/*",
  "tests/*",
  "src/unicode_runtime_tables.rs", # generated table (see exclusions)
  "src/unicode_backend.rs",        # mostly generated lookups
  "*/mod.rs",                      # only the 1-3 line re-export shims
]
```

> Note: `exclude-files` with `*/mod.rs` is too broad — several `mod.rs` files
> hold real logic (`audit/mod.rs`, `man/mod.rs`, `target/package_mfp/mod.rs`,
> the 15.6k-line `code/mod.rs`). Replace the glob with an explicit list of the
> *trivial* shim mods during Phase 0 once the inventory is finalized.

### 4. CI workflow

There is currently **no `.github/workflows/`**. Add `coverage.yml` (Linux
runner):

```yaml
name: coverage
on: [push, pull_request]
jobs:
  coverage:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.96.0
        with: { components: llvm-tools-preview }
      - run: cargo install cargo-tarpaulin
      - run: cargo tarpaulin --config tarpaulin.toml
      - uses: actions/upload-artifact@v4
        with: { name: coverage, path: target/tarpaulin }
```

The `--fail-under 95` makes the job fail when overall coverage regresses.
Per-file enforcement is done by `scripts/coverage-check.sh` run as an extra
step against the produced `lcov` (tarpaulin's own per-file fail flag is
coarse).

---

## What counts: coverage targets & exclusions

"95–100% per file" is the target, but a compiler has lines that are
genuinely not unit-testable or not meaningful to cover. Those are **excluded
explicitly and visibly**, never silently:

- **Generated data tables** — `src/unicode_runtime_tables.rs`,
  the lookup arrays in `src/unicode_backend.rs`,
  `src/target/shared/code/private/unicode.rs`. Test the *accessors*, exclude
  the data literals.
- **Platform-gated branches** — macOS-only code can't execute on the Linux CI
  runner and vice-versa. Wrap the unreachable arm with
  `// GRCOV_EXCL_START` / `// GRCOV_EXCL_STOP`, and run grcov on macOS to
  cover the other side. Coverage is the **union** of both platforms' runs.
- **Defensive `unreachable!()` / `todo!()` / `panic!("internal")`** — excluded
  by the `--excl-line` regex; their existence is the assertion.
- **Trivial re-export `mod.rs` shims** (`arch/mod.rs`, `os/mod.rs`,
  `target/shared/mod.rs`, `target/shared/lower.rs`, etc., 1–6 lines each) —
  no logic, excluded from the denominator.

Everything else is in scope for ≥95%. A file that cannot reach 95% without
running real syscalls/hardware (see Tier C) documents *why* in a comment at
the top of its `mod tests`, and is covered by the integration harness instead
— it is listed as an explicit exception in the tracking table, not hand-waved.

---

## Testing strategy by tier

The 78 files split into tiers by how unit-testable they are. The strategy and
the realistic per-file target differ by tier.

### Tier A — Pure logic / data transforms (unit-test to 95–100%)

Deterministic in → out, no IO. Highest ROI; these should hit 98–100%.

`numeric.rs`, `escape.rs`, `rules.rs`, `lexer.rs`, `ast.rs` (parser),
`ir.rs` (lowering + JSON + binary), `binary_repr.rs`, `typecheck.rs`,
`resolver.rs`, `monomorph.rs`, `target.rs`, all `builtins/*.rs` (signature
tables), `audit/*.rs`, `man/mod.rs`, `arch/aarch64/{abi,ops,encode}.rs`,
`unicode_backend.rs`, `unicode_runtime_tables.rs` (accessors).

Approach: table-driven `#[test]` cases. For parsers/typecheckers, feed source
strings and assert on the AST/IR/diagnostic. For `binary_repr`/`ir` binary
codecs, assert **round-trip** (`decode(encode(x)) == x`) plus malformed-input
error paths. Reuse the existing builder helpers (e.g. the `func(...)` /
`res(...)` constructors already in `escape.rs`'s test module) — factor shared
fixtures into a `src/testutil.rs` (`#[cfg(test)] pub`).

### Tier B — Codegen / plan builders (assert on emitted artifacts)

The `target/shared/code/builder_*.rs`, `net.rs`, `tls.rs`, `nir.rs`,
`plan.rs`, `runtime.rs`, `code/mod.rs`, `link_thunk.rs`, and the per-platform
`target/{macos,linux}_aarch64/*` files. These emit AArch64 instructions,
relocations, and link plans.

They *are* unit-testable without running the binary: drive a `CodeBuilder` /
plan over a small hand-built IR and assert on the produced
`Vec<Instruction>` / `NativeCodePlan` / relocation list / JSON. This is what
the existing `code/mod.rs` arena tests and `validate.rs` tests already do —
extend that pattern per builder.

Realistic target: **95% line coverage of the lowering logic**; the exhaustive
*semantic* correctness of generated machine code stays the job of the `tests/`
run-the-program harness. Where a builder branch only differs in an emitted
immediate, one representative case per branch is enough for line coverage —
note it so reviewers don't expect exhaustive value testing.

### Tier C — OS object/link emitters & syscall-bound IO (hybrid)

`os/macos/{object,link}.rs`, `os/linux/{object,link,flavor}.rs`,
`os/{macos,linux}/mod.rs`, `target/macos_aarch64/app.rs` (NSApplication
plumbing), the FD/socket runtime helpers.

Object/link **writers** are pure (struct → `Vec<u8>` Mach-O/ELF) and unit-test
to 95% by asserting on header fields, segment layout, and symbol tables —
`os/macos/link.rs` already has 12 such tests; replicate for `object.rs` and
the Linux side. The genuinely syscall/UI-bound parts (`app.rs` objc_msgSend
event loop, live socket IO) are **excluded with justification** and remain
covered by `macos-app-mode-*` and `net-*`/`thread-*` integration fixtures.

### Tier D — CLI / process orchestration (`main.rs`)

`main.rs` (2918 LOC) mixes pure manifest parsing/validation (Tier-A-like,
already has 12 tests) with `build_project` subprocess orchestration. Unit-test
the parsers/validators to 95%; the end-to-end command dispatch stays under the
`repo_acceptance.rs` harness. Splitting `main.rs` per
`plan-11-large-files.md` (into `cli/` + `manifest/`) makes the pure parts
trivially coverable — **coordinate with plan-11**; do the split first where it
unblocks coverage.

---

## Per-file inventory & targets

Legend: **T** = tier, **#** = current `#[test]` count, **Target** = line-cov
goal. Files not listed are trivial shims (excluded). Counts are a planning
snapshot and will drift.

### Front-end (Tier A — highest priority, do first)

| File | LOC | # | Target |
|------|----:|--:|-------:|
| `src/numeric.rs` | 25 | 0 | 100% |
| `src/escape.rs` | 562 | 5 | 100% |
| `src/lexer.rs` | 726 | 5 | 98% |
| `src/rules.rs` | 995 | 0 | 98% |
| `src/ast.rs` | 5268 | 7 | 95% |
| `src/ir.rs` | 5878 | 7 | 95% |
| `src/binary_repr.rs` | 3928 | 4 | 97% |
| `src/typecheck.rs` | 6961 | 0 | 95% |
| `src/resolver.rs` | 1522 | 0 | 95% |
| `src/monomorph.rs` | 1674 | 0 | 95% |
| `src/target.rs` | 239 | 0 | 98% |

### Builtins & audit (Tier A)

| File | LOC | # | Target |
|------|----:|--:|-------:|
| `src/builtins/{general,thread,net,fs,math,json,io,strings,tls,resource,mod}.rs` | ~2.7k | 4 | 98% |
| `src/audit/{collect,json,report,text,mod}.rs` | ~1.9k | 0 | 95% |
| `src/man/mod.rs` | 337 | 0 | 95% |
| `src/unicode_backend.rs` | 260 | 12 | 95%* |
| `src/unicode_runtime_tables.rs` | 523 | 4 | accessors only* |

### Arch / codegen (Tier B)

| File | LOC | # | Target |
|------|----:|--:|-------:|
| `src/arch/aarch64/{encode,abi,ops}.rs` | ~1.8k | 3 | 95% |
| `src/target/shared/code/mod.rs` | 15631 | 4 | 95%† |
| `src/target/shared/code/builder_*.rs` (14 files) | ~17k | 0 | 95%† |
| `src/target/shared/code/{net,tls,link_thunk}.rs` | ~5.1k | 0 | 95%† |
| `src/target/shared/code/private/unicode.rs` | 921 | 0 | accessors only |
| `src/target/shared/{plan,runtime,nir,validate}.rs` | ~7.5k | 3 | 95% |
| `src/target/shared/lower.rs` | 19 | 0 | shim |
| `src/target/{macos,linux}_aarch64/*.rs` | ~3.3k | 0 | 95% |
| `src/target/package_mfp/mod.rs` | 350 | 4 | 98% |

### OS backends (Tier C)

| File | LOC | # | Target |
|------|----:|--:|-------:|
| `src/os/macos/link.rs` | 1745 | 12 | 97% |
| `src/os/macos/object.rs` | 988 | 1 | 95% |
| `src/os/linux/{link,object,flavor,mod}.rs` | ~1.9k | 5 | 95% |
| `src/target/macos_aarch64/app.rs` | 1687 | 0 | 70%‡ |

### CLI (Tier D)

| File | LOC | # | Target |
|------|----:|--:|-------:|
| `src/main.rs` | 2918 | 12 | 90%‡ |

### repository crate (Tier A/C)

| File | LOC | # | Target |
|------|----:|--:|-------:|
| `repository/src/{store,server,client,local,crypto,validation,package}.rs` | ~1.9k | 14 | 95% |
| `repository/src/{lib,main}.rs` | 86 | 0 | shim/90% |
| `repository/target/**` (generated bindgen, serde) | — | — | excluded |

\* table data excluded, accessors covered.
† Tier-B target = line coverage of lowering logic; semantic correctness stays
  with the integration harness.
‡ syscall/UI/subprocess-bound remainder excluded-with-justification + covered
  by integration fixtures.

---

## Conventions for writing the tests

- **Co-locate** tests in each file under `#[cfg(test)] mod tests { use
  super::*; … }` — matches every existing test module in the repo. No separate
  per-module files unless a `tests.rs` already exists from a plan-11 split.
- **Shared fixtures** → `src/testutil.rs` (`#[cfg(test)]`), holding the
  `Function`/`Statement`/`Expression`/IR builders currently duplicated inside
  `escape.rs`. Submodules `use crate::testutil::*`.
- **Table-driven** where the input space is enumerable (operators, builtin
  signatures, error codes, lexer tokens): one `#[test]` iterating a `&[(input,
  expected)]` slice, with the case in the assertion message.
- **Round-trip** for every serializer/codec (`ir` JSON+binary, `binary_repr`,
  `nir`, `ast` JSON, Mach-O/ELF writers): `decode(encode(x)) == x` plus
  explicit malformed/truncated-input error cases — these are the lines most
  often missed.
- **Negative paths first** when chasing the last 5% — error branches, bounds
  checks, and `Result::Err` arms are where coverage gaps concentrate.
- **No network/filesystem/process** in unit tests. Use `tempfile` (already a
  dev-dep) for unavoidable path work; anything needing a real socket or the
  built binary belongs in `tests/`.
- **Mark exclusions inline** with `// GRCOV_EXCL_START/STOP` and a one-line
  reason, so the denominator is auditable in review.

---

## Execution phases

Land coverage tooling first (so every subsequent PR shows its delta), then
work file-by-file, **lowest-tier-and-highest-value first**. Commit per file or
per cohesive group on the current branch (no branches per repo policy).

- **Phase 0 — Tooling & baseline.** Add `rust-toolchain.toml`,
  `scripts/coverage.sh`, `scripts/coverage-check.sh`, `tarpaulin.toml`,
  `.github/workflows/coverage.yml`, `src/testutil.rs`. Record the **baseline
  per-file report** as the starting line. Finalize the `mod.rs` exclusion list.
- **Phase 1 — Tier A front-end.** `numeric`, `escape`, `lexer`, `rules`,
  `target`, then the big three `ast` / `ir` / `binary_repr`, then
  `typecheck`, `resolver`, `monomorph`. Biggest coverage gain per test.
- **Phase 2 — Builtins + audit + man.** Mostly signature/validation tables;
  fast wins to 98%.
- **Phase 3 — Tier B codegen.** Per-builder instruction/plan assertions.
  Start with `validate.rs`, `plan.rs`, `runtime.rs`, `nir.rs` (more
  data-shaped), then the `builder_*` family, then `code/mod.rs`. Coordinate
  with `plan-11-large-files.md`: split a file *then* cover it when the split
  exposes seams.
- **Phase 4 — Tier C OS writers.** `os/macos/object.rs`, full Linux backend,
  extend `os/macos/link.rs`. Run grcov on macOS to capture platform-gated
  arms; union with the Linux CI run.
- **Phase 5 — Tier D CLI + repository crate.** Cover manifest parsing/
  validation in `main.rs` (after the optional `cli/`+`manifest/` split);
  fill `repository/src/{package,main,lib}.rs` gaps.
- **Phase 6 — Gate.** Flip `fail-under` to 95, enable per-file enforcement in
  `coverage-check.sh`, document the justified-exception list, and require the
  `coverage` CI job to pass on PRs.

---

## Verification & acceptance criteria

Per file/PR:

1. `cargo test --workspace` passes (unit + existing integration suite — the
   `tests/` harness must stay green; unit tests never replace it).
2. `sh scripts/coverage.sh` then `sh scripts/coverage-check.sh` reports the
   touched file ≥95% (or on the documented exception list with a reason).
3. `cargo fmt --check` and `cargo clippy` clean on the new test code.

Project-complete when:

- Every in-scope file is ≥95% line coverage in **both** the grcov (macOS) and
  tarpaulin (Linux CI) reports, with their overall numbers agreeing within
  rounding.
- The CI `coverage` job gates PRs at `--fail-under 95` and per-file check
  passes.
- Every excluded region carries an inline reason; the exception list (Tier C
  syscall/UI lines, generated tables) is enumerated in this file's tracking
  table and reviewed.
