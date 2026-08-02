# bug-411: `WIN_FILETIME_MAX_UNIX_SEC` is ~1000× too large (contradicts its own `(i64::MAX - epoch)/1e7` formula), defeating the FILETIME overflow guard

Last updated: 2026-07-28
Effort: small (<1h)
Severity: LOW
Class: Correctness (wrong constant defeats an overflow guard)

STATUS: FIXED (a9ef157f0)

Corrected `WIN_FILETIME_MAX_UNIX_SEC` from `"910692730085477"` to
`"910692730085"` (`src/target/shared/code/datetime.rs:72`) and the stale guard
comment at line 303 to match. Pinned with a unit test
(`datetime::tests::win_filetime_max_unix_sec_matches_no_wrap_formula`) asserting
the constant equals the documented `(i64::MAX - epoch)/1e7` formula AND the exact
no-wrap boundary (`bound*1e7 + epoch` fits i64; `(bound+1)*1e7 + epoch`
overflows). RED against the old constant (`left: 910692730085477`,
`right: 910692730085`), GREEN after. Full `cargo test` green across all targets
(main bin 3738, repository lib 313, repo main 21, all integration suites, 0
failed). No golden references the constant (memory: no Windows `.ncodesum`
golden; grep for `910692730085477` tree-wide = 0 hits).

Deviation from the doc's suggested regression test: the doc phrased the formula
as `(i64::MAX - WIN_UNIX_EPOCH_TO_1601_SEC*1e7)/1e7`; the test uses the
equivalent `WIN_FILETIME_UNIX_EPOCH_100NS` (= `WIN_UNIX_EPOCH_TO_1601_SEC*1e7`)
directly, matching the code's own doc-comment. Windows-only + year-~58000
runtime repro is not cheaply reproducible on the macOS host, per the doc;
verified via arithmetic + the const/boundary unit test.

Status: FIXED
Regression Test: tests/ — a Windows datetime unit/const test asserting
`WIN_FILETIME_MAX_UNIX_SEC == (i64::MAX - WIN_UNIX_EPOCH_TO_1601_SEC*1e7)/1e7`.
Landed as `datetime::tests::win_filetime_max_unix_sec_matches_no_wrap_formula`.

`WIN_FILETIME_MAX_UNIX_SEC` (`src/target/shared/code/datetime.rs:72`) is the
Windows `localOffset`/`offsetAt`/`toLocal` HIGH bound on `epochSeconds`, guarding
`epochSeconds*1e7 + epoch` from wrapping into a valid-looking FILETIME that
`FileTimeToSystemTime` accepts (silently returning a garbage offset instead of
`ErrInvalidArgument`). Its own doc-comment gives the formula `(i64::MAX - epoch) /
1e7`:

```
(9223372036854775807 - 116444736000000000) / 10000000 = 910692730085
```

but the constant is `"910692730085477"` — the correct value with **three extra
digits** (~1000× too large). Because the bound is 1000× too permissive,
`epochSeconds` in the wrapping sub-range (roughly (1.83e12, 9.11e14], i.e. years
~58000+ AD) passes the `branch_gt` guard, the `epochSeconds*1e7` multiply wraps
u64, and a garbage FILETIME can survive the downstream NULL checks →
`datetime::localOffset`/`offsetAt`/`toLocal` returns a wrong UTC offset instead of
trapping. (For `epochSeconds` in (910692730085, ~1.83e12] the value merely exceeds
i64 without wrapping u64, so the `FileTimeToSystemTime` NULL path still catches it —
the silent-wrong window is only the wrapping sub-range.)

Windows-only and requires a year-~58000 instant, so latent/LOW — but a definite
off-by-1000 constant that contradicts its own documented formula and re-opens the
exact failure mode the guard was written to close. The Windows datetime path was
added by plan-66 after goal-01 reviewed `datetime.rs` (then 122 loc), so this
constant was never audited.

References:

- `src/target/shared/code/datetime.rs:70-72` (the doc formula and the wrong
  constant). Found during goal-07.

## Failing Reproduction

Windows-only + a year-~58000 date; not cheaply reproducible on the macOS host.
Arithmetic proof: `(9223372036854775807 - 116444736000000000)//10000000 =
910692730085`, whereas the constant is `910692730085477`; `910692730085477*1e7 +
116444736000000000 ≈ 9.1e21`, which overflows u64 (max ~1.84e19).

- Observed: for `epochSeconds` in the wrapping sub-range, the guard passes and a
  garbage FILETIME yields a wrong offset.
- Expected: `epochSeconds` above `910692730085` is rejected as
  `ErrInvalidArgument`.

## Root Cause

The constant string carries three extra digits versus its documented
`(i64::MAX - epoch)/1e7` value.

## Goal

- `WIN_FILETIME_MAX_UNIX_SEC == "910692730085"`, so any `epochSeconds` that would
  wrap the FILETIME computation is rejected.

### Non-goals (must NOT change)

- The guard logic and the FILETIME math; only the constant value.

## Blast Radius

- `src/target/shared/code/datetime.rs:72` — the single constant. Grep for other
  uses of `WIN_FILETIME_MAX_UNIX_SEC` (only the `branch_gt` guard) to confirm no
  dependent magic number.
