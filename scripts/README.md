# scripts/

Maintenance, gating, and code-generation scripts for the MFBASIC compiler. Most
gate scripts take the compiler binary as their first argument (e.g.
`target/release/mfb`) and resolve the repo root from their own location, so they
can be run from anywhere. Entries below are grouped by purpose.

## Codegen / byte-identity gates

Prove the compiler's emitted bytes did not change (or did not change
unintentionally) across a refactor.

- **artifact-gate.sh** — The fast codegen gate: regenerates only the
  deterministic build dumps (`.ncode`, `.nobj`, etc.) with no link or run, and
  diffs them against committed goldens. Multi-target — a fixture's `linux-*`
  goldens regenerate via cross-compile even on a macOS host. Usage:
  `artifact-gate.sh <mfb-exe> <builtin|all>`.
- **artifact-kinds.sh** — Shared data table (not executable) listing every
  execution-free codegen dump `mfb build -<flag>` emits and how to produce it.
  Sourced by both `test-accept.sh` and `artifact-gate.sh` so the two can't drift
  about which dump kinds exist. Also owns `artifact_kind_is_level_variant`, which
  says whether a kind is emitted downstream of the `-O` dial (every per-target
  native dump is) and so cannot be compared against a default-level golden.
- **ncode-determinism-alltargets.sh** — Determinism harness: compiles each
  in-scope fixture N times in fresh processes (fresh HashMap seeds) for every
  goldened target (the host plus the three `linux-*` cross targets) and counts
  distinct `.ncode` hashes per target, comparing each against its
  `<target>.ncodesum` golden, so residual nondeterminism or a stale golden shows.
  Usage: `ncode-determinism-alltargets.sh <mfb-binary> [N]` (N defaults to 50).
- **regen-native-goldens.sh** — Rewrites every existing per-target native golden
  (`<pkg>.<target>[.app].<nir|nplan|nobj|ncode|mir>[sum]`) after an intended
  codegen change: rebuilds the target (and app mode) named in each filename,
  copies the dump or writes its sha256. The enumeration is `artifact-gate.sh`'s,
  so it rewrites exactly what the gate checks. Never creates a golden, never
  writes one whose build failed; takes the gate lock; host from `uname`. Usage:
  `regen-native-goldens.sh <mfb-exe> [fixture-dir...]` (no dirs = all of `tests/`).
- **artifact-baseline.sh** — Captures or verifies a SHA-256 manifest of every
  codegen dump and every linked `.out` the compiler emits for a set of targets,
  building each fixture in a scratch copy (never in-tree, so no gate lock). Covers
  what `artifact-gate.sh` cannot: linked executables and fixtures with no golden.
  Default targets are the three Linux ones; `--targets macos-aarch64` is the
  host full-executable oracle. Usage:
  `artifact-baseline.sh <mfb-exe> capture|verify <manifest> [--targets t1,t2]`
  (`FILTER=`, `JOBS=`; use a release `mfb` and `JOBS=10`).
- **test-accept.sh** — The full acceptance harness: builds and runs every
  fixture under `tests/`, comparing produced artifacts and program output against
  committed goldens. Refuses to run concurrently with another copy. Usage:
  `test-accept.sh <mfb-exe> <actual-output-dir> [name-glob ...]`. `MFB_OPT=<n>`
  re-runs the suite at optimizer level `n`; at any level other than the default
  `1` the per-target native dumps are skipped (and counted in the summary line),
  so a healthy tree exits 0 at every level.
- **test-accept-selftest.sh** — Self-test for the harness's own per-fixture
  watchdog (bug-320): exercises the timeout helper directly so a program that
  blocks forever fails *that* fixture instead of wedging the whole suite. Also
  covers the rival-process guard (bug-455) and the `MFB_OPT` level-variant golden
  skip (bug-456), extracting each decision from the shipping scripts rather than
  restating it.
- **sync-goldens.sh** — Regenerates existing golden files in place by running the
  harness and copying each freshly produced "actual" over its golden. Never
  creates new goldens; forwards a name-glob so a single-fixture sync only runs
  that fixture. Usage: `sync-goldens.sh <mfb-exe> [name-glob ...]`.
- **test-macapp.sh** — Runtime acceptance for macOS app mode: builds an app-mode
  `.app` bundle and launches it headlessly, proving AppKit/Foundation bind and
  the worker thread runs the program entry. Requires a macOS window-server
  session. Usage: `test-macapp.sh <mfb-exe>`.
- **test-appimage.sh** — Linux counterpart of `test-macapp.sh`: builds an
  AppImage here, ships it over ssh to a real Linux box, runs it, and asserts —
  because an AppImage can't be emulated under qemu/Rosetta. Usage:
  `test-appimage.sh <mfb-exe> [--box <port>] [--libc glibc|musl|both] [--gui]`.
- **linux-runtime-proof.sh** — The behavioral half of the Linux proof: builds
  every runnable `.run` fixture on the host, ships each executable over ssh, runs
  it on the target hardware, and diffs the output against the fixture's
  `golden/build.log`. Usage:
  `linux-runtime-proof.sh <mfb-exe> <ssh-port> <target> [flavor]`.

## Coverage

- **coverage.sh** — Runs the instrumented workspace test suite once via
  cargo-llvm-cov and leaves the merged profile in place for the report step. Uses
  the same LLVM engine locally and in CI so numbers agree per platform. CI runs it
  with no arguments. `coverage.sh --bins` instead runs only the `mfb` unit tests and
  writes `target/coverage/coverage.json`: the same `src/**` measurement in minutes
  rather than hours (nothing under `tests/` links `src/**`), but meaningless for
  `repository/src/**`.
- **coverage-check.sh** — Per-file coverage gate: reads the profile left by
  `coverage.sh` and prints every in-scope source file below the floor, exiting
  non-zero if any fall short. Floor defaults to 98; override with `FLOOR=`.
- **coverage-common.sh** — Shared coverage settings (sourced, not executable):
  the `IGNORE` denominator-exclusion regex and `PKG_FLAGS` package selection,
  used by both coverage scripts and the CI global-floor step so they can't drift.
- **coverage-report.py** — Instruments over a llvm-cov JSON report, `src/**` only:
  `gaps` (files below the floor ranked by lines short), `lines <report> <file>
  [--source]` (uncovered ranges of one file), `shapes` (what kind of line each gap
  is), `dead` (functions that never ran, folded across instantiations) and `delta
  <base> [current]` (per-file movement between two reports). Applies
  `coverage-exceptions.txt` from its own directory, so it works from any cwd. Usage:
  `python3 scripts/coverage-report.py <gaps|lines|shapes|dead|delta> …` (`FLOOR=`).
- **coverage-exceptions.txt** — Data file listing source files exempt from the
  per-file 95% gate because their uncovered remainder (network/TTY/subprocess/GUI
  paths) is reachable only from the integration harness.

## Generated sources

- **check-generated.sh** — Generated-artifact integrity gate: re-runs each
  generator and fails if the checked-in artifact no longer matches, so "re-run the
  generator" is always safe and drift can't land.

## Packages

- **sync-package-mfp.sh** — Rebuilds every buildable package fixture from source
  and overwrites every committed copy of its `.mfp` (consumer and golden copies),
  which otherwise go stale when the binary-representation format changes. Skips the
  deliberately-tampered security fixtures.

## Network / riscv validation helpers

- **check-tcp-connect-timeout.sh** — Standalone runtime check for
  `tcp::connect`'s `timeoutMs`: starts a blackhole TCP server, then builds and
  runs a program that must fail with `ErrTimeout` well before the OS default
  connect timeout. Usage: `check-tcp-connect-timeout.sh <mfb-exe>`. (Was
  `check-net-connect-timeout.sh` against `net::connectTcp` before plan-110-B moved
  the transport into `tcp`.)
- **net_blackhole_server.py** — Helper for the above: a TCP server that saturates
  a tiny accept backlog so new connects get no SYN-ACK and block until their
  deadline. Prints its port and sleeps; started in the background by the check.
