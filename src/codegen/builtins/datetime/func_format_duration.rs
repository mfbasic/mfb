//! `datetime::formatDuration` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md): the descriptor, the authored docs,
//! and the member's MFBASIC source body (`Body::mfb`).

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

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __datetime_formatDuration(d AS Duration) AS String
  MUT totalMs AS Integer = d.seconds * 1000 + d.nanos / 1000000
  MUT sign AS String = ""
  IF totalMs < 0 THEN
    sign = "-"
    totalMs = -totalMs
  END IF
  LET days AS Integer = totalMs / 86400000
  LET hh AS Integer = (totalMs / 3600000) MOD 24
  LET mm AS Integer = (totalMs / 60000) MOD 60
  LET ss AS Integer = (totalMs / 1000) MOD 60
  LET ms AS Integer = totalMs MOD 1000
  MUT out AS String = sign
  IF days > 0 THEN
    out = out & toString(days) & "d "
  END IF
  RETURN out & __datetime_pad2(hh) & ":" & __datetime_pad2(mm) & ":" & __datetime_pad2(ss) & "." & strings::padLeft(toString(ms), 3, "0")
END FUNC"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "formatDuration",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Duration"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![super::Parameter {
                name: "d",
                desc: "The duration to render.",
                aliases: &[],
                ty: super::ParameterType::named("Duration"),
                default: super::DefaultValue::None,
            }],
            return_type: super::ParameterType::String,
            errors: vec![],
            body: super::Body::mfb(BODY, "__datetime_formatDuration"),
        }],
    });
}
