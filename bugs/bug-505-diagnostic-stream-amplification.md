# bug-505: uncapped diagnostic count + full-line echo + per-diagnostic file re-read → O(errors×filesize) CPU and multi-GB stderr

Last updated: 2026-09-03
Effort: small (<1h)
Severity: HIGH
Class: security (denial of service on hostile source)

Status: Open (found in audit-3, Surface 2 FE-03; agent-demonstrated, mechanism code-verified by the lead)

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
