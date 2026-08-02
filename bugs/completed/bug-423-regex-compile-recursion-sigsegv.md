# bug-423: `regex::compile` (parser) has no nesting-depth cap → a deeply-nested-group pattern overflows the native stack (uncatchable SIGSEGV) — bug-315 guarded only the matcher

Last updated: 2026-07-28
Effort: small (<1h)
Severity: HIGH
Class: Security / Robustness (DoS — uncatchable SIGSEGV on untrusted pattern)

Status: FIXED (6ab9dd8e7)
Regression Test: tests/rt_native_regex_parser_depth.rs — a 2000-deep nested-group
pattern (unbalanced AND balanced) compiled from untrusted runtime data yields a
clean `ErrInvalidFormat` (77050003) instead of a SIGSEGV, and a modest 40-deep
pattern still compiles and matches.

## STATUS: FIXED

Commits (branch `worktree-B-423`, merged to main):
- `6ab9dd8e7` — cap regex parser nesting depth (fix + RED→GREEN regression test)
- `43bce5a57` — regenerate regex codegen goldens (byte-identity `.ir` + 5 target
  `.ncodesum`; rt-behavior/regex `.ir`)

What was done: threaded a `depth` counter through the parser recursion cycle
(`__regex_parseAlt` → `parseConcat` → `parseParen` → `parseAlt`, plus
`parseNamedGroup`); `parseParen`/`parseNamedGroup` increment it once per group
descent, and `parseAlt` rejects `depth > __REGEX_PARSE_DEPTH_LIMIT` with the
ordinary `ErrInvalidFormat` (a catchable failure).

Deviation from the doc's plan — the doc said to *reuse* the matcher's
`__REGEX_DEPTH_LIMIT` (600). Measured and rejected: the produced executable's
native stack is exhausted around 350 nested `(` (N≤300 fails cleanly, N≥400
SIGSEGVs), *below* 600 — a depth-600 check would never fire before the crash. The
parser cycle costs three native frames per nesting level, so the same ~600-frame
budget the matcher proved safe corresponds to ~200 levels here. Added a
parser-specific `__REGEX_PARSE_DEPTH_LIMIT = 200` (verified at the boundary:
N=200 balanced compiles, N=201 fails cleanly, N=800/2000 fail cleanly — no crash
at any depth).

Verification: full `cargo test` (main merged into the worktree) — 40 binaries,
4119 passed, 0 failed; artifact-gate on byte-identity/regex PASSED (7 goldens);
original reproduction (N=800) now exits 255 with `7-705-0003 regex: pattern
nested too deeply`, previously exit 139 (SIGSEGV), on macOS-aarch64.

The regex **parser** is unbounded recursive descent on nested groups:
`__regex_compile (:1748) → __regex_parseAlt (:1724) → __regex_parseConcat (:1667) →
__regex_parseParen (:1485) → __regex_parseAlt …` — one native frame per `(` with no
depth guard. A pattern of ~400 nested `(` (≈400 bytes) overflows the native stack →
SIGSEGV during compile.

`regex::match`/`find`/`findAll`/`replace` all call `__regex_compile(pattern)` first,
so any untrusted-pattern caller triggers it — and this file's own comment states the
engine "accepts untrusted patterns AND untrusted text." A caller `TRAP` cannot catch
a SIGSEGV.

bug-315 (FIXED) added a depth limit + step budget to the **matcher**
(`__regex_matchNode`) for the ReDoS/stack-overflow class, but the parser/compile
path was out of scope and remains unguarded. The json sibling has the same gap
(bug-422); the regex *matcher* has a `__REGEX_DEPTH_LIMIT` — the *compiler* should
reuse it.

References:

- `src/builtins/regex_package.mfb:1485` (`__regex_parseParen`), `:1667`
  (`parseConcat`), `:1724` (`parseAlt`), `:1748` (`__regex_compile`) — the unbounded
  parser recursion.
- bug-315 (`bugs/completed/`) — matcher guarded, compiler not. Found during goal-07.

## Failing Reproduction

```
mfb init /tmp/regexdos
# /tmp/regexdos/src/main.mfb:
IMPORT regex
FUNC main() AS Integer
  MUT s AS String = ""
  MUT i AS Integer = 0
  WHILE i < 800
    s = s & "("
    i = i + 1
  END WHILE
  LET m = regex::match("x", s)   ' s is the PATTERN (2nd arg)
  RETURN 0
END FUNC
mfb build /tmp/regexdos && /tmp/regexdos/build/*.out ; echo $?
```

- Observed (verified 2026-07-28, `target/debug/mfb`, macOS-aarch64): N=800 → exit
  **139 (SIGSEGV)**. Agent measured the threshold at ~400 (N=200/300 → clean error
  `77050003`; N≥400 → 139).
- Expected: a bounded error (e.g. "regex nested too deeply"), no crash.

## Root Cause

`__regex_compile`'s recursive-descent parser recurses once per `(` / alternation
level with no depth counter; recursion depth = pattern nesting depth, exhausting the
native stack.

## Goal

- `__regex_compile` rejects a pattern nested beyond a fixed depth (reuse
  `__REGEX_DEPTH_LIMIT`) with a clean error — never a SIGSEGV, for any pattern.

### Non-goals (must NOT change)

- The matcher's existing depth/step budget (bug-315). Compilation of
  legitimately-nested patterns within the cap.

## Blast Radius

- `src/builtins/regex_package.mfb` — thread a depth counter through
  `__regex_parseAlt`/`parseConcat`/`parseParen` (reuse `__REGEX_DEPTH_LIMIT`).
- Every `regex::match`/`find`/`findAll`/`replace` caller with an untrusted pattern
  benefits.
