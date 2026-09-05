# bug-502: `mfb fmt` has no nesting cap and rewrites the file non-atomically → quadratic memory blowup and source destruction on hostile input

Last updated: 2026-09-03
Effort: small (<1h)
Severity: HIGH
Class: security (data destruction + denial of service)

Status: FIXED (28d80fd4c; see STATUS block at the end)

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

## STATUS: FIXED

Fixed in `28d80fd4c` (bug-502: cap mfb fmt nesting depth and replace the source
atomically), landed on `main` via the `worktree-B-502` merge.

**Mechanism confirmed.** On the pre-fix release binary a 40 034-byte file of
2 000 nested `IF TRUE THEN` blocks came back as 8 048 036 bytes ("Formatted",
exit 0), while `mfb build` on the same file stops at line 259 with
`MFB_PARSE_BLOCK_TOO_DEEP` — the formatter accepted, and inflated, what the
compiler refuses. `format_source` had no depth ceiling (`indent_str(level, width)`
per line) and `format_path` wrote the result with `fs::write`.

**Fix.** `fmt::format_source` returns `Result<String, FormatError>` and refuses
past `MAX_NESTING_DEPTH` (1024) open frames — in the block stack and in
`format_link_block`'s own `depth` alike — at the line whose opener crossed it.
The cap is four times the parser's `MAX_STMT_DEPTH` because the formatter counts
frames the parser does not (`MATCH`+`CASE`, `TESTING`/`TGROUP`/`TCASE`, the LINK
DSL), so no program that builds is refused; with `--indent` already bounded at
256 (bug-220) a line carries at most 1024 × 256 bytes of indentation, so output
is linear in input. The CLI reports the refusal with the parser's own
`MFB_PARSE_BLOCK_TOO_DEEP` (no new rule code minted) and leaves the file
untouched in both modes. `replace_contents` writes the full text to a sibling
`.<name>.mfb-fmt-<pid>.tmp`, syncs, copies the permission bits, and renames it
over the canonicalized original; any failure removes the temporary. The original
is opened for writing (no truncation) first, so a read-only file still fails
with the same permission error it always did.

**Tests.** `src/fmt.rs` (`nesting_at_the_cap_still_formats`,
`nesting_past_the_cap_is_refused_at_the_crossing_line`,
`link_block_nesting_past_the_cap_is_refused`), `src/cli/fmt.rs`
(`format_path_refuses_a_too_deep_file_and_leaves_it_intact`,
`replace_contents_leaves_no_temporary_and_keeps_permissions`,
`replace_contents_refuses_a_read_only_file_untouched`),
`tests/cli_fmt_nesting_depth.rs` (real binary: 2 000-deep tower → exit 1 +
located diagnostic + byte-identical file + no temporary; `--check` likewise; an
ordinary file formats identically; a tower exactly at the cap still formats).
RED on the pre-fix binary (2 of 4 CLI tests: exit 0, file rewritten), GREEN
after.

**Semantics preserved.** `mfb fmt` output is byte-identical between the pre-fix
and fixed binaries across all 1 445 `.mfb` files under `examples/` and `tests/`
(36 of which fmt actually rewrites). Man page (`src/docs/man/tooling/fmt.md`) and
spec (`src/docs/spec/tooling/05_fmt.md`) synced.

**Deviation from the doc.** The "effective indent width" cap the Best fix asks
for already existed (`MAX_INDENT` = 256, bug-220); this fix bounds the other
factor, depth, and documents the product bound.
