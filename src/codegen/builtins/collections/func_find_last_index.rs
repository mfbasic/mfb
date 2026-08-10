//! `collections::findLastIndex` — descriptor entry + MFBASIC source body.
//!
//! Converted from an inline `native(...)` descriptor stub to
//! `Implementation::Mfb`: body moved out of `package.mfb`, authored doc consts
//! moved here from `mod.rs`. Still source-generic (a call monomorphizes
//! `__collections_findLastIndex`) with a separate native String fast path in
//! `src/target`. Body byte-significant (2-space indent → `.ncode` columns); do
//! not reformat.

use super::{custom, opt, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str =
    "Index of the last element at or before an end position that satisfies a predicate";
const DESC: &str = r#"`collections::findLastIndex` scans `value` **backward**, beginning at the
element selected by `endIndex` and decreasing by one down to index `0`, calling
`predicate` with each element. It returns the zero-based index of the first
element (in that backward order) for which `predicate` returns `TRUE` — that is,
the last matching element at or before `endIndex`. The scan short-circuits at
that element: no lower index is examined. When the scan passes index `0` without
a match, the call raises `ErrNotFound` (`77050004`) rather than returning a
sentinel index.

The third parameter is named `endIndex`. It is resolved in two steps, and the
order matters:

1. **Negative resolution.** A negative `endIndex` counts from the end of the
   list: the effective index becomes `len(value) + endIndex`. The default of
   `-1` therefore selects the last element, so the common call form scans the
   whole list from its end. A non-negative `endIndex` is used as written.
2. **Range check.** *After* resolution, the call raises `ErrIndexOutOfRange`
   (`77050001`) when the resolved index is less than `0` or greater than or
   equal to `len(value)`.

Because the range check runs on the resolved index, the upper bound is
`len(value) - 1`, not `len(value)`. This is deliberately asymmetric with
`collections::findIndex`, whose `start` may equal `len(value)` and whose
negative values are rejected instead of resolved.

One consequence is worth stating explicitly: on an **empty** list `len(value)`
is `0`, so every `endIndex` resolves outside `0 .. -1` and is rejected. The
default `-1` resolves to `-1`, which fails the range check. `findLastIndex` on
an empty list therefore raises `ErrIndexOutOfRange` (`77050001`), **not**
`ErrNotFound`. A caller that treats "no match" and "empty input" alike must
handle both codes.

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` propagates out of the `collections::findLastIndex` call to
the caller rather than being reported as a non-match. Note that a lambda passed
here may not capture an outer `MUT` binding; the callback position proven
non-escaping is `collections::forEach`, not `findLastIndex`.

`findLastIndex` is a generic implemented in MFBASIC source; a call is rewritten
to the internal `__collections_findLastIndex` generic and instantiated for the
element type like any other generic function.
It does not mutate `value`."#;

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_findLastIndex OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean, endIndex AS Integer = -1) AS Integer
  MUT e AS Integer = endIndex
  IF e < 0 THEN
    e = len(value) + e
  END IF
  IF e < 0 OR e >= len(value) THEN
    FAIL error(77050001, \"List or string index/range is outside valid bounds.\")
  END IF
  MUT i AS Integer = e
  WHILE i >= 0
    IF predicate(collections::get(value, i)) THEN
      RETURN i
    END IF
    i = i - 1
  END WHILE
  FAIL error(77050004, \"Requested item, key, file, or resource was not found.\")
END FUNC";

pub(crate) const FIND_LAST_INDEX: BuiltinFunction = BuiltinFunction::mfb(
    "collections.findLastIndex",
    "findLastIndex",
    INTRO,
    DESC,
    &["ErrIndexOutOfRange", "ErrNotFound"],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("predicate", &[], "FUNC(T) AS Boolean"),
        opt("endIndex", &[], "Integer"),
    ])],
    BODY,
);
