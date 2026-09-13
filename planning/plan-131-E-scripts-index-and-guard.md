# plan-131-E: scripts/ cleanup — one complete index, and a test that keeps it complete

Last updated: 2026-09-12
Effort: medium (1h–2h)
Depends on: plan-131-D (or plan-131-C, if the user dropped D)

The dumping ground happened because nothing stopped it. A file could land in `scripts/` with no
README entry, and 36 did (plan-131-A §2). This sub-plan rewrites `scripts/README.md` as a complete
index of the final layout. It adds a `cargo test` census that fails when the index and the
directory disagree, and it writes the `scripts/` vs `tools/` rule into AGENTS.md so agents see it
before adding a file.

References:

- `plan-131-A-scripts-fix-and-delete.md` — prerequisites gate, §3 rule and inventory.
- `scripts/README.md` — the index being rewritten.
- `tests/gate/gate_lock_covers_every_writer.rs` — the precedent for a filesystem census test in
  this tree.
- Memory `pin-over-a-gitignored-file-is-not-a-pin`, `source-census-needle-matches-its-own-message`.

## Prerequisites

See plan-131-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-131-D complete, or D dropped by the user and plan-131-C complete | `ls planning/plan-131-{C,D}-* 2>/dev/null` → no match | NOT MET |

## 1. Goal

- `scripts/README.md` has exactly one entry per file in `scripts/`, and no entry for a missing file.
- Each entry states: purpose, usage, and who runs it (`CI: <job>` / `cargo test: <test>` /
  `by hand` / `sourced by <scripts>` / `data for <script>`).
- `tests/gate/scripts_index_is_complete.rs` fails on:
  - an unindexed file;
  - a dangling entry;
  - a `tools/<dir>` without a `README.md`.
- AGENTS.md states the rule.
- `test-accept-selftest.sh` and `remote-common-selftest.sh` (if D landed) are run by `cargo test`,
  not only by hand.

### Non-goals

- No script behavior changes.
- ~~Network harnesses stay out of CI unless the user decides otherwise.~~ The user decided to add a
  CI job (plan-131-A Open Decisions, 2026-09-12); Phase 3 below does it.
- Subdirectories are not introduced.

## 2. Current State

- `scripts/README.md` groups entries under `##` headings, one `- **name** — …` bullet each
  (read 2026-09-12). This bullet shape is the needle the census parses.
- The final file set is UNMEASURED until C/D land. Re-run
  `git ls-files scripts | grep '^scripts/[^/]*$'` at the start of Phase 1 and compare it with the
  plan-131-A §3 inventory. Any difference goes in Corrections.
- `tools/` directories without a README (`for d in tools/*/; do [ -f $d/README.md ] || echo $d; done`,
  run 2026-09-12): `tools/link-package-sources/`, `tools/oracles/`,
  `tools/security-package-sources/`, `tools/thread-package-sources/`.
- Whether `test-accept-selftest.sh` needs arguments or a built binary: UNVERIFIED. Read its usage
  block in Phase 2.

## 3. Design

- **Census test.** `tests/gate/scripts_index_is_complete.rs`, registered as a `[[test]]` in
  `Cargo.toml` next to the other `tests/gate` entries. It:
  - `read_dir("scripts")` → the regular files, skipping dot-files and directories. Directories are
    skipped because the gitignored `__pycache__` must not count, and a census over an ignored file
    is not a pin;
  - parses every `- **<name>**` bullet in `scripts/README.md`;
  - asserts the two sets are equal and prints both differences;
  - asserts every `tools/*/` directory has a `README.md`.
  Its failure message spells the needle with `concat!`, so the message text cannot satisfy the
  census it reports on.
- **Selftest spawn test.** `tests/gate/script_selftests.rs` spawns `bash scripts/test-accept-selftest.sh`
  (and `scripts/remote-common-selftest.sh` if present) and asserts exit 0 with stdout attached.
- **AGENTS.md rule**, under "Always":
  - `scripts/` holds reusable gates, harnesses and maintenance actions, plus the files they source.
    Every file is indexed in `scripts/README.md`.
  - Generators, probes, oracles and benchmarks live under `tools/<name>/` with a README.
  - A one-off probe for one bug or plan goes in `/tmp` or the bug doc, never in `scripts/`.
  - Deleting a script means grepping for its name and updating every live caller in the same commit.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit; `- [~]` partial; moot
> tasks struck through with evidence; fill `Commit:`. **An unticked box means NOT DONE.**

### Phase 1 — Index

- [ ] Re-measure the final `scripts/` file list. Reconcile it with plan-131-A §3 and record
      differences in Corrections.
- [ ] Rewrite `scripts/README.md`. For each file, read the script's header and usage block (never
      write an entry from memory) and give it one `- **name** — purpose. Usage: `…`. Run by: …`
      entry. Suggested groups:
      - Gates (CI)
      - Golden maintenance
      - Acceptance harness
      - Platform proofs (remote boxes)
      - Network harnesses
      - Man docs
      - Coverage
      - Shared libraries and data
      Add a top paragraph stating the §3 rule and pointing to `tools/`.
- [ ] Add a `README.md` to each tools directory that lacks one: read each directory's contents and
      write what it is and who consumes it.

Acceptance: the Phase 2 census passes against this README (run it locally before committing Phase 1).
Commit: —

### Phase 2 — Guards

- [ ] Add `tests/gate/scripts_index_is_complete.rs` and its `[[test]]` entry.
- [ ] Mutation checks, in the working tree only, each reverted:
      - `touch scripts/zz-unindexed.sh` → the test fails naming it;
      - delete one README bullet → the test fails naming that file;
      - add a bullet `- **ghost.sh**` → the test fails naming it;
      - `rm tools/mfbgen/README.md` → the test fails naming `tools/mfbgen`.
      Record each failure message.
- [ ] Read `scripts/test-accept-selftest.sh`'s usage. Add `tests/gate/script_selftests.rs` and its
      `[[test]]` entry. Run it → pass. Mutation: make one selftest assertion false in the working
      tree → the test fails; revert.
- [ ] Fresh worktree: `git worktree add --detach /tmp/wt-131e HEAD` and run both new tests there →
      pass. Then remove the worktree.
- [ ] AGENTS.md: add the §3 rule.
- [ ] Record the memory update needed: a feedback/project lesson that `scripts/` is indexed and
      census-guarded, and where generators and probes go. A sub-agent makes it, per AGENTS.md.

Acceptance:
- `cargo test --test scripts_index_is_complete --test script_selftests --test gate_lock_covers_every_writer --no-fail-fast`
  passes in the main tree and the fresh worktree.
- All mutation checks failed as recorded.

Commit: —

### Phase 3 — Network harnesses in CI (user decision, 2026-09-12)

- [ ] Read `check-net-harness-selftest.sh` and the four harnesses for their host needs (python3,
      openssl, `unshare -Urn` / `sandbox-exec`, loopback). Choose the runner where every leg runs
      rather than SKIPs, and record what each leg needs.
- [ ] Add a job (or a step in an existing job) to `.github/workflows/` that builds the release
      binary and runs `bash scripts/check-net-harness-selftest.sh target/release/mfb`, failing the
      job on non-zero exit.
- [ ] Update the `coverage.yml` comment near the ICMP-denied note, and the `scripts/README.md`
      "Run by" fields for the five network files, to name the CI job.
- [ ] Prove the job locally as far as possible: run the exact step commands on the runner's OS
      class (Linux box 2227/2228, or locally for macOS) → exit 0; and push nothing without asking.

Acceptance:
- The workflow YAML parses (`python3 -c 'import yaml,sys; yaml.safe_load(open(sys.argv[1]))' <file>`).
- The step's command exits 0 on a host of the runner's OS class, and a sabotaged harness makes it
  exit non-zero.

Commit: —

## Validation Plan

- Tests: the two new `tests/gate` tests plus the existing census and lock tests.
- Full suite before archiving plan-131:
  - `cargo test --release --no-fail-fast` (per memory, never piped to `tail`);
  - `sh scripts/check-generated.sh`;
  - `bash scripts/artifact-gate.sh target/release/mfb all`;
  - `bash scripts/man-examples-gate.sh target/release/mfb`.
- Doc sync: AGENTS.md, `scripts/README.md`, the `tools/*/README.md` files.

## Open Decisions

See plan-131-A (network harnesses in CI).

## Corrections

- **2026-09-12: the network-harness CI decision reverses a non-goal.** The user chose "add a CI job"
  over run-by-hand. Added Phase 3 (below) to wire `check-net-harness-selftest.sh` into CI. Plan-131-A
  Phase 3's `coverage.yml:213` comment is written to match: the check becomes a CI job in E.

## Summary

Low risk. The value is that the mess cannot come back silently: the next file dropped into
`scripts/` without an index entry fails `cargo test`. Script behavior is untouched.
