//! `datetime::nowNanos` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). datetime members are
//! `Implementation::Custom` (arity/type resolved by `DatetimeResolver`); the
//! source bodies live in the shared `package.mfb`. This file owns the
//! descriptor + docs migrated from `src/docs/man/builtins/datetime/nowNanos.md`.

const INTRO: &str = r#"The current wall-clock reading as nanoseconds since the Unix epoch."#;
const DESC: &str = r#"`datetime::nowNanos` is the low-level OS-seam intrinsic behind `datetime::now`.
It reads the host's real-time clock (`clock_gettime(CLOCK_REALTIME)` on the
supported platforms) and returns a single `Integer` giving nanoseconds elapsed
since `1970-01-01T00:00:00Z` on the UTC timeline (the Unix epoch, without leap
seconds). The reading is formed as `tv_sec * 1_000_000_000 + tv_nsec` from the
libc `timespec`, folding whole seconds and the sub-second remainder into one
count rather than the `seconds`/`nanos` pair an `Instant` carries.


Most programs should call `datetime::now`, which splits this same reading into a
structured `Instant` whose `seconds` and `nanos` fields can be projected through
a zone with `datetime::toUtc`, `datetime::toLocal`, or `datetime::inZone`. Reach
for `nowNanos` directly only when a raw integer count of nanoseconds is what is
wanted — to stamp a log line, derive a millisecond count, or difference two
readings without building `Instant` values.

`nowNanos` reports nanoseconds since the epoch and is bounded by the range of an
`Integer`: a 64-bit signed nanosecond count overflows in the year 2262. This is
a limit on the intrinsic, not on the `Instant` type, whose `seconds` field spans
the full `Integer` range. On any correctly configured host the reading is
non-negative.

`nowNanos` is **not pure**: two calls may return different values, and a
program's output depends on the host clock. For reproducible logic, capture one
reading and derive everything else from it. It takes no arguments, reads host
clock state only, and has no side effects."#;
const EX: &str = r#"Read the current time as a raw nanosecond count:

```
IMPORT datetime

SUB main()
  LET ns AS Integer = datetime::nowNanos()
END SUB
```

Derive a millisecond timestamp from the nanosecond reading:

```
IMPORT datetime

SUB main()
  LET ns AS Integer = datetime::nowNanos()
  LET ms AS Integer = ns / 1000000
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    super::intrinsic(pkg, "nowNanos", INTRO, DESC, EX, vec![]);
}
