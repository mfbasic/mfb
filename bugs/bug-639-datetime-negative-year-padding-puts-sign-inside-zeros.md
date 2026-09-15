# bug-639: `datetime::format` `yyyy` and `datetime::toIso` zero-pad a negative year with the sign inside the zeros (`00-1`)

Last updated: 2026-09-15
Effort: small
Severity: LOW
Class: Correctness (malformed output)

Status: Open
Regression Test: none yet — see Phase 1

`datetime::date` accepts negative years (proleptic Gregorian), and `datetime::civil`
builds a `DateTime` from them. But the year renderer zero-pads the **text of the
signed number** to the token width instead of padding the magnitude and prefixing
the sign. A short negative year comes out with the minus sign buried among the pad
zeros:

```
year -1:    format "yyyy" = 00-1    toIso = 00-1-01-01T00:00:00Z
year -44:   format "yyyy" = 0-44    toIso = 0-44-01-01T00:00:00Z
year -2026: format "yyyy" = -2026   toIso = -2026-01-01T00:00:00Z   (no padding needed)
year 0:     format "yyyy" = 0000    toIso = 0000-01-01T00:00:00Z    (correct)
year 7:     format "yyyy" = 0007    toIso = 0007-01-01T00:00:00Z    (correct)
```

`00-1-01-01T00:00:00Z` is not a valid timestamp in any convention, and it cannot
be parsed back: `datetime::parseIso` raises `datetime: expected date/time separator`.

**The single correct behavior a fix produces:** a negative year renders as a minus
sign followed by the zero-padded magnitude: `-0001` and `-0044` for `yyyy`, and
`-0001-01-01T00:00:00Z` for `toIso`. Non-negative years render exactly as today.

Found by plan-125-C Phase 3 while probing the `toIso` page's readback claim
(`/tmp/p125-ex/dttoiso`, then `/tmp/p125-ex/dtnegyear`). **Filed, not fixed**, by
user instruction during a documentation-only plan ("file all bugs, make no fixes").

## Reproduction

```basic
IMPORT io
IMPORT datetime

SUB main()
  LET dt = datetime::civil(datetime::date(-1, 1, 1), datetime::time(0, 0), datetime::utc())
  io::print(datetime::format(dt, "yyyy"))
  io::print(datetime::toIso(dt, 0))
END SUB
```

Observed (macos-aarch64, `worktree-P-125` at `a659057b9`, `/tmp/p125-ex/dtnegyear`):
`00-1` and `00-1-01-01T00:00:00Z`.
Expected: `-0001` and `-0001-01-01T00:00:00Z`.

## Root cause

Hypothesis, to confirm in Phase 1. The year path pads `toString(year)` on the left
to the run length, probably via `helper_pad_n.rs:__datetime_padN` (and whatever
`func_to_iso.rs:BODY_2` uses for its fixed 4-digit year). A width-4 pad of the
two-character string `-1` yields `00-1`. The padding must apply to `abs(year)`, with
the sign written first.

Also check `yy`: year -1 renders `99` today (a floor-mod of 100). Decide whether that
is the intended two-digit form for negative years; it is out of scope for the
sign-placement fix.

## Non-goals

- Changing the rendering of years 0 .. 9999 or of years with at least as many digits
  as the token width.
- Making `datetime::parseIso` or `datetime::parse` accept negative or five-digit
  years. That is a separate feature, and the `toIso` man page documents that such
  years do not read back.

## Blast-radius audit

- `datetime::format` with `yyyy` or any `y` run of length 3 or more (confirmed for
  `yyyy`); `y` alone prints `-1`, which is unaffected because no padding applies.
- `datetime::toIso`, every `digits` form (confirmed for `digits` 0).
- Any other caller of the same pad helper with a possibly-negative argument, such as
  offset or duration rendering. Audit `__datetime_padN` / `__datetime_pad2` call sites.

## Fix

Phase 1 — RED tests: `format(dt(-1), "yyyy") = "-0001"`, `format(dt(-44), "yyyy") =
"-0044"`, `toIso(dt(-1), 0) = "-0001-01-01T00:00:00Z"`; audit the pad-helper callers.
Commit:

Phase 2 — pad the magnitude and prefix the sign (GREEN); full suite, and the datetime
example goldens. Update the `format` and `toIso` man pages, which document the
as-is rendering. Commit:
