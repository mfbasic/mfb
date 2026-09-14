# bug-629: a built-in package member accepts an argument of the wrong record type (`datetime::toMillis(DateTime)` compiles and returns garbage)

Last updated: 2026-09-14
Effort: medium
Severity: HIGH
Class: Correctness (type-checker soundness — silent wrong result)

Status: Open
Regression Test: none yet — see Phase 1

`datetime::toMillis` has one declaration, `toMillis(at AS datetime::Instant) AS
Integer`. Passing it a `datetime::DateTime`, `datetime::Date` or
`datetime::Duration` **compiles**, and the call then reads the argument as if it
were an `Instant`:

```
toMillis(DateTime 2026-03-07 12:00Z) = 32000           (should be a build error)
toMillis(DateTime 2026-03-08 12:00Z) = 32000           (same garbage for a different day)
toMillis(resolve(2026-03-07 12:00Z)) = 1772884800000   (correct, via the right type)
toNanos(DateTime 2026-03-07 12:00Z)  = 32000000056
```

The same wrong-type argument is **rejected** for a user-written function:

```
FUNC userMillis(at AS datetime::Instant) AS Integer ...
userMillis(dt)   ->  error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]:
                     Argument 1 for `userMillis` has type datetime.DateTime, expected datetime.Instant.
```

So the checker knows the types are different; the check is skipped only on the
built-in member's call path.

**The single correct behavior a fix produces:** a call to a built-in package member
whose argument is a record of a different type than the parameter is rejected at
build time with `TYPE_CALL_ARGUMENT_MISMATCH`, exactly as for a user function.
Every well-typed built-in call still compiles and behaves identically.

Found by plan-125-C Phase 3 while probing `datetime` DST claims: a probe line
wrote `datetime::toMillis(dateTimeValue)` by mistake, and it compiled and printed a
difference of `0` for two times 23 hours apart (`/tmp/p125-ex/dtdst`). Confirmed with
`/tmp/p125-ex/dtconfuse` and `/tmp/p125-ex/dtconfuse2`. **Filed, not fixed**, by user
instruction during a documentation-only plan ("file all bugs, make no fixes").

## Reproduction

```basic
IMPORT io
IMPORT datetime

SUB main()
  LET a AS datetime::DateTime = datetime::civil(datetime::date(2026, 3, 7), datetime::time(12, 0), datetime::utc())
  LET b AS datetime::DateTime = datetime::civil(datetime::date(2026, 3, 8), datetime::time(12, 0), datetime::utc())
  io::print(toString(datetime::toMillis(a)))
  io::print(toString(datetime::toMillis(b)))
END SUB
```

Observed (macos-aarch64, `worktree-P-125` at `8968ec2a4`): builds; prints `32000`
twice. Expected: build fails with `TYPE_CALL_ARGUMENT_MISMATCH` on both calls.

Also accepted, and run (`/tmp/p125-ex/dtconfuse2`):

```
toMillis(datetime::date(2026, 3, 7)) = 2026000   (the Date's year field read as Instant.seconds)
toMillis(datetime::duration(90))     = 90000     (a Duration read as an Instant)
```

## Root cause

Hypotheses, to confirm in Phase 1:

1. `datetime::toMillis` is an MFBASIC-source member (`Body::mfb`,
   `src/codegen/builtins/datetime/func_to_millis.rs`). Its parameter type is declared
   as the bare `ParameterType::named("Instant")`, while the call site's argument is the
   qualified `datetime.DateTime`. The built-in call matcher may compare these
   leniently, e.g. treating any `Named` record as compatible, or failing open when
   one side is unqualified. See the memory note "a type-keyed selector must fail
   CLOSED".
2. Alternatively, the registry's strict matcher checks the argument's shape rather
   than its identity. `Instant` and `Duration` have identical fields
   (`seconds`, `nanos`), but `DateTime` does not, so shape-matching alone would not
   explain `DateTime` being accepted.

Confirm by locating where a built-in `Body::mfb` call's arguments are checked
against its registry parameters, and comparing that with the path that produced
the user-function diagnostic.

## Non-goals

- Changing any record's layout, or any correct built-in call.
- Adding `DateTime` overloads to `toMillis`/`toNanos` as a workaround.

## Blast-radius audit

- Every built-in member whose parameter is a package record type declared bare
  (`ParameterType::named("…")`): all of `datetime`, `color`, `canvas`, `net`, `json`
  and so on. Phase 1 builds a census of which accept a wrong record.
- Built-in members taking a resource or union, to check whether the lenient
  path extends beyond records.

## Fix

Phase 1 — RED tests: `datetime::toMillis(<DateTime>)`, `(<Date>)` and `(<Duration>)`
each fail with `TYPE_CALL_ARGUMENT_MISMATCH`. Run the census above. Commit:

Phase 2 — make the built-in call check compare record identity, fully qualified,
the same way the user-function path does (GREEN); full suite, and the acceptance
goldens that exercise built-in record arguments. Commit:
