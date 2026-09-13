# bug-611: `datetime` arithmetic members declare no errors, and which inputs raise is unmeasured

Last updated: 2026-09-13
Effort: small (<1h)
Severity: LOW
Class: Documentation (registry error declaration)

Status: Fixed
Regression Test: `datetime::tests::members_declare_the_errors_they_raise`
(`src/codegen/builtins/datetime/mod.rs`) and `tests/rt-behavior/datetime/datetime-arith-errors-rt`

## STATUS: FIXED (81bd5400f)

Landed in three commits: 81bd5400f (declarations + unit test), e59527195 (runtime
fixture), e48d30463 (prose and spec corrections).

Deviations from the doc:

- **Wider than listed.** `format` (`ErrInvalidFormat`, `ErrOverflow`) and
  `formatDuration` raise too. `dayOfYear`, `weekday`, `resolve` and `toIso` raise
  `ErrOverflow` on a directly built `datetime::DateTime` record. `fromMillis` was on the
  list but never raised, and keeps `vec![]`.
- **Three no-failure claims corrected:** the `toIso` and `resolve` pages, and the spec's
  `fromMillis` sentence.
- **No golden drift.** The Fix Design expected `.ir` goldens to move. None did.

Verification (macOS aarch64, worktree `target/release/mfb`):

- `cargo test --no-fail-fast` → 172 `test result: ok.` lines, 5540 passed, 0 failed,
  6 ignored.
- `bash scripts/test-accept.sh target/release/mfb /tmp/b611-accept-actual` →
  `acceptance tests passed (1464 test(s) ran)`.
- `man-census.sh --memory-scope datetime` → 0 unclassified; `--scope datetime` → 0.
- `spec-census.sh --links` → 0 unresolved; the new citations are not among the
  `--citations` misses, and the 61/2 misses there are plan-125-N's recorded baseline.
- `cargo fmt --all -- --check` clean in the root and `repository/` workspaces.
- After merging `main` (bug-609, `man.rs`/`man-census.sh` only, no file overlap):
  `test-accept.sh` → `acceptance tests passed (1464 test(s) ran)`; the `datetime` man
  pages render the same Errors tables; both man-census scopes 0; fmt clean. The
  post-merge `cargo test` was stopped before completion at the owner's request, so the
  5540-passed figure above is from before the merge.
- Linux and Windows were not run. The change is descriptor metadata and doc text only,
  and emits no code. The new fixture's program exercises the existing datetime bodies,
  and the per-backend corpus runs it in CI.

Found while fixing bug-520 (S8). Every `datetime` descriptor declared `errors: vec![]`.
bug-520 probed and filled in the members that raise on bad arguments or out-of-range
host instants: `date`, `time`, `toIso(dt, digits)`, `parse`, `parseIso`, `fixedOffset`
(both), `localOffset`, `offsetAt`, `toLocal`, `inZone` and `civil`. The rest still
list nothing, and nobody has measured which of them can raise.

## Failing Reproduction

```
grep -l 'errors: vec!\[\]' src/codegen/builtins/datetime/func_*.rs
```

The members most likely to raise are the ones doing checked `Integer` arithmetic on
caller values, which fails `ErrOverflow` (`77050010`):

- `add`, `subtract`, `plus`, `minus`, `negate`, `between`
- `addDays`, `addMonths`, `startOfDay` (via `civil`: `ErrOverflow`, and for a Local
  zone `ErrInvalidArgument`)
- `toMillis`, `toNanos`, `fromMillis`, `instant`, `duration`

Each is a **guess** until probed. For example, `civil` with a UTC year of `3·10^14`
raises 77050010 (measured in bug-520), so `addDays` from such a value very likely
does too.

## Fix Design

For each member: read the body, probe the boundary inputs (Integer max/min, huge
day/month counts, a Local zone far from the epoch), and list exactly the codes seen
in its descriptor `errors`, the way bug-520's S8 table did. Descriptor `errors` feed
the `.ir` goldens; regenerate and inspect.

## Probe findings (2026-09-13)

Measured on macOS aarch64 with `target/release/mfb` at `9423d8d22`. Every call was
wrapped in `TRAP` and printed its code. It ran under the host zone and again under
`TZ=America/New_York`, and the output was identical (both exit 0). The raises are
pinned by `tests/rt-behavior/datetime/datetime-arith-errors-rt`, and the declarations by
`datetime::tests::members_declare_the_errors_they_raise`.

| Member (overload) | Raising input | Code |
| --- | --- | --- |
| `add` | `add(instant(max), duration(1))` | `ErrOverflow` |
| `subtract` | `subtract(instant(min), duration(1))` | `ErrOverflow` |
| `plus` / `minus` | `duration(max)` + 1 / `duration(min)` − 1 | `ErrOverflow` |
| `negate` | `negate(duration(min))` | `ErrOverflow` |
| `between` | `between(instant(min), instant(max))` | `ErrOverflow` |
| `instant` 2–5, `duration` 2–5 | `instant(max, 1000000000)`, `instant(max, 0, 0)`, … | `ErrOverflow` |
| `toMillis` / `toNanos` | `instant(max)` / `instant(10^10)` | `ErrOverflow` |
| `addDays` | `days = max` / Local zone, `days = 10^12` | `ErrOverflow` / `ErrInvalidArgument` |
| `addMonths` | `months = max` / Local zone, `months = 10^11` | `ErrOverflow` / `ErrInvalidArgument` |
| `startOfDay` | `toUtc(instant(max))` (the `±86400` probe in `resolveLocal`) / Local zone, year `3·10^9` | `ErrOverflow` / `ErrInvalidArgument` |
| `withZone` | `toUtc(instant(max))` into `fixedOffset(3600)` / into `local()` from `instant(10^17)` | `ErrOverflow` / `ErrInvalidArgument` |
| `format` | unknown token `"q"` / a record with offset `min` and pattern `ZZ` | `ErrInvalidFormat` / `ErrOverflow` |
| `formatDuration` | `duration(max)` (`seconds * 1000`) | `ErrOverflow` |
| `toIso` (both) | a record with offset `min` (`-s` in the offset label) | `ErrOverflow` |
| `dayOfYear`, `weekday`, `resolve` | a record with year `max` | `ErrOverflow` |

- **Only a directly built record raises in the last three rows and `format`/`toIso`.**
  `datetime::DateTime[datetime::Date[max, 1, 1], …, offset]` compiles. Values that come
  from the constructors did not raise there: `dayOfYear`, `weekday`, `resolve` and
  `toIso` of `toUtc(instant(max))` and `toUtc(instant(min))` all returned.
- **No raise at the extremes:** `fromMillis` (min, max), `toUtc` (min, max), `compare`,
  `equals`, `isBefore`, `isAfter`, `isLeapYear(min)`, `daysInMonth(min, 2)` and
  `daysInMonth(2026, 13)`, `now`, `nowNanos`, `monotonic`, `monotonicNanos`, `local`,
  `utc`. They keep `errors: vec![]`.
- **`format` was outside the doc's list.** `__datetime_formatToken` ends in
  `FAIL error(77050003, …)` for an unrecognised letter, so the page claimed no error
  for a documented failure.

## Phases

### Phase 1 — probe and declare

- [x] Probe each member above and record its raising inputs and codes here.
      Recorded under Probe findings. Beyond the list above: `format` and
      `formatDuration` raise too, and `dayOfYear`/`weekday`/`resolve`/`toIso` raise
      on a directly built record. Two pages claimed no failure outright (`toIso`: "emits
      a result for every `datetime::DateTime`"; `resolve`: "The computation is total"),
      and `mfb spec stdlib datetime` said `fromMillis` can raise `ErrOverflow`, which
      the probe disproves. All three are corrected.
      The Fix Design expected the new `errors` lists to drift `.ir` goldens. They
      did not: `bash scripts/test-accept.sh target/release/mfb /tmp/b611-accept-actual`
      → `acceptance tests passed (1464 test(s) ran)`, no `mismatch` or `unexpected` lines.
      The only new goldens belong to the new fixture.
- [x] Fill in `errors` for every member that raises; leave `vec![]` only where the probe
      shows none.

Acceptance: `mfb man datetime <member>` shows an Errors table for every member that
raises, and nothing else changes.
Measured: `mfb man datetime add|instant|toIso|format|resolve` render the new tables
(`instant`: "Overload 1 raises no errors."); `mfb man datetime fromMillis` renders none.
`datetime::tests::members_declare_the_errors_they_raise` was RED at `9423d8d22`
(`datetime.add` declared `[[]]`) and is GREEN.
Commit: 81bd5400f (declarations + unit test), e59527195 (`datetime-arith-errors-rt`),
e48d30463 (`toIso`/`resolve` prose, spec `fromMillis` claim)
