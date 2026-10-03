# bug-787: spec output panics when its pipe reader closes

Last updated: 2026-10-03
Effort: small (<1h)
Severity: LOW
Class: Correctness
Status: Fixed
Regression Test: `tests/cli/cli_spec_closed_stdout.rs` (closed reader, all four modes); `src/cli/spec.rs:tests::every_rendering_mode_propagates_write_failures` (non-broken-pipe errors)

`mfb spec architecture --all` panics and exits 101 when its stdout pipe has
no reader. The correct behavior is to terminate quietly with exit 0 when the
reader closes, matching `mfb man`. Other output failures must remain errors.

Found while rendering the architecture spec through `head`. Filed only, as
requested; no compiler or test changes are included.

## Failing Reproduction

Run from the repository root after `cargo build --release`:

```sh
python3 - <<'PY'
import os
import subprocess
for args in [('spec', 'architecture', '--all'), ('man', 'variable')]:
    reader, writer = os.pipe()
    os.close(reader)
    result = subprocess.run(
        ['./target/release/mfb', *args], stdout=writer,
        stderr=subprocess.PIPE, text=True)
    os.close(writer)
    print(args, 'exit', result.returncode)
    print(result.stderr)
PY
```

Observed on macOS AArch64: `spec` exits 101 and stderr includes:

```text
failed printing to stdout: Broken pipe (os error 32)
```

The `man variable` control exits 0 with empty stderr. The same closed-reader
spec reproduction also fails with the debug binary. Capturing the complete
spec output with `subprocess.run(..., capture_output=True)` succeeds with
exit 0. Expected: both closed-reader commands exit 0 without a panic or error
message. Closing the pipe before spawning avoids the timing dependence of
`mfb spec architecture --all | head` and observes the producer's actual status.

## Root Cause

`src/cli/spec.rs:show_spec`, `print_spec_index`, `print_spec_package`,
`print_spec_all`, and `print_spec_listing` use `println!`, which panics when
stdout writing fails. The `Result` returned by `show_spec` does not capture
those output errors.

`src/cli/man.rs:print_markdown` already uses fallible `writeln!`, suppresses
`ErrorKind::BrokenPipe`, and reports other output errors. This sibling
implementation establishes the intended CLI behavior.

Evidence: `rg -n 'println!|print!' src/cli/spec.rs src/cli/man.rs` and
`sed -n '1070,1087p' src/cli/man.rs`, plus the subprocess reproduction above.

## Goal and Non-goals

- Stop spec output quietly when stdout returns `BrokenPipe`.
- Preserve diagnostics and failure status for other output errors.
- Preserve complete output, rendering, width/color handling, and argument errors.
- Do not hide all I/O failures, catch arbitrary panics, or weaken existing tests.
- Do not change generated-program signal handling or runtime I/O semantics.

## Blast Radius

Search: `rg -n 'println!|print!' src/cli/spec.rs src/cli/man.rs`.

- `spec.rs:print_spec_all` — observed failure, in scope.
- `spec.rs:show_spec` subtopic branch, `print_spec_index`,
  `print_spec_package`, and `print_spec_listing` — same output primitive;
  in scope, add coverage for each public rendering mode.
- `man.rs:print_markdown` — unaffected; already handles broken pipes, verified
  with the control above. Preserve as a behavior reference.
- Other CLI commands — not independently reproduced; a broader CLI output
  audit is outside this spec-rendering fix.

## Fix Design

Use fallible writes for spec output and propagate output errors through the
rendering helpers. Handle `BrokenPipe` once at the spec command boundary,
ending output immediately and returning success; report other errors normally.
Keep output bytes unchanged when a reader consumes the complete stream.

## Phases

### Phase 1 — failing tests and audit

- [x] Add deterministic subprocess tests with a closed stdout reader for
      overview, subtopic, index, and `--all` output.
- [x] Assert exit 0 and empty stderr; verify the current implementation fails.
- [x] Cover a non-BrokenPipe write error through a failing writer seam.

Acceptance: closed-reader cases fail on the current implementation and the
other-error test preserves an actual error.
Commit: —

### Phase 2 — fix

- [x] Replace infallible stdout writes throughout the spec rendering path.
- [x] Stop on BrokenPipe and propagate other errors.

Acceptance: closed-reader tests pass and ordinary output is unchanged.
Commit: —

### Phase 3 — validation

- [x] Run `cargo test` and `cargo test --bin mfb spec`.
- [x] Run `scripts/spec-census.sh --render architecture` after rebuilding release.
- [x] Rerun the original subprocess reproduction against debug and release.
- [x] Compare fully captured output before and after; no golden changes expected.

Acceptance: full suite passes, complete output is unchanged, and both binaries
terminate quietly on a closed reader.
Commit: —

## Resolution (2026-10-03)

`show_spec` now resolves the request (`parse_spec_request`) before writing, and
`write_spec` renders every mode through fallible `writeln!` on a locked stdout. A
`BrokenPipe` ends output with exit 0; any other write error prints
`error: failed writing spec output: …` and exits 1, mirroring `man.rs:print_markdown`.
`mfb spec tooling cli-reference` documents the exit codes.

Proof: `MFB_TEST_EXE=<main release mfb> cargo test --test cli_spec_closed_stdout`
fails with `left: Some(101)`; against the fix it passes. The bug's original Python
reproduction prints exit 0 and empty stderr for all four spec modes. Full-suite
validation runs at the bug-staging batch gate.
