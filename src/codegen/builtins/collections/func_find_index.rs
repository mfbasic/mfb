//! `collections::findIndex` — descriptor entry + MFBASIC source body.
//!
//! Converted from an inline `native(...)` descriptor stub (Implementation::Same,
//! source-generic) to `Implementation::Mfb`: the body moved out of `package.mfb`,
//! and the authored doc consts moved here from `mod.rs`. Still source-generic (a
//! call monomorphizes `__collections_findIndex`); the native String fast path for
//! `findLastIndex` is unaffected. Body byte-significant (2-space indent →
//! `.ncode` columns); do not reformat.

use super::{custom, opt, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str =
    "Index of the first element at or after a start position that satisfies a predicate";
const DESC: &str = r#"`collections::findIndex` scans `value` **forward**, beginning at index `start`
and advancing by one, calling `predicate` with each element. It returns the
zero-based index of the first element for which `predicate` returns `TRUE`. The
scan short-circuits at that element: no later element is examined. When the scan
reaches the end of the list without a match, the call raises `ErrNotFound`
(`77050004`) rather than returning a sentinel index.

`start` defaults to `0`, so the common call form scans the whole list. It is
validated **before** any element is read: the call raises `ErrIndexOutOfRange`
(`77050001`) when `start < 0` or `start > len(value)`. Two consequences are
worth stating precisely:

- `start` equal to `len(value)` is **legal**. It selects an empty scan, so the
  call raises `ErrNotFound`, not `ErrIndexOutOfRange`. `start` strictly greater
  than `len(value)` is the out-of-range case.
- A negative `start` is **not** interpreted as an offset from the end of the
  list. It is simply out of range and raises `ErrIndexOutOfRange`. This is
  deliberately asymmetric with `collections::findLastIndex`, whose `endIndex`
  parameter *does* resolve negative values from the end.

On an empty list every legal `start` is `0`, which is `len(value)`, so
`findIndex` on an empty list raises `ErrNotFound`.

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` propagates out of the `collections::findIndex` call to the
caller rather than being reported as a non-match. Note that a lambda passed here
may not capture an outer `MUT` binding; the callback position proven
non-escaping is `collections::forEach`, not `findIndex`.

`findIndex` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_findIndex` generic and instantiated for the element
type like any other generic function.  It
does not mutate `value`."#;

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_findIndex OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean, start AS Integer = 0) AS Integer
  IF start < 0 OR start > len(value) THEN
    FAIL error(77050001, \"List or string index/range is outside valid bounds.\")
  END IF
  MUT i AS Integer = start
  WHILE i < len(value)
    IF predicate(collections::get(value, i)) THEN
      RETURN i
    END IF
    i = i + 1
  END WHILE
  FAIL error(77050004, \"Requested item, key, file, or resource was not found.\")
END FUNC";

pub(crate) const FIND_INDEX: BuiltinFunction = BuiltinFunction::mfb(
    "collections.findIndex",
    "findIndex",
    INTRO,
    DESC,
    &["ErrIndexOutOfRange", "ErrNotFound"],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("predicate", &[], "FUNC(T) AS Boolean"),
        opt("start", &[], "Integer"),
    ])],
    BODY,
);
