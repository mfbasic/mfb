//! `datetime::formatDuration` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/formatDuration.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Render a `Duration` as a human-readable `[-][Nd ]HH:MM:SS.mmm` span."#;
const DESC: &str = r#"`datetime::formatDuration` renders the signed span `d` as a fixed-shape string of
the form `[-][Nd ]HH:MM:SS.mmm`. The hour, minute, and second fields are always
two digits and the millisecond field always three; the day field and its trailing
space appear only when the span is at least one whole day. A span of one day, two
hours, three minutes, four-and-a-half seconds renders as `1d 02:03:04.500`, while
ninety seconds renders as `00:01:30.000` and a zero span as `00:00:00.000`.


The span is reduced to whole milliseconds before formatting: the value used is
`d.seconds * 1000 + d.nanos / 1000000`, so any sub-millisecond remainder in the
`nanos` field is truncated and does not appear in the output. A negative span is
rendered as its absolute magnitude prefixed with a single leading minus sign; the
hour, minute, second, and millisecond fields are taken from the absolute value and
never carry their own sign. The day count is the full number of whole days and is
not wrapped, so a multi-day span shows a multi-digit day field; the hour field is
the remaining whole hours modulo 24, the minute field the remaining minutes modulo
60, and the second field the remaining seconds modulo 60.


`datetime::formatDuration` is pure: the same `Duration` always yields the same
string, and it has no side effects. Because the reduction to milliseconds is
ordinary signed `Integer` arithmetic, a span whose second count is large enough
that multiplying by 1000 (or negating the reduced total) leaves the signed
`Integer` range traps rather than formatting."#;
const EX: &str = r#"Render a sub-day span:

```
IMPORT datetime
IMPORT io

SUB main()
  LET d AS Duration = datetime::duration(90)
  io::print(datetime::formatDuration(d))        ' 00:01:30.000
END SUB
```

Render a span that includes whole days:

```
IMPORT datetime
IMPORT io

SUB main()
  LET d AS Duration = datetime::duration(1, 2, 3, 4, 500_000_000)
  io::print(datetime::formatDuration(d))        ' 1d 02:03:04.500
END SUB
```

A negative span is prefixed with a minus sign:

```
IMPORT datetime
IMPORT io

SUB main()
  LET d AS Duration = datetime::duration(-30)
  io::print(datetime::formatDuration(d))        ' -00:00:30.000
END SUB
```"#;

pub(crate) const FORMAT_DURATION: BuiltinFunction = BuiltinFunction::custom(
    "datetime.formatDuration",
    "formatDuration",
    INTRO,
    DESC,
    &[],
    &[super::ov(&[super::req("d", "Duration")], "String")],
)
.with_example(EX);
