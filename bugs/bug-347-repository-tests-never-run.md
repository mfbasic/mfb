# bug-347: `repository/` is outside the Cargo workspace, so its 123 tests never run and ~13k lines are invisible to the CI coverage gate

Last updated: 2026-07-18
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Test infrastructure (silently-dead gate)

Status: Open
Regression Test: `cargo test --workspace --no-run` must list a `mfb_repository` unittests binary

The root `Cargo.toml` declares no `[workspace]` section, so the workspace
contains exactly one member: `mfb`. `mfb_repository` is pulled in only as a
`path` dependency. Cargo therefore never builds its **test** targets under
`cargo test`, `cargo test --workspace`, or `cargo llvm-cov --workspace` — the
lib is compiled (as a dependency) but its `#[cfg(test)]` modules are not.

The result is 123 `#[test]` functions across 11 files that have never executed
in this repository's normal test flow, and ~13,214 lines of Rust — including the
package registry's HTTP surface, its SQLite store, its credential handling, and
its crypto — that are outside every automated gate. This is silent in the worst
way: `.github/workflows/coverage.yml` enforces a **global 95% line floor** and a
per-file 95% gate, both of which pass today precisely *because* this code is not
in the denominator.

The single correct behavior a fix produces: `cargo test` at the repo root builds
and runs `mfb_repository`'s test targets, and `scripts/coverage.sh` measures
`repository/src/**`.

References:

- `Cargo.toml` (root) — no `[workspace]` section; `mfb_repository = { path = "repository" }`.
- `repository/Cargo.toml` — `[package] name = "mfb_repository"`, with a `[[bin]] name = "mfb-repo"`.
- `scripts/coverage.sh:17` — the `IGNORE` regex.
- `scripts/coverage-check.sh:15` — the same regex, per-file 95% gate.
- `.github/workflows/coverage.yml:18-25` — per-file gate + `--fail-under-lines 95` global floor.
- `tests/repo_acceptance.rs:28-32` — the `cargo build --manifest-path` workaround.
- Found during the cleanup-focused source review (worktree `cleanup-review`).

## Failing Reproduction

```sh
cargo metadata --no-deps --format-version 1 \
  | python3 -c "import json,sys; print([p['name'] for p in json.load(sys.stdin)['packages']])"
```

- Observed: `['mfb']`
- Expected (after fix): `['mfb', 'mfb_repository']`

```sh
cargo test --workspace --no-run 2>&1 | grep -i executable
```

- Observed (2026-07-18, `b12213d2`): 20 binaries, **none** of them
  `mfb_repository`:

```
  Executable unittests src/main.rs (target/debug/deps/mfb-bc126d42e10a1482)
  Executable tests/build_verbosity_output.rs (…)
  … 17 more, all `mfb`'s own integration tests …
  Executable tests/repo_acceptance.rs (target/debug/deps/repo_acceptance-…)
  Executable tests/syscall_return_robustness.rs (…)
```

- Expected: an additional `Executable unittests src/lib.rs (…/mfb_repository-…)`,
  plus `unittests src/main.rs (…/mfb_repo-…)` and `tests/s3_backend.rs`.

Contrast — those three binaries *do* exist and *do* build; they are simply never
selected:

```sh
cargo test -p mfb_repository --no-run
```

- Observed:

```
  Executable unittests src/lib.rs (target/debug/deps/mfb_repository-4a2620ec38a7779c)
  Executable unittests src/main.rs (target/debug/deps/mfb_repo-35ebb1e0c21d78e2)
  Executable tests/s3_backend.rs (target/debug/deps/s3_backend-5969d5f7a8e94d4a)
```

`-p` reaches the package through the dependency graph; `--workspace` does not,
because `--workspace` means "all workspace members" and the member list is
`['mfb']`. That asymmetry is the whole bug — the tests are *reachable* but never
*reached*.

Corroborating: `repository/Cargo.lock` exists (79,060 bytes, tracked), which is
only meaningful for a workspace root. A `path` dependency inside a workspace
would not carry its own lockfile.

## Quantification

`grep -rn '#\[test\]' repository/ --include='*.rs' | wc -l` → **123**.

| File | `#[test]` fns |
| --- | --- |
| `repository/src/store.rs` | 30 |
| `repository/src/client.rs` | 22 |
| `repository/src/package.rs` | 15 |
| `repository/src/abi.rs` | 11 |
| `repository/src/log.rs` | 9 |
| `repository/src/main.rs` | 8 |
| `repository/src/local.rs` | 8 |
| `repository/src/blobstore.rs` | 7 |
| `repository/src/validation.rs` | 6 |
| `repository/src/crypto.rs` | 5 |
| `repository/src/server.rs` | 2 |
| **total** | **123** |

`find repository/src repository/tests -name '*.rs' | xargs wc -l` → **13,214**
lines, of which `repository/src/server.rs` alone is **4,060** — and that file
carries just 2 tests (`#[cfg(test)]` at `repository/src/server.rs:2375`). So the
single largest file in the sub-crate is both the least tested *and* entirely
unmeasured.

## Root Cause

Cargo's `--workspace` flag selects workspace **members**. With no `[workspace]`
table in the root `Cargo.toml`, the workspace is the implicit single-package
workspace `{ mfb }`; `repository/` is a separate workspace rooted at
`repository/Cargo.toml` (which its own `Cargo.lock` confirms). Every
workspace-scoped command therefore skips it:

- `cargo test` / `cargo test --workspace` — builds `mfb_repository` as a *lib
  dependency* only. A dependency's `#[cfg(test)]` code is compiled out, so the
  123 tests are not merely unrun, they are not even codegen'd.
- `scripts/coverage.sh:38` — `cargo llvm-cov --workspace --all-targets` inherits
  the same member set. Its `IGNORE` regex
  (`scripts/coverage.sh:17`) excludes `repository/target/` — a build-artifact
  exclusion — and notably does **not** exclude `repository/src/`. So the
  denominator's treatment of `repository/src/**` is decided by member selection,
  not by an explicit policy anyone wrote down.
- `.github/workflows/coverage.yml:18-25` — the per-file 95% gate and the global
  `--fail-under-lines 95` floor both read that same profile. Both are green over
  a corpus that omits the registry server entirely.

The workaround already in the tree is the tell:
`tests/repo_acceptance.rs:28-32` shells out to
`Command::new("cargo") … "--manifest-path", "repository/Cargo.toml"` **from
inside a test** to get the `mfb-repo` binary built. That is a test invoking the
build system to compensate for the split — nested cargo invocations that share a
target dir, ignore the parent's profile/feature selection, and cannot be
instrumented for coverage.

## Goal

- `cargo test --workspace --no-run` lists a `mfb_repository` unittests binary.
- `cargo test` at the repo root executes all 123 `repository/` tests.
- `scripts/coverage.sh` produces a per-file coverage figure for
  `repository/src/**`.
- `tests/repo_acceptance.rs` obtains the `mfb-repo` binary via
  `env!("CARGO_BIN_EXE_mfb-repo")` instead of spawning `cargo`.

### Non-goals (must NOT change)

- The `s3` feature must stay **off by default**. `repository/Cargo.toml`
  documents why (the AWS SDK would otherwise be compiled into every `mfb`
  build); joining the workspace must not implicitly enable it, and feature
  unification must be checked, not assumed.
- The `mfb-repo` binary's name, CLI, and on-disk data format.
- Do **not** make this green by *adding* `repository/src/` to the coverage
  `IGNORE` regex. That converts an accidental blind spot into a deliberate one
  and is explicitly forbidden.
- Do not delete or `#[ignore]` any of the 123 tests to get the suite passing.

## Blast Radius

- `Cargo.toml` (root) — needs the `[workspace]` table; fixed by this bug.
- `repository/Cargo.lock` — must be deleted; a workspace member cannot have its
  own lockfile, and the root lock becomes authoritative. Dependency versions may
  resolve differently on unification — that delta must be reviewed, not assumed
  benign.
- `tests/repo_acceptance.rs:28-32` — the in-test `cargo build`; replaced by
  `env!("CARGO_BIN_EXE_mfb-repo")`, which Cargo only defines for a workspace
  member's binaries. Fixed by this bug.
- `scripts/coverage.sh:17` and `scripts/coverage-check.sh:15` — the `IGNORE`
  regexes and `.github/workflows/coverage.yml:21` (a third, hand-duplicated copy
  of the same regex). All three must agree after the decision below. The
  triplication is itself a hazard: a future edit to one will silently diverge.
- `scripts/coverage-exceptions.txt` — the documented per-file exemption list;
  the likely landing place for any `repository/src/**` carve-out.
- `repository/src/server.rs` (4,060 lines, 2 tests) — the file most likely to
  fail a newly-applied 95% per-file gate.
- `repository/tests/s3_backend.rs` (92 lines) — gated behind the `s3` feature;
  will start being *built* under `--all-targets` even with the feature off.
  Verify it still compiles in that configuration.
- The `mfb` compiler crate — **unaffected** at the source level; only its
  measured coverage denominator changes.

## Fix Design

Three mechanical changes plus one policy decision.

1. Root `Cargo.toml` gains:

   ```toml
   [workspace]
   members = [".", "repository"]
   ```

2. `git rm repository/Cargo.lock`. Re-resolve and inspect the resulting root
   `Cargo.lock` diff for version movement caused by feature/version unification
   between `mfb` and `mfb_repository` (both depend on `sha2` and `serde_json`,
   and both pull `tempfile`).

3. `tests/repo_acceptance.rs` uses `env!("CARGO_BIN_EXE_mfb-repo")` and drops the
   `Command::new("cargo")` block at `:28-32`.

Rejected: keeping the split and adding a second CI job that runs
`cargo test --manifest-path repository/Cargo.toml`. It would run the tests, but
the coverage profiles cannot be merged across two workspaces, so the global floor
would remain blind — which is the more consequential half of this bug.

Rejected: making `repository` a member but excluding it from coverage via
`IGNORE`. Same objection as the Non-goal above.

**Expected shift in outputs:** the global line-coverage number will move
(direction unknown until measured — the 123 tests may well cover their own files
densely). The per-file gate will produce a new set of below-floor files under
`repository/src/**`. That delta is the point of the change and must be reported,
not suppressed.

## Phases

### Phase 1 — reproduce + quantify (no behavior change)

- [ ] Record `cargo metadata --no-deps` output and the `cargo test --workspace
      --no-run` binary list (done — see Failing Reproduction).
- [ ] Record the 123-test / 13,214-line census (done — see Quantification).
- [ ] Run `cargo test -p mfb_repository` and record how many of the 123 tests
      actually **pass** today. This is the unknown that sizes Phase 2: tests that
      have not run in months may have rotted.

Acceptance: the pass/fail split of the 123 tests is known and written into this
file.
Commit: —

### Phase 2 — join the workspace

- [ ] Add `[workspace] members = [".", "repository"]` to the root `Cargo.toml`.
- [ ] `git rm repository/Cargo.lock`; review the root `Cargo.lock` diff and
      confirm no unintended version movement.
- [ ] Confirm `cargo build` does **not** pull the AWS SDK (the `s3` feature stays
      off under unification).
- [ ] Fix any of the 123 tests found broken in Phase 1.

Acceptance: `cargo test --workspace --no-run` lists the `mfb_repository`
binaries; `cargo test` is green; no new transitive dependencies in the default
build.
Commit: —

### Phase 3 — drop the workaround + resolve the coverage policy

- [ ] Replace `tests/repo_acceptance.rs:28-32` with
      `env!("CARGO_BIN_EXE_mfb-repo")`.
- [ ] Run `sh scripts/coverage.sh` and record the new global figure and the
      per-file report for `repository/src/**`.
- [ ] Apply the Open Decision below (gate now, or exempt with dated entries in
      `scripts/coverage-exceptions.txt`).
- [ ] Reconcile all three copies of the `IGNORE` regex
      (`scripts/coverage.sh:17`, `scripts/coverage-check.sh:15`,
      `.github/workflows/coverage.yml:21`).

Acceptance: `sh scripts/coverage.sh && sh scripts/coverage-check.sh` pass
locally; the CI global floor passes; `repository/src/**` appears in the report.
Commit: —

## Validation Plan

- Regression test(s): `cargo test --workspace --no-run` listing a
  `mfb_repository` unittests binary is the assertion. Consider pinning it as a
  CI step so the split cannot silently reappear.
- Runtime proof: `cargo test` at the root runs and passes all 123 tests;
  `tests/repo_acceptance.rs` passes without spawning `cargo`.
- Doc sync: `scripts/coverage.sh`'s header comment enumerates what the `IGNORE`
  regex excludes and why — it must be updated to state the `repository/src/**`
  policy explicitly, whichever way the decision goes.
- Full suite: `cargo test`, `sh scripts/coverage.sh`, `sh scripts/coverage-check.sh`,
  and `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Does the per-file 95% gate apply to `repository/src/**`?** This is the
  decision the fix forces, and it should be made deliberately rather than
  inherited from whatever the first coverage run reports.
  - *Recommended:* yes, apply it — with `repository/src/server.rs` (4,060 lines,
    2 tests) and any other shortfall recorded as **dated, justified** entries in
    `scripts/coverage-exceptions.txt` and burnt down. This keeps one standard for
    all Rust in the repo and makes the debt visible.
  - *Alternative:* exempt `repository/src/**` from the per-file gate while still
    counting it toward the global floor. Cheaper to land, but it establishes a
    second-class tier of Rust in the tree — and the registry server is
    network-facing code handling credentials and blobs, which is the worst place
    to have a lower bar.

## Summary

The engineering risk is **not** the three-line workspace change — it is Phase 1
(how many of 123 never-run tests still pass) and Phase 3 (the coverage delta and
the policy decision it forces). The likely outcome is that the global floor
survives while the per-file gate surfaces real debt in `repository/src/server.rs`.
Nothing in the compiler's own source changes; what changes is that ~13k lines of
network-facing Rust stop being invisible to every gate the project runs.
