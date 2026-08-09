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
  about which dump kinds exist.
- **bug387-gate.sh** — Byte-identity gate for the bug-387 output-preserving
  refactor: compares the app-mode `-ncode` of the three app fixtures (and
  optionally the full exe-oracle corpus) across four targets against a
  pre-fix baseline in `/tmp/bug387`. Usage: `bug387-gate.sh <mfb-exe> [app|full]`.
- **exe-oracle.sh** — Full-executable byte-identity oracle: cross-builds every
  executable-producing fixture for a target and records/compares the sha256 of
  each linked `.out`, catching changes in the entry stub and runtime-helper
  bodies that the package-object gate can't see. Usage:
  `exe-oracle.sh <mfb-exe> <target> record|compare <manifest>`.
- **ncode-determinism.sh** — Determinism harness: compiles each in-scope fixture
  N times in fresh processes (fresh HashMap seeds) for the *host* target and
  counts distinct `.ncode` hashes, flagging residual nondeterminism or a stale
  golden. Usage: `ncode-determinism.sh <mfb-binary> [N]`.
- **ncode-determinism-alltargets.sh** — Same determinism check as above, run
  across all four goldened targets (including the three `linux-*` cross targets)
  and compared against each `<target>.ncodesum` golden.
- **linux-artifact-baseline.sh** — Captures or verifies a SHA-256 manifest of
  every artifact the compiler emits for the three Linux targets (both libc
  flavors), substituting for the Linux byte-identity gate the tree otherwise
  lacks. Cross-compiles on the host; no Linux box needed. Usage:
  `linux-artifact-baseline.sh <mfb-exe> capture|verify <manifest>`.

### plan-85 remote-execution verification

Byte-identity proves the *bytes* did not change; these prove the emitted code
actually *runs* on a real box of each ISA/ABI — the correctness check for the
targets plan-85's ABI rework changes by design (SysV-x86, Win64) and for RISC-V
where the codegen (fused/`addr_of`) was reworked. Each cross-compiles a fixture,
ships the executable to its box, runs it, and diffs stdout against the golden's
execution section (`golden/build.log`). Usage: `<script> <fixture-dir>...`.

- **p85-riscv-verify.sh** — Ships the `linux-riscv64` **musl** ELF to the riscv64
  box (port 2229) and runs it.
- **p85-x86-verify.sh** — Ships the `linux-x86_64` musl ELF to the x86_64 box
  (port 2227). SysV-x86 is byte-*changing* under plan-85's aligned ABI, so
  execution — not bytes — is its correctness gate.
- **p85-win-verify.sh** — The Win64 sibling: ships the `.exe` to the Windows box
  (port 2230), runs it via `cmd`, and diffs stdout (Windows byte-identity is a
  non-goal, so execution is the only Win64 check).

## Acceptance / runtime harness

Build fixture programs, run them, and diff their behavior against goldens.

- **test-accept.sh** — The full acceptance harness: builds and runs every
  fixture under `tests/`, comparing produced artifacts and program output against
  committed goldens. Refuses to run concurrently with another copy. Usage:
  `test-accept.sh <mfb-exe> <actual-output-dir> [name-glob ...]`.
- **test-accept-selftest.sh** — Self-test for the harness's own per-fixture
  watchdog (bug-320): exercises the timeout helper directly so a program that
  blocks forever fails *that* fixture instead of wedging the whole suite.
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
  the same LLVM engine locally and in CI so numbers agree per platform.
- **coverage-check.sh** — Per-file coverage gate: reads the profile left by
  `coverage.sh` and prints every in-scope source file below the floor, exiting
  non-zero if any fall short. Floor defaults to 95; override with `FLOOR=`.
- **coverage-common.sh** — Shared coverage settings (sourced, not executable):
  the `IGNORE` denominator-exclusion regex and `PKG_FLAGS` package selection,
  used by both coverage scripts and the CI global-floor step so they can't drift.
- **coverage-exceptions.txt** — Data file listing source files exempt from the
  per-file 95% gate because their uncovered remainder (network/TTY/subprocess/GUI
  paths) is reachable only from the integration harness.

## Man pages

- **update_man.sh** — For each compiler built-in function, uses the `claude` CLI
  to review and update/create its man page from the `.ai/` templates. Optional
  first arg restricts to one package/module.
- **update_man_package.sh** — Same, for each built-in package's `package.md`
  overview page (the per-module summary, not per-function pages).
- **man_rules.sh** — Shared man-page authoring rules (sourced, not executable) so
  the two `update_man*` drivers can't carry divergent copies of the same prose.
- **check-man-examples.py** — Extracts every fenced code block under an
  `## Examples` heading in `src/docs/man/` and verifies each compiles with
  `mfb build -q`, reporting non-compiling blocks. Usage:
  `check-man-examples.py [path-glob-substring ...]`.

## Generated sources

- **gen_regex_unicode.py** — Generates `src/builtins/unicode_gencat.mfb`, the
  pinned Unicode general-category table the regex package resolves through. Output
  is tied to the interpreter's Unicode version, so regenerate only under Python
  3.14 (Unicode 16.0.0).
- **gen_vector_package.py** — Generates `src/builtins/vector_package.mfb`: the
  nine vector records and ~170 overloaded geometry/utility functions, keeping the
  per-(element-type, dimension) patterns and evaluation order uniform.
- **check-generated.sh** — Generated-artifact integrity gate: re-runs each
  generator and fails if the checked-in artifact no longer matches, so "re-run the
  generator" is always safe and drift can't land.

## Packages

- **sync-package-mfp.sh** — Rebuilds every buildable package fixture from source
  and overwrites every committed copy of its `.mfp` (consumer and golden copies),
  which otherwise go stale when the binary-representation format changes. Skips the
  deliberately-tampered security fixtures.

## Network / riscv validation helpers

- **check-net-connect-timeout.sh** — Standalone runtime check for
  `net::connectTcp`'s `timeoutMs`: starts a blackhole TCP server, then builds and
  runs a program that must fail with `ErrTimeout` well before the OS default
  connect timeout. Usage: `check-net-connect-timeout.sh <mfb-exe>`.
- **net_blackhole_server.py** — Helper for the above: a TCP server that saturates
  a tiny accept backlog so new connects get no SYN-ACK and block until their
  deadline. Prints its port and sleeps; started in the background by the check.
- **rvv-qemu-runner.sh** — A `runtime_ulp.py --runner` that runs a `linux-riscv64`
  mfb executable under qemu-user on the riscv64 box under a chosen CPU profile, so
  the same binary can be scored under emulated `v=true` (native RVV) and `v=false`
  (scalar).
- **rvv-ulp-two-profile.sh** — Drives the ULP harness to prove the one
  `linux-riscv64` binary is bit-identical and ≤1 ULP under both `v=true` and
  `v=false` for every math kernel, asserting the dual-path lowering changes no
  result bit.

## Introspection

- **list_functions.py** — Prints the built-in surface the compiler supports by
  scanning `src/builtins/`, merging the Rust `pkg::name` call surface with the
  MFBASIC `EXPORT`ed types.
