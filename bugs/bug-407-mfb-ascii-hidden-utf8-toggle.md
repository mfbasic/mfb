# bug-407: undocumented compile-time `MFB_ASCII` env toggle silently emits a UTF-8 validator that rejects all bytes > 127

Last updated: 2026-07-28
Effort: small (<1h)
Severity: LOW
Class: Footgun / dead-code (hidden build-time behavior switch)

Status: Open
Regression Test: tests/ — a rt-behavior test that `fs.readText` on a file
containing valid multi-byte UTF-8 (e.g. "é") succeeds — and, if the toggle is
kept, a documented/covered test for the ASCII-only mode.

`lower_validate_utf8_helper` (`src/target/shared/code/codegen_utils.rs:176`)
branches on `std::env::var("MFB_ASCII").is_ok()` at **compile time**:

```rust
if std::env::var("MFB_ASCII").is_ok() {
    // emit a helper that rejects every byte > 127
    ...
}
// else: the real UTF-8 validator
```

When `MFB_ASCII` is present in the compiler process's environment during a build,
the emitted `_mfb_rt_validate_utf8` helper rejects every byte > 127 as invalid
instead of validating real UTF-8. A program compiled under `MFB_ASCII` then traps
`ErrEncoding` on any `fs::readText`/`readLine`/`net::readText` whose input contains
legitimate multi-byte UTF-8 — silently diverging from the language's documented
UTF-8 behavior, with no source construct, flag, or diagnostic indicating the mode.

`MFB_ASCII` occurs at exactly this one line tree-wide — no documentation, no test,
no plan/bug reference. It reads as a leftover experiment hook: an undocumented,
untested, behavior-changing build-time switch is a footgun (a stray env var in a
CI/dev shell produces subtly-wrong binaries) and, if unintended, dead code.

References:

- `src/target/shared/code/codegen_utils.rs:176` (sole occurrence of `MFB_ASCII`).
  Found during goal-07.

## Failing Reproduction

Static (the ASCII branch is only taken when the env var is set at build time):

- Observed: `MFB_ASCII=1 mfb build <prog>` → `_mfb_rt_validate_utf8` rejects
  bytes > 127; the built program traps `ErrEncoding` on valid UTF-8 input.
- Expected: UTF-8 validation is unconditional (the `else` path), or the ASCII mode
  is an explicit, documented, tested build option — not a silent env toggle.

## Root Cause

A compile-time `std::env::var("MFB_ASCII")` switch selects between two emitted
validators with no documentation, test, or user-facing control.

## Goal

- The emitted UTF-8 validator does not depend on an undocumented env var: either
  remove the `MFB_ASCII` branch (dead experiment) or promote it to a documented,
  tested build flag with a clear name and spec note.

### Non-goals (must NOT change)

- The correctness of the real (`else`) UTF-8 validator path.

## Blast Radius

- `src/target/shared/code/codegen_utils.rs:176` — the only site. Decide keep-and-
  document vs. delete; there are no other `MFB_ASCII` consumers.
