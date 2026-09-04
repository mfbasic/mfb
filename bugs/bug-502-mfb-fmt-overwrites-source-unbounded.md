# bug-502: `mfb fmt` has no nesting cap and rewrites the file non-atomically → quadratic memory blowup and source destruction on hostile input

Last updated: 2026-09-03
Effort: small (<1h)
Severity: HIGH
Class: security (data destruction + denial of service)

Status: Open (found in audit-3, Surface 2 FE-02; agent-demonstrated, mechanism code-verified by the lead)

Regression Test: a fixture running `mfb fmt` on a deeply-nested source, asserting a bounded diagnostic and that the original file is preserved on error.

## Summary

`mfb fmt` re-indents by nesting depth with no cap, so a deeply-nested (or
`--indent`-inflated) input produces quadratic-to-exponential output — 336 KB →
512 MB, 1.3 MB → 8.2 GB (17 GB RSS), `--indent 256` → 61 GB — and it writes the
result back over the user's file with a non-atomic `fs::write`. A hostile source
formatted on save (or a batch `mfb fmt` over a repo) can exhaust memory and, if it
runs out mid-write, leave the user's source truncated or destroyed.

## Mechanism

`src/fmt.rs:122-124` emits `depth * indent` spaces per line with no ceiling on
`depth` or on the product; `src/cli/fmt.rs:117-119` writes the formatted buffer
back with a plain `fs::write` (not temp-file + atomic rename), so a partial write
on OOM/interrupt corrupts the original.

## Reproduction

Agent-demonstrated: 336 KB nested source → 512 MB output; 1.3 MB → 8.2 GB / 17 GB
RSS; `--indent 256` → 61 GB. Lead code-verified the uncapped indent computation
and the non-atomic write-back.

## Best fix

Cap nesting depth (emit a diagnostic past it, matching the parser's depth cap) and
cap the effective `indent` width. Write via a sibling temp file + atomic rename
(and only replace the original after a successful full write), so a failed format
never destroys the input.

## Non-goals

Do not change formatting output for well-formed files; no language-surface change.

## Prior art

bug-220 fixed a related depth cap on the parse path; the `fmt` output/write path
was uncovered (searched `mfb fmt`, `indent`, `fs::write`, `atomic` across `bugs/`,
`audit-1-*`, `audit-2-*`).
