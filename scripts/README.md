# scripts/

Reusable gates, harnesses and maintenance actions you run against the compiler or the
tree, plus the libraries and data files those scripts source. Every file here has
exactly one entry below; `cargo test --test scripts_index_is_complete` fails if a file
is added without one, or if an entry names a file that is gone.

What does NOT belong here: generators with their own inputs, measuring probes, oracles
and benchmarks live under `tools/<name>/`, each with a README. A one-off probe for a
single bug or plan lives in `/tmp` or in that bug's doc. Deleting a script means
grepping for its name and updating every live caller in the same commit.

"Run by" names what executes the file: a CI job in `.github/workflows/coverage.yml`,
a `cargo test` target, another script, or a person.

## Gates (CI)

- **check-generated.sh** — Re-runs every generator under `tools/` and fails if a
  committed generated artifact no longer matches its output (and runs the vector body
  checker), so re-running a generator is always safe and drift cannot land.
  Usage: `sh scripts/check-generated.sh`. Run by: CI `build`.
- **artifact-gate.sh** — Execution-free codegen gate: rebuilds each fixture's
  deterministic dumps (host front-end kinds, and every per-target native kind named by
  a golden's filename) and diffs them against the committed goldens. Takes the tree's
  gate lock. Usage: `bash scripts/artifact-gate.sh <mfb-exe> [all|<selection>]`.
  Run by: CI `artifact`; `cargo test --test golden`.
- **test-accept.sh** — The acceptance harness: builds and runs every fixture under
  `tests/` (or those matching the name globs), with a per-fixture watchdog, and diffs
  build logs, run output and artifacts against goldens into `<actual-output-dir>`.
  The second argument is removed and recreated. Usage:
  `bash scripts/test-accept.sh <mfb-exe> <actual-output-dir> [name-glob ...]`.
  Run by: CI `acceptance`.
- **man-examples-gate.sh** — Every Examples block on every built-in package's rendered
  `mfb man` pages must build, and run unless listed in `man-examples-not-run.txt`.
  Usage: `bash scripts/man-examples-gate.sh [path/to/mfb]`. Run by: CI `man-examples`.
- **build-examples.sh** — Builds every example project under `examples/` for every
  supported target into `target/examples/<example>/<target>/` and zips the tree to
  `target/examples.zip`. Usage: `bash scripts/build-examples.sh`. Run by: CI `examples`.

## Golden maintenance

- **regen-native-goldens.sh** — Rewrites every existing per-target native golden
  (`<pkg>.<target>[.app].<nir|nplan|nobj|ncode|mir>[sum]`) after an intended codegen
  change, rebuilding the target named in each filename. Never creates a golden, never
  writes one whose build failed, fails closed without a sha256 tool; takes the gate
  lock; host from `uname`. Usage: `bash scripts/regen-native-goldens.sh <mfb-exe>
  [fixture-dir...]`. Run by: by hand.
- **sync-goldens.sh** — Runs `test-accept.sh` and overwrites each EXISTING golden with
  its fresh actual output (new goldens are never created). Usage:
  `bash scripts/sync-goldens.sh <mfb-exe> [name-glob ...]`. Run by: by hand.
- **sync-package-mfp.sh** — Rebuilds every buildable package fixture from source
  (`tools/thread-package-sources`, `tools/link-package-sources`, `tests/`) and
  overwrites every committed copy of its `.mfp`, which otherwise go stale when the
  package format changes. Skips the deliberately tampered security packages.
  Usage: `bash scripts/sync-package-mfp.sh <mfb-exe>`. Run by: by hand.
- **artifact-baseline.sh** — Captures or verifies a SHA-256 manifest of every codegen
  dump and every linked `.out` for a set of targets, building each fixture in a
  scratch copy (no gate lock). Covers what `artifact-gate.sh` cannot: linked
  executables and fixtures with no golden. Default targets are the three Linux ones;
  `--targets macos-aarch64` is the host full-executable oracle. Usage:
  `bash scripts/artifact-baseline.sh <mfb-exe> capture|verify <manifest> [--targets t1,t2]`
  (`FILTER=`, `JOBS=`; use a release `mfb`). Run by: by hand.
- **ncode-determinism-alltargets.sh** — Compiles each in-scope fixture N times in fresh
  processes for every goldened target and counts distinct `.ncode` hashes per target,
  comparing each against its `.ncodesum` golden, so nondeterminism or a stale golden
  shows. Takes the gate lock. Usage:
  `bash scripts/ncode-determinism-alltargets.sh <mfb-binary> [N]` (N defaults to 50).
  Run by: by hand.
- **diag-set-diff.sh** — Diagnostic set-equality harness (plan-107-A): re-runs each
  golden's recorded `mfb build`/`mfb test` command and compares the SET of
  (file, line, code, detail) diagnostics, so an expected reorder is told apart from a
  real wording or line change. Takes the gate lock.
  Usage: `bash scripts/diag-set-diff.sh <mfb-exe>`. Run by: by hand.
- **regen-spirv.sh** — Regenerates the canvas Vulkan shaders' checked-in SPIR-V from
  their GLSL with glslang (a maintainer action when the GLSL changes; unit tests check
  the two stay in step). Usage: `bash scripts/regen-spirv.sh`. Run by: by hand.

## Acceptance harness

- **test-accept-selftest.sh** — Exercises `test-accept.sh`'s own machinery (the
  per-fixture watchdog, signal exit codes, stdin isolation, level-variant kinds)
  without a hanging fixture. Usage: `bash scripts/test-accept-selftest.sh`.
  Run by: `cargo test --test script_selftests`.
- **gate-lock.sh** — Per-tree mutual exclusion (bug-470) for every script that
  rewrites fixture dumps inside the tree; runs in different worktrees stay concurrent.
  Sourced, not executable. Run by: sourced by `artifact-gate.sh`, `test-accept.sh`,
  `sync-goldens.sh`, `regen-native-goldens.sh`, `ncode-determinism-alltargets.sh`,
  `diag-set-diff.sh`, `tools/bench-lowering/bench-lowering.sh`.
- **artifact-kinds.sh** — The shared table of execution-free dump kinds (host and
  per-target native) and the `-O` level-variance predicate, so the harness and the
  gate cannot drift about which dumps exist. Sourced, not executable.
  Run by: sourced by `test-accept.sh`, `artifact-gate.sh`, `regen-native-goldens.sh`.

## Platform proofs (remote boxes)

- **remote-common.sh** — Shared helpers for the platform proofs: `pass`/`fail`
  bookkeeping, a work dir with cleanup, `--box` parsing, `remote_ssh`/`remote_scp`
  (always `BatchMode` + `ConnectTimeout`, overridable with `MFB_SSH_CONNECT_TIMEOUT`),
  a `watchdog`, `win_ship`, `scaffold_project`. Sourced, not executable.
  Run by: sourced by `test-winprocess.sh`, `test-winapp.sh`, `test-appimage.sh`,
  `test-canvas-vulkan.sh`, `test-macapp.sh`, `linux-runtime-proof.sh`.
- **remote-common-selftest.sh** — Checks the helpers locally: the watchdog kills and
  passes through, `remote_ssh` fails fast on a closed port, failure counting, and
  `rgba_compare.py` at and beyond tolerance. Usage: `bash scripts/remote-common-selftest.sh`.
  Run by: `cargo test --test script_selftests`.
- **rgba_compare.py** — Compares two raw RGBA8 frames under `Tolerance::GPU_DEFAULT`
  (no channel off by more than 2, no more than 2% of pixels differing) and prints an
  `ok …` verdict or the first pixel beyond tolerance. Usage:
  `python3 scripts/rgba_compare.py <reference.rgba> <candidate.rgba> [width]`.
  Run by: `test-canvas-vulkan.sh`, `test-winapp.sh`.
- **test-macapp.sh** — Runtime acceptance for macOS app mode: builds `.app` bundles and
  runs them headlessly; the GUI legs (real windows, screenshots, injected keystrokes)
  run only with `MFB_MACAPP_GUI=1`, never while someone is using the machine.
  Usage: `bash scripts/test-macapp.sh <mfb-exe>`. Run by: by hand (macOS).
- **snap-macos.py** — Launches a macOS app and captures only its window to a PNG
  (needs Screen Recording permission). Usage: `python3 scripts/snap-macos.py <app> [name]`.
  Run by: `test-macapp.sh` (GUI legs).
- **test-appimage.sh** — Runtime acceptance for Linux app mode: builds the glibc and
  musl AppImages, ships them to real boxes (an AppImage cannot run under emulation of
  its ELF loader), and checks extraction, sonames, the payload and startup. Usage:
  `bash scripts/test-appimage.sh <mfb-exe> [--box <port>] [--libc glibc|musl|both] [--gui]`.
  Run by: by hand (boxes 2228/2227).
- **test-canvas-vulkan.sh** — Runtime acceptance for the canvas Vulkan backend: renders
  the same scenes on a Linux box with and without `MFB_CANVAS_GPU=1` and requires the
  frames to agree within tolerance, including resize and group scenes. Usage:
  `bash scripts/test-canvas-vulkan.sh <mfb-exe> [--box <port>] [--libc glibc|musl] [--icd auto|<manifest>]`.
  Run by: by hand (box 2228, or 2227 with `--libc musl --icd auto`).
- **test-winapp.sh** — Runtime acceptance for Windows `--app` builds: app-mode startup,
  file writes, environment, canvas software and Vulkan frames, resize and `term`
  gating, run on the Windows box. Usage: `bash scripts/test-winapp.sh <mfb-exe> [--box <port>]`.
  Run by: by hand (box 2230).
- **test-winprocess.sh** — Runtime acceptance for the Windows `process` backend on the
  console entry path: argv quoting, environment, working directory and stdin/stdout of
  spawned children, run on the Windows box.
  Usage: `bash scripts/test-winprocess.sh <mfb-exe> [--box <port>]`. Run by: by hand (box 2230).
- **linux-runtime-proof.sh** — Cross-builds the acceptance suite's runnable fixtures,
  ships each executable to a Linux box, runs it there and diffs its output against the
  golden. Usage: `bash scripts/linux-runtime-proof.sh <mfb-exe> <ssh-port> <target> [flavor]`
  (`FILTER=`, `JOBS=`, `RUN_TIMEOUT=`). Run by: by hand (prefer native box 2223).

## Network harnesses

- **check-net-harness-selftest.sh** — Runs each of the four networking harnesses as
  shipped (must pass) and against an injected wrong expectation (must fail for that
  reason). Usage: `bash scripts/check-net-harness-selftest.sh <mfb-exe>`.
  Run by: CI `net-harness`.
- **check-tcp-connect-timeout.sh** — Proves `tcp::connect`'s `timeoutMs` deadline
  against a blackhole server: the connect must fail with `ErrTimeout` well before the
  OS default. Usage: `bash scripts/check-tcp-connect-timeout.sh <mfb-exe>`.
  Run by: `check-net-harness-selftest.sh` (CI `net-harness`).
- **check-udp-echo.sh** — Proves `udp::send`/`udp::receive` against a real POSIX echo
  peer: byte-exact round trips (including empty and multi-byte payloads) and the sender
  address. Usage: `bash scripts/check-udp-echo.sh <mfb-exe>`.
  Run by: `check-net-harness-selftest.sh` (CI `net-harness`).
- **check-tls-loopback.sh** — Local TLS proof: an MFBASIC `tls::listen` server against
  `openssl s_client` (trusted and untrusted), plus the MFBASIC client leg where the
  backend accepts a local trust anchor; `--remote <ssh-port> [linux-target]` runs the
  client legs on a Linux box. Usage: `bash scripts/check-tls-loopback.sh <mfb-exe>
  [--remote <ssh-port> [linux-target]]`. Run by: `check-net-harness-selftest.sh`
  (CI `net-harness`); `--remote` by hand.
- **check-icmp-permission.sh** — `net::ping` in an ICMP-denied environment
  (`unshare -Urn` on Linux, `sandbox-exec` on macOS) must raise rather than return a
  status; a missing denial mechanism is a SKIP, never a pass.
  Usage: `bash scripts/check-icmp-permission.sh <mfb-exe>`.
  Run by: `check-net-harness-selftest.sh` (CI `net-harness`).
- **gen-test-tls-identity.sh** — Generates a throwaway test CA and a `127.0.0.1` server
  identity (certificate, PKCS#1 and PKCS#8 keys, chain) with openssl.
  Usage: `bash scripts/gen-test-tls-identity.sh <outdir>`. Run by: `check-tls-loopback.sh`.
- **net_blackhole_server.py** — A TCP server with a saturated accept backlog, so new
  connects get no SYN-ACK and must hit their deadline; prints its port. Usage:
  `python3 scripts/net_blackhole_server.py`. Run by: `check-tcp-connect-timeout.sh`.
- **net_udp_echo_server.py** — A UDP echo peer that returns each datagram to its
  sender and exits after echoing `QUIT`. Usage: `python3 scripts/net_udp_echo_server.py`.
  Run by: `check-udp-echo.sh`.

## Man docs

- **man-run-examples.sh** — Compiles (and with `--run` runs, or with `--test` runs
  `mfb test` on) every Examples block lifted from one package's rendered `mfb man`
  pages. Usage: `bash scripts/man-run-examples.sh <pkg> [--run|--test] [fn...]`.
  Run by: `man-examples-gate.sh`; by hand while writing a page.
- **man-census.sh** — Measures `mfb man` prose coverage from rendered output: per-package
  fill, per-function stragglers, and the memory-vocabulary scope check.
  Usage: `bash scripts/man-census.sh [--fill|--functions|--memory-scope|--banned-list] [pkg...]`.
  Run by: by hand.
- **man-examples-not-run.txt** — The examples that build but cannot run under the gate,
  one `pkg::fn#N  reason` per line; an entry for an example that no longer exists fails
  the gate. Run by: data for `man-examples-gate.sh`.
- **man-examples-stdin.txt** — The input piped to each example's standard input during
  the gate's runs. Run by: data for `man-examples-gate.sh`.

## Coverage

- **coverage.sh** — Runs the instrumented workspace test suite once via cargo-llvm-cov and
  writes HTML, lcov and cobertura reports, leaving the profile for `coverage-check.sh`.
  `--bins` instead runs only the `mfb` unit tests and writes
  `target/coverage/coverage.json` (the same `src/**` measurement in minutes; not valid
  for `repository/src/**`). Usage: `sh scripts/coverage.sh [--bins [cargo args]]`.
  Run by: CI `coverage` (no arguments).
- **coverage-check.sh** — The per-file gate: reads the profile `coverage.sh` left and
  lists every in-scope file below the floor, exiting non-zero if any fall short
  (`FLOOR=`, default 98). Usage: `sh scripts/coverage-check.sh`. Run by: CI `coverage`.
- **coverage-common.sh** — Shared coverage settings: `IGNORE` (denominator exclusions)
  and `PKG_FLAGS` (packages each report covers), plus the step that drops never-run
  binaries before reporting. Sourced, not executable. Run by: sourced by `coverage.sh`,
  `coverage-check.sh` and CI `coverage`'s global-floor step.
- **coverage-exceptions.txt** — Files exempt from the per-file gate because their
  uncovered remainder is reachable only from the integration harness, each with its
  reason. Run by: data for `coverage-check.sh` and `coverage-report.py`.
- **coverage-report.py** — Instruments over a llvm-cov JSON report, `src/**` only:
  `gaps`, `lines <report> <file> [--source]`, `shapes`, `dead`, `delta <base> [current]`.
  Usage: `python3 scripts/coverage-report.py <gaps|lines|shapes|dead|delta> …` (`FLOOR=`).
  Run by: by hand.
