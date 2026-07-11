# bug-24: `mfb audit` text output prints untrusted package/dependency names raw → ANSI/newline terminal spoofing

Last updated: 2026-07-08
Effort: small (<1h)

`src/audit/text.rs::render` writes manifest- and `.mfp`-derived strings
(dependency names, package names, and other externally-sourced fields) verbatim
with `writeln!`, no sanitization (`:34-49` dependencies, `:56-67` packages, and the
other sections). Those strings originate from **untrusted** package headers /
manifests (an installed `packages/x.mfp`, a typosquatted dependency). A crafted
name containing ANSI escape sequences or an embedded newline — e.g.
`evil\n  error AUDIT-DEP-MISSING spoofed` — lets a malicious package forge
additional report lines or hide/recolor output in the operator's terminal when they
run `mfb audit` in the default text format.

The single correct behavior a fix produces: externally-sourced strings are
sanitized (control/ESC characters escaped or stripped, embedded newlines rejected)
before being written to the terminal, so a package cannot inject report content.

Severity LOW: requires an already-installed malicious/typosquatted package, and the
audit output is informational; but audit is exactly the tool an operator uses to
*decide whether to trust* a package, so spoofing its output is meaningful.

References:

- `src/audit/text.rs:34-49` (dependency rendering), `:56-67` (package rendering),
  and sibling sections writing untrusted fields with `writeln!`.
- Contrast (safe): `src/audit/json.rs:72-86` (`write_string` escapes quotes,
  backslash, and control chars), so the JSON format is not spoofable.
- Related class: audit-1 REPO-06/07 (untrusted owner/hash interpolation).
- Found during goal-01 review of `src/audit/**`.

## Failing Reproduction

Install a package whose `.mfp` header name is
`legit\u{001b}[2K\rmalicious` (or contains `\n`), then run `mfb audit`
(text format).

- Observed: the crafted escape/newline is emitted to the terminal verbatim,
  letting the package erase/overwrite lines or inject a fake finding row.
- Expected: the name is shown with control/ESC chars escaped (e.g. rendered as
  `\x1b`), on a single line.

Contrast: `mfb audit --format json` (or whatever the flag is) escapes all values
via `write_string` and is not spoofable.

## Root Cause

`text.rs` treats manifest/`.mfp`-derived strings as display-safe and writes them
directly. No escaping layer exists for the human text renderer, unlike the JSON
renderer.

## Goal

- No externally-sourced string written by `text.rs` can emit raw control/ESC
  characters or embedded newlines to the terminal.

### Non-goals (must NOT change)

- The JSON renderer (already safe).
- The layout of legitimate (well-formed) names.

## Blast Radius

- Every `text.rs` write of a manifest/`.mfp`-derived field (dependency name,
  package name, path, message). A single shared `sanitize_for_terminal` helper
  applied at those write sites fixes them together.

## Fix Design

Add a `sanitize_for_terminal(&str) -> String` that escapes C0/C1 control chars and
ESC (and either escapes or rejects `\n`), and route every externally-sourced field
through it in `text.rs`. Fields the compiler fully controls (finding codes,
section headers) do not need it.

## Phases

### Phase 1 — failing test + audit

- [ ] Add a text-renderer test feeding a name with ESC/newline; assert the output
      contains no raw ESC and no injected line. Confirm it fails today.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Sanitize externally-sourced fields at the `text.rs` write sites.

### Phase 3 — validation

- [ ] `scripts/test-accept.sh`; well-formed-name audit goldens byte-identical.

## Validation Plan

- Regression test(s): the ESC/newline sanitization test.
- Full suite: `scripts/test-accept.sh`.

## Summary

The audit text renderer trusts untrusted package strings; a shared terminal
sanitizer at the write sites closes the spoofing vector without touching the safe
JSON path.
