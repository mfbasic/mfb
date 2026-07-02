# Security regression tests

Test cases for the security findings in `planning/audit-unicode.md` (the `strings::` /
Unicode runtime audit). One directory per finding: `unicode-0N-<slug>`.

## Status: wired into the harness; fixes landed

`scripts/test-accept.sh` discovers these tests (it iterates `tests/*` and
`tests/security/*`; the test name is the path relative to `tests/`, so filter with
e.g. `'security/*'` or just the directory basename). `scripts/sync-goldens.sh`
covers them the same way.

Each `golden/<name>.run` was authored by hand from the expected behavior and asserts
the **post-fix** contract (the harness embeds the run output in `build.log`; the
`.run` file is the run trigger and the human-readable contract). The tests were
written test-first: against the pre-fix compiler, unicode-01/02 segfaulted (heap
overflow) and unicode-04 died with SIGBUS (out-of-bounds read); all nine now produce
their golden output.

## Coverage

| Directory | Finding | Severity | Asserts (post-fix) |
| --- | --- | --- | --- |
| `unicode-01-repeat-overflow` | #1 | CRIT | `repeat(v, 2^59)` on 32-byte `v` raises catchable `77050002`; no heap overflow |
| `unicode-02-pad-overflow` | #2 | CRIT | `padLeft(v, 2^62+1, "😀")` raises catchable `77050002`; no heap overflow |
| `unicode-03-ingress-utf8-invariant` | #3 | HIGH (latent) | `toString` rejects overlong/surrogate/>U+10FFFF/truncated/bad-continuation byte lists with `77020004` — the ingress invariant that keeps the decoder OOB unreachable |
| `unicode-04-count-underread` | #4 | MED | `count("ab","abcdef")` returns `0`; no OOB read / runaway loop |
| `unicode-05-find-fold-parity` | #5 (retracted) | — | constant-arg `find`/`mid` out-of-range compile fine and raise the catchable runtime `77050001` (single evaluation path, no build error) |
| `unicode-06-find-negative-start` | #6 | LO | `find(v, needle, -1)` raises catchable `77050001` |
| `unicode-07-padchar-scalar` | #7 | LO | empty and multi-scalar `padChar` rejected with `77050002`; single scalar accepted |
| `unicode-08-tobytes-roundtrip` | #8 | LO | `toBytes` of a multi-byte string round-trips byte-for-byte (derived-length sizing correct) |
| `unicode-09-expanding-two-pass` | #9 | LO | expanding `upper`/`lower`/`normalizeNfc`/`graphemes` produce correct output+length (a count/write divergence would break these) |

Findings #1, #2, #4, and #6 are expressed with an inline `TRAP` on a FUNC wrapper
because `strings::*` members are inline-lowered and cannot take an inline TRAP directly
(`TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`); the wrapper FUNC owns a callable symbol, so the
trap compiles.
