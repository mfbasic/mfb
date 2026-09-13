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
| plan-131-D complete, or D dropped by the user and plan-131-C complete | `ls planning/plan-131-{C,D}-* 2>/dev/null` → no match | MET (2026-09-12: C and D archived at `362b89fb6`) |

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

- [x] Re-measure the final `scripts/` file list. Reconcile it with plan-131-A §3 and record
      differences in Corrections.
      `git ls-files scripts` → 40 tracked files, plus the 3 D helpers (`remote-common.sh`, `rgba_compare.py`,
      `remote-common-selftest.sh`) = 43. plan-131-A §3 predicted 42 with D (see Corrections).
- [x] Rewrite `scripts/README.md`. For each file, read the script's header and usage block (never
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
      Done from each file's own header (`head -n 14` of all 43 read before writing): 43 `- **name**` entries in
      eight groups, each with purpose, usage and "Run by" (CI job, `cargo test` target, sourcing script, data
      consumer, or by hand).
- [x] Add a `README.md` to each tools directory that lacks one: read each directory's contents and
      write what it is and who consumes it.
      Written for `tools/link-package-sources`, `tools/oracles`, `tools/security-package-sources`,
      `tools/thread-package-sources`, from their contents and their consumers (`sync-package-mfp.sh`,
      `tests/rt-behavior/security/README.md`, `tests/net/rt_tls_listener_thread_transfer.rs`, the threading spec).

Acceptance: the Phase 2 census passes against this README (run it locally before committing Phase 1).
(Met: `cargo test --test scripts_index_is_complete` → 2 passed, exit 0.)
Commit: cca7e6683

### Phase 2 — Guards

- [x] Add `tests/gate/scripts_index_is_complete.rs` and its `[[test]]` entry.
- [x] Mutation checks, in the working tree only, each reverted:
      - `touch scripts/zz-unindexed.sh` → the test fails naming it;
      - delete one README bullet → the test fails naming that file;
      - add a bullet `- **ghost.sh**` → the test fails naming it;
      - `rm tools/mfbgen/README.md` → the test fails naming `tools/mfbgen`.
      Record each failure message.
      Results (`/tmp/p131-e-mutations.sh`, each exit 101, all restored, `cmp` identical):
      - `files with no index entry …: ["zz-unindexed.sh"]`;
      - bullet for `coverage-report.py` removed → `files with no index entry …: ["coverage-report.py"]`;
      - `index entries naming no file (remove them): ["ghost.sh"]`;
      - `these tools/ directories have no README.md …: ["tools/mfbgen"]`.
- [x] Read `scripts/test-accept-selftest.sh`'s usage. Add `tests/gate/script_selftests.rs` and its
      `[[test]]` entry. Run it → pass. Mutation: make one selftest assertion false in the working
      tree → the test fails; revert.
      Usage: no arguments, no binary (it passed with none). `script_selftests.rs` spawns both selftests,
      `#[cfg(unix)]` because they need perl/ssh/python3/pgrep, which a Windows runner lacks. Run → 2 passed
      (acceptance selftest 62 s). Mutation: `rc_failures -eq 2` → `-eq 3` in `remote-common-selftest.sh` →
      exit 101, `BAD  pass/fail: two fails counted`; restored.
- [x] ~~Fresh worktree: build and run both new tests in `git worktree add --detach /tmp/wt-131e`~~ —
      replaced 2026-09-12 (a fresh worktree is a full cold build). The failure it guards against is a
      census that passes only because of an untracked/ignored local file. Cheapest check that fails on
      exactly that: `git status --short --ignored scripts tools` → no ignored or untracked file under
      the census roots (est. seconds).
      The census skips dot-files and directories by construction, so an ignored `__pycache__` or a selftest's
      `.selftest-*` copy cannot satisfy it; the index lists only tracked files plus the three D helpers
      committed at `f18268512`.
- [x] AGENTS.md: add the §3 rule. (Added under "Always", naming the census test.)
- [x] Record the memory update needed: a feedback/project lesson that `scripts/` is indexed and
      census-guarded, and where generators and probes go. A sub-agent makes it, per AGENTS.md.
      Dispatched to a sub-agent on 2026-09-12.

Acceptance:
- `cargo test --test scripts_index_is_complete --test script_selftests --test gate_lock_covers_every_writer --no-fail-fast`
  passes (est. <3 min incremental), and the ignored-file check above is empty.
- All mutation checks failed as recorded.
  (Met: `cargo test --test scripts_index_is_complete --test script_selftests --test gate_lock_covers_every_writer`
  → 2 + 2 + 1 passed, exit 0; five mutations each exit 101 by name.)

Commit: cca7e6683

### Phase 3 — Network harnesses in CI (user decision, 2026-09-12)

- [x] Read `check-net-harness-selftest.sh` and the four harnesses for their host needs (python3,
      openssl, `unshare -Urn` / `sandbox-exec`, loopback). Choose the runner where every leg runs
      rather than SKIPs, and record what each leg needs.
      tcp: python3 (blackhole server). udp: python3 (echo peer). tls: openssl (identity + `s_client`), and on
      Linux the MFBASIC client leg too. icmp: on Linux `unshare -Urn` plus a permitted baseline — the
      harness SKIPs if ICMP is already denied or user namespaces are unavailable, and a SKIP also passes the
      sabotaged copy, which the selftest reports as a broken harness. Runner: `ubuntu-latest`, with
      `net.ipv4.ping_group_range` opened and `kernel.apparmor_restrict_unprivileged_userns=0` (Ubuntu 24.04
      blocks unprivileged user namespaces).
- [x] Add a job (or a step in an existing job) to `.github/workflows/` that builds the release
      binary and runs `bash scripts/check-net-harness-selftest.sh target/release/mfb`, failing the
      job on non-zero exit.
      `net-harness` in `.github/workflows/coverage.yml`: needs `build`, downloads the `mfb-bin` artifact, sets
      the two sysctls, runs the selftest.
- [x] Update the `coverage.yml` comment near the ICMP-denied note, and the `scripts/README.md`
      "Run by" fields for the five network files, to name the CI job.
      The acceptance job's comment now says the `net-harness` job runs the check; `scripts/README.md` names
      CI `net-harness` for the selftest and the four harnesses it drives.
- [x] Prove the job locally as far as possible: run the exact step commands on the runner's OS
      class (Linux box 2227/2228, or locally for macOS) → exit 0; and push nothing without asking.
      Box: 2223 (native aarch64 Linux), not the emulated 2227/2228; `bash scripts/check-net-harness-selftest.sh
      <mfb>` (est. <5 min) → exit 0.
      First run on 2223 FAILED 7 of 8 legs: every harness ran `<name>-musl.out` on a glibc host (exit 127,
      "required file not found"), because each took the LAST `Wrote executable to` line and a Linux build writes
      both flavors. Fixed in all four harnesses (a `host_exe` filter keeps the flavor the host's `ldd` reports;
      macOS writes one `.out` and passes through). After the fix: 2223 (`Linux aarch64`, `GLIBC 2.43`) →
      8/8 `ok`, `PASS: every networking harness …`, exit 0, 5 s; macOS → 8/8 `ok`, exit 0. The CI job would
      have been red on its first run without the fix. Nothing was pushed.

Acceptance:
- The workflow YAML parses (`python3 -c 'import yaml,sys; yaml.safe_load(open(sys.argv[1]))' <file>`).
  (Met: `yaml.safe_load` of `coverage.yml` succeeded after the edit.)
- The step's command exits 0 on a host of the runner's OS class, and a sabotaged harness makes it
  exit non-zero.
  (Met: exit 0 on glibc Linux 2223; the selftest's own four sabotaged copies each FAIL for their injected reason.)

Commit: 642d9c8fd

## Validation Plan

- Tests: the two new `tests/gate` tests plus the existing census and lock tests.
- Final gate — runs ONCE for all of plan-131, after E's last phase; no letter repeats it. Start each
  in the background and keep working; measure and record each wall time:
  - `cargo test --release --no-fail-fast` (per memory, never piped to `tail`);
  - `sh scripts/check-generated.sh`;
  - `bash scripts/artifact-gate.sh target/release/mfb all`;
  - `bash scripts/man-examples-gate.sh target/release/mfb`.
- Doc sync: AGENTS.md, `scripts/README.md`, the `tools/*/README.md` files.

## Open Decisions

See plan-131-A (network harnesses in CI).

## Corrections

- **2026-09-12: `scripts/` holds 43 files, not 42.** D added `remote-common-selftest.sh` beside the two
  helpers plan-131-A §3 counted. Measured: `git ls-files scripts` after D → 43 (including `README.md`).
- **2026-09-12: the networking harnesses never worked on Linux glibc.** Each ran the last executable a build
  wrote, which on Linux is the musl flavor. Found by the Phase 3 proof on native box 2223 (7 of 8 legs failed
  with exit 127); fixed with a libc-aware `host_exe` filter in all four harnesses and re-proved on 2223 and
  macOS. The new CI job depends on this fix.
- **2026-09-12: the 2223 proof could not set the CI job's sysctls** (`sudo: a password is required`). It did
  not need to: 2223 already reads `ping_group_range = 0 2147483647` and has no AppArmor user-namespace
  restriction, so every leg ran and none SKIPped.

- **2026-09-12: the network-harness CI decision reverses a non-goal.** The user chose "add a CI job"
  over run-by-hand. Added Phase 3 (below) to wire `check-net-harness-selftest.sh` into CI. Plan-131-A
  Phase 3's `coverage.yml:213` comment is written to match: the check becomes a CI job in E.

## Summary

Low risk. The value is that the mess cannot come back silently: the next file dropped into
`scripts/` without an index entry fails `cargo test`. Script behavior is untouched.
