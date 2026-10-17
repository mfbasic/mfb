# Thread failure diagnostics

Status: Complete
Last updated: 2026-10-15
Effort: medium (original guess: 1h–2h)

## Goal

Print each error that escapes an MFB thread's entry function to stderr, using
`Thread:`, `Error:`, `Message:` and `Location:` labels. Handled errors and successful
threads produce no error report. Reports from concurrent failures must not interleave.

## Original state

`lower_program_entry` prints code and message at process failure.
`lower_thread_finish` stores a worker result for its parent, then destroys the
worker arena; `lower_thread_block_release` eventually destroys the stored result.
Read with `sed -n '1391,1640p' src/codegen/runtime/thread/runtime_helpers_thread.rs`
and `sed -n '760,845p' src/codegen/runtime/thread/control_block.rs`.
The current thread identity is a control-block pointer, not an OS identifier
(`rg -n CURRENT_THREAD src/codegen/runtime/thread/runtime_helpers.rs`).
`emit_thread_external_call` translates mutex operations to SRWLOCK on Windows
(`sed -n '61,95p' src/codegen/runtime/thread/runtime_helpers.rs`).

## Design

Add one allocation-free, vreg-allocated runtime formatter. Arguments are runtime
thread identity, error code, message pointer and source pointer. Print hexadecimal
thread identity (at least eight digits; main is zero), canonical hyphenated error
code, verbatim message, and the source file/line/column when present. No source
means no Location line, consistent with the earlier owner discussion. Separate
reports with a blank line.

Use the thread-table mutex around the entire report in programs that start
workers. The reporter runs outside every control-block critical section, before
worker arena destruction. It takes no allocation lock and invokes no callbacks.
No additional error history or result ownership is required: report each failed
thread as it finishes. The main thread's later error is a separate report.
The main entry and the worker finish path call the same formatter. Thread results,
waitFor and parent traps retain their existing behavior. A worker failure is
reported even when a parent subsequently handles that result.

Native startup errors have no source: clear the source register for entry return
range errors and argument allocation failure before invoking the formatter.

Reject collecting stored errors only at process exit: lexical cleanup may already
have freed their control blocks. Reject reporting at every raise: caught errors
are explicitly excluded. Generated artifacts are expected to change on every
backend; byte identity is not a correctness criterion for this behavior change.

## Phases

- [x] Add shared formatter and main-entry call in `src/codegen/error/report.rs`,
  error module wiring, builder emission, entry lowering and string constants.
- [x] Report failed workers in `lower_thread_finish` before block locking and
  result copying, preserving all saved result fields.
- [x] Update owning language/startup/threading specs and add runtime coverage for
  main failure, propagation, no origin, failed worker, handled error and success.

Acceptance: `MFB_TEST_EXE=/tmp/mfb-thread-report-compiler cargo test --release
--test rt_entry_error_location` passed all seven runtime assertions.
Commit: (this commit)

## Validation plan

Owner resumed testing on 2026-10-15 and requested all tests, fixes and a commit
containing only this work. Validation: focused runtime tests, app-mode runtime proof, then the full
acceptance and artifact gates. Before changing any golden, establish its provenance,
protected behavior and consumers, and limit updates to the requested output change.
Spec rendering and `cargo test --bin mfb spec` verify documentation integration.

## Decisions

- Owner requested aligned labels and per-thread reports.
- Owner chose MFB identity, not OS identity; hexadecimal is appropriate for pointers.
- Owner requested errors that caused threads to fail, excluding handled errors.
- Owner resumed tests on 2026-10-15 and requested fixes plus commits limited to this work.

## Corrections

Initial scope was source-location output only. Owner expanded it to labeled reports
for every failed thread. Shared reporting replaces the initial inline entry-only work.

Implementation and specification changes are complete. `cargo check --bin mfb`
passed on 2026-10-14. The seven focused runtime tests passed under the release compiler. Full-suite
validation and reviewed golden updates are complete.

## Golden and assertion provenance

The prior two-line error banner was established by `2fca3b281` (`git log -S
'Error: 7-705-0002' -- tests/rt-error/vector/normalize_zero_rt/golden/build.log`;
`git log --follow -S 'Error: 1234' -- tests/runtime/rt_native_io_runtime.rs`).
The runtime assertion protects the primary error plus secondary close-failure
audit; the error fixtures protect failure code/message/exit status. Their readers
are the runtime test and `scripts/test-accept.sh`; codegen goldens are read by
`scripts/artifact-gate.sh` (`rg -n 'golden|compare' scripts/test-accept.sh
scripts/artifact-gate.sh`). The owner explicitly replaced the two-line contract
with labeled per-thread reports, and supplied a real app execution showing both
the original worker error and the later main error. This is the authority for
changing banner expectations; cleanup failure and process status remain protected.

Before updating native artifacts, a fresh parser-hello-world ncode was compared
to its golden (`/tmp/mfb-thread-ncode-review.diff`). Differences are diagnostic
strings, replacement of the inline entry printer by the reporter call, and the
new reporter function and relocations. The full artifact gate reported 166 diffs
and no missing artifacts; full test execution precedes any golden update.


## Full-suite follow-up

The first workspace run was interrupted during `rt_scope_drop_leaks`; every
remaining target was resumed before updating goldens. Its completed unit binary passed
4,983 tests (`/tmp/mfb-thread-full-tests.log`). The full acceptance run reached
1,724 fixtures and reported 157 mismatches: 148 build logs and nine native dumps.
Reversing only the new main banner and added worker blocks reproduces every old
build log exactly (audit against `/tmp/mfb-thread-accept-mismatches.txt`).

Pre-existing failures were reproduced using an unchanged HEAD compiler built in
`/tmp/mfb-thread-report-head` with a detached worktree (`17a9b5799`):

- `rt_resource_union_member_float`: the inline-TRAP case opens the author's
  `/tmp/g933/data.txt` instead of the six-byte file created by `check`. Authored
  by `edc967e47`; the bug-1046 document and sibling cases require that fixture
  file. Only this test consumes the path (`rg -n /tmp/g933/data.txt tests`).
  Correct the path, retaining the direct producer under inline TRAP and every
  total, allocation and descriptor assertion. HEAD reproduces `-10` vs `180`.
- `rt_compress_throughput`: the reference operation became faster at `4bb55ea8c`
  (arena zero fill). Authored by `1d3a2fbbe` to distinguish bug-774's CRC/inflate
  regressions; consumers are this runtime test and code-shape assertions. A/B
  its unchanged PROGRAM with compilers at `4bb55ea8c^` and HEAD: CRC medians
  926/926 us, reference 179/62 us; inflate 2420/2027 us, tiny 203/90 us
  (`/tmp/mfb-thread-compress-{prefill,head}.log`). HEAD fails the old CRC bound;
  the parent passes. Scale only the disproved CRC/inflate ratio bounds for
  the 2.89x faster denominator, retaining separation from the old regressions
  and preserving the tiny and code-shape checks.

The worker report contains an ASLR pointer, so acceptance logs canonicalize only
complete nonzero Thread fields to stable worker identities. The self-test was
RED before `normalize_thread_ids` existed and GREEN afterward
(`bash scripts/test-accept-selftest.sh`); it preserves zero, repeated/distinct
identity, message text and location. Raw pointer formatting has runtime coverage.

The TLS clean-close test failed with macOS's bundled LibreSSL 3.3.6 because
its TLS 1.3 peer trace contained no sent close_notify. The existing test
correctly rejects that peer; running the same test with installed OpenSSL
3.6.2 first in PATH passes (`/tmp/mfb-thread-tls-openssl3.log`). No TLS test
expectation or production TLS code is changed. The resumed suite uses that
OpenSSL. The debugger test passes both alone against HEAD and in the resumed
suite; no debugger changes are warranted by the first timing failure.


## Final verification

The runtime change, tests, specs, harness normalization and reviewed golden
updates land together with this completed document (use `git log -1 --
planning/completed/plan-177-thread-failure-diagnostics.md` for the commit).

- `cargo test --release --workspace --no-fail-fast -- --skip artifact_gate_all`
  completed the compiler unit binary (4,983 passed, two ignored), then was
  interrupted during the integration run. A coverage audit of `cargo metadata
  --no-deps --format-version 1` against the successful target summaries in the
  initial run, resumed run and registration rerun found all 349 current MFB
  integration targets covered. The artifact test is driven separately below.
- `cargo test --release -p mfb_repository -p mfb_wire --no-fail-fast` passed
  (450 library, 24 binary and 54 wire tests, plus empty doc-test targets).
- `bash scripts/test-accept.sh /tmp/mfb-thread-report-compiler
  target/accept-thread-final` passed all 1,724 fixtures.
- `bash scripts/artifact-gate.sh /tmp/mfb-thread-report-compiler all` checked
  2,352 goldens with zero diffs. The changed native golden paths are exactly
  the first gate's 166 differences. Structural JSON comparison shows only entry
  reporting, the new formatter and its five diagnostic strings changed. The
  two MIR dumps also renumber vregs in unchanged helper bodies: bijective
  renaming makes those bodies identical (`Vregs::next` delegates to the shared
  minter in `src/codegen/engine/util/vreg_frame.rs`, read with
  `sed -n '640,652p' src/codegen/engine/util/vreg_frame.rs`).
- `cargo check --all-targets` passed without warnings; `cargo fmt --all --check`
  and `git diff --check` passed.
- `scripts/spec-census.sh --citations language memory threading`, `--links`
  and `--render` passed, with no unresolved references or rendered leaks.
- `bash scripts/test-macapp.sh /tmp/mfb-thread-report-compiler` passed the
  non-GUI cases; optional GUI cases were not enabled. The owner also confirmed
  the actual audio-worker/main error blocks from the app before testing resumed.
- Generated worker/main failure programs executed on macOS, Windows x86-64
  (192.168.1.206), Linux AArch64 (2226) and Linux RISC-V (2229), printing complete
  reports with status 255. Cross-target builds cover Linux x86-64 too; its
  configured runtime boxes were unreachable. Remote probe files were removed.

Test maintenance discovered by this validation is itemized separately:
`1870cb274` fixes the hard-coded inline-TRAP fixture path; `fa8250a84` corrects
the disproved ratio bounds after the allocator speedup; `c8973771d` also escapes
the shared resource prelude's host path. For the last correction, an executable
probe generated from the exact prelude function opens a real filename containing
a backslash: unchanged HEAD exits 255 with ErrFileNotFound, while the shared
`mfb_path_literal` escaping exits 0 and prints 6. All six resource cases pass
afterward (`cargo test --release --test rt_resource_union_member_float`). This
prelude was authored by `edc967e47` and is consumed only by this test module;
the file-open, resource and allocation assertions remain intact.
