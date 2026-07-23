# Security regression tests

Test cases for the security findings in `planning/old-plans/audit-unicode.md` (the `strings::` /
Unicode runtime audit), one directory per finding `unicode-0N-<slug>`, and for the
`.mfp` package decode/verify audit `planning/old-plans/audit-1-package-decode.md`, one directory
per finding `pkg-0N-<slug>`.

## `.mfp` package decode fixtures (`pkg-0N-*`)

Each `pkg-0N-*` fixture is an executable project that imports a deliberately
malicious compiled package under its `packages/`. The `.mfp` bytes are *generated*
(never hand-typed) by the `generate.py` script that lives next to each benign base
package under `tools/security-package-sources/pkg-0N-<slug>/`, using the shared
`tools/security-package-sources/mfp_craft.py` helpers. Regenerate after a container-
format change with, e.g.:

```
python3 tools/security-package-sources/pkg-06-duplicate-section/generate.py
```

The build must fail during package verification/decode rather than produce an
executable, so each `golden/build.log` asserts a non-zero exit with the finding's
diagnostic. PKG-03 and PKG-07 only trip on the full merge (after `-ast -ir`
succeeds via the lossy external-type path), so those two carry a `.run` trigger plus
`.ast`/`.ir` goldens; the rest fail at resolve time and carry only `build.log`.

| Directory | Finding | Severity | Asserts |
| --- | --- | --- | --- |
| `pkg-01-tampered-signature` | PKG-01 | CRIT | signed package tampered post-sign → `uses … - [Tampered]`, build refuses (non-zero) |
| `pkg-03-decode-depth` | PKG-03 | HIGH | ~300 nested `Unary` in the MFBR body → decode aborts at the 256-level cap (no stack overflow) |
| `pkg-04-type-cycle` | PKG-04 | HIGH | self-referential `List` type id → `cyclic type id 10` (no infinite recursion) |
| `pkg-05-alloc-count` | PKG-05 | MED | `0xFFFFFFFF` string-pool count → clean truncation error (no gigabyte `with_capacity`) |
| `pkg-06-duplicate-section` | PKG-06 | MED | duplicate MFPC section id → `duplicate MFPC section id 1` |
| `pkg-07-need-overflow` | PKG-07 | LOW | `0xFFFFFFFF`-byte MFBR string length → overflow-safe `need` reports truncation (no wrap/panic) |

PKG-02 (semantic verification of decoded IR) is tracked in
`planning/old-plans/plan-19-ir-semantic-verification.md`; three fixtures now
exist under `tests/syntax/security/`: `pkg-02-type-confusion`,
`pkg-02b-computed-confusion`, and `pkg-02c-operator-confusion`.

## Unicode runtime fixtures (`unicode-0N-*`)

Test cases for the security findings in `planning/old-plans/audit-unicode.md` (the `strings::` /
Unicode runtime audit). One directory per finding: `unicode-0N-<slug>`.

## Status: wired into the harness; fixes landed

`scripts/test-accept.sh` discovers these tests (it walks every `project.json`
under `tests/`). After the tests reorganization the security fixtures are split
by kind: the `pkg-0N-*` package-decode diagnostics live under
`tests/syntax/security/*`, and the runtime `unicode-0N-*` / `allocator-0N-*`
fixtures live under `tests/rt-behavior/security/*`. The test name is the path
relative to `tests/`, so filter with e.g. `'*/security/*'` or just the directory
basename. `scripts/sync-goldens.sh` covers them the same way.

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

Findings #1, #2, #4, and #6 are expressed with an inline `TRAP` on a FUNC wrapper:
the `strings::repeat` member is inline-lowered, so the fallible call is placed in a
wrapper FUNC and the inline `TRAP` is attached to that call.

## Arena allocator fixtures (`allocator-0N-*`)

Runtime exercises of the bump/free-list arena — the allocator that backs every
collection and string. Each builds and runs to completion, asserting the arena
stays sound under the stress named. These live in *this* directory (unlike the
`pkg-0N-*` decode diagnostics, which are compile-time and live under
`tests/syntax/security/`).

| Directory | Asserts (runs clean) |
| --- | --- |
| `allocator-01-quick-bins` | quick-bin (small-size) allocation churn stays consistent |
| `allocator-02-size-overflow` | an oversized `repeat` request is caught as `77010001` (OOM), never an undersized allocation |
| `allocator-03-free-list-integrity` | mixed alloc/free churn keeps the free list intact |
| `allocator-04-thread-arena-init` | a worker package's per-thread arena initializes correctly (`uses allocator_churn_worker`) |
| `allocator-05-grow-carve` | a large (`1048576`-byte) allocation grows and carves the arena correctly |
