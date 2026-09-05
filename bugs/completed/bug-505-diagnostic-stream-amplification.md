# bug-505: uncapped diagnostic count + full-line echo + per-diagnostic file re-read → O(errors×filesize) CPU and multi-GB stderr

Last updated: 2026-09-03
Effort: small (<1h)
Severity: HIGH
Class: security (denial of service on hostile source)

Status: FIXED (4e9699f3d; see STATUS block at the end)

Regression Test: a fixture with many errors asserting a bounded diagnostic count and no per-diagnostic file re-read.

## Summary

The diagnostic renderer emits one entry per error with no cap, echoes the full
offending source line, and **re-reads the entire source file for every
diagnostic**. A hostile source that provokes an error per line makes `mfb build` /
`mfb audit` do O(errors × filesize) work and print multi-gigabyte stderr — 240 KB
of source → 10.4 GB of stderr in ~6 s. A cheap CPU/IO/stderr exhaustion by anyone
whose source the compiler is asked to process.

## Mechanism

`src/rules/mod.rs:63-83` renders each diagnostic by re-reading the file and
echoing the located line; there is no ceiling on the number of diagnostics
collected or printed, so the cost is quadratic in file size when errors scale with
lines.

## Reproduction

Agent-demonstrated: 240 KB source → 10.4 GB stderr in 6 s. Lead code-verified the
per-diagnostic re-read and the absence of a count cap at `src/rules/mod.rs:63-83`.

## Best fix

Cap the number of diagnostics rendered (e.g. first N, then "... and M more"), and
read the source once into memory (or cache the line index) rather than re-reading
per diagnostic. Pairs naturally with bug-506's diagnostic-sanitization fix at the
same site.

## Non-goals

Do not drop diagnostics for well-formed small inputs; keep the located-line echo
for the capped set.

## Prior art

None (searched `diagnostic`, `rules`, `re-read`, `stderr` across `bugs/`,
`audit-1-*`, `audit-2-*`). Companion to FE-04 (bug filed separately or folded:
diagnostics echo raw source bytes → terminal injection, same site).

## STATUS: FIXED

Fixed in `4e9699f3d` (bug-505: cap rendered diagnostics, read the source once,
and escape the echoed line (FE-04)), landed on `main` via the `worktree-B-505`
merge.

**Mechanism confirmed.** `rules::show_diagnostic` called `fs::read_to_string`
on the source for every diagnostic and had no count ceiling. On the pre-fix
release binary a 152-line file with one `TYPE_BINDING_MISMATCH` per line rendered
all 151 diagnostics (46 KB of stderr), and a line carrying `\x1b[31m` was echoed
with the raw ESC bytes.

**Fix.** `show_diagnostic` renders at most `MAX_RENDERED_DIAGNOSTICS` (100)
located diagnostics per process and counts the rest; every CLI exit path that
can carry a diagnostic stream (`build`/`test`/`audit`/`doc`/`fmt` in
`cli::dispatch::exit_after_diagnostics`, `cli::dispatch_command_error` for
`pkg`/`repo`, and `run`'s normal return) closes it with one
`... and N more diagnostics not shown (only the first 100 are rendered)` line.
The source is read and line-indexed once (`cached_source`, keyed by path and
revalidated by a single `stat` on length + mtime). **FE-04 (same site):** the
echoed line goes through `terminal_safe` (`safe_source_line`) — every C0/C1
control and bidi/format code point is escaped as `\u{XXXX}`; a tab is kept
verbatim. Unlocated diagnostics (`show_general_diagnostic`) are neither counted
nor capped. The cap bounds only what is printed: every diagnostic is still
collected and an error still fails the build.

**Tests.** `src/rules/mod.rs` (`cached_source_reads_a_file_once_and_indexes_its_lines`,
`cached_source_notices_a_rewritten_file`, `cached_source_is_none_for_a_missing_file`,
`safe_source_line_escapes_controls_and_bidi_but_keeps_tabs`),
`tests/cli_diagnostic_stream.rs` (real binary: 8 000 errors → exactly 100
rendered + `and 7900 more`, stderr < 64 KB for a 250 KB source; 22 errors all
render with no "more" line; singular form at 101; ESC/BEL/CR/RLO escaped and
absent as raw bytes), `tests/syntax/general/diagnostic_render_cap` (golden: 120
errors → 100 rendered + `and 20 more`). RED on the pre-fix binary (3 of 4 CLI
tests; the fixture renders 120 with no "more" line), GREEN after.

**Goldens preserved.** No golden `build.log`/`test.log`/`.testrun` records more
than 22 diagnostics or contains a byte the sanitizer escapes, so none moves:
`test-accept.sh` over all 655 `tests/syntax` fixtures byte-exact,
`diag-set-diff.sh` 561 fixtures SAME. Spec synced
(`src/docs/spec/diagnostics/01_rule-codes.md`, "Diagnostic Rendering"); no man
page describes the renderer.
