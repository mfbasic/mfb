# bug-602: do `Error` and `ErrorLoc` values alias the way the pointer-`String` records did?

Last updated: 2026-09-12
Effort: small to test; unknown to fix (depends on what the test shows)
Severity: unknown until tested — HIGH if it reproduces (bug-601's shape is a SIGSEGV and a silent wrong value)
Class: Memory-safety / value semantics — investigation

Status: Open — **not yet reproduced**. Filed on the owner's instruction so the question is
tested, not assumed.
Regression Test: — (the probes below, once written)

## Why this is filed

bug-601 showed that `MUT ys = xs` on a list of `net::Address` shares one block: the
compiler makes no copy of a value that is not `type_is_memcpy_copyable`, and the
pointer-`String` records (`net::Address`, `udp::Datagram`, `audio::AudioDevice`) are not.
An in-place `removeAt` on the copy empties the source; a growing `append` frees it. The owner
decided (2026-09-12) to flatten those three records onto the ordinary inline-`String` layout.

`Error` and `ErrorLoc` were suspected to be on the same pointer layout, because the doc
comment on `CodeBuilder::is_pointer_string_record`
(`src/codegen/collection/layout/builder_collection_layout.rs`) lists them: "`Error`/`ErrorLoc`:
the fallible-call ABI, trap materialization, `FAIL`".

**The sources disagree, and the spec plus the code say that comment is stale:**

- The free function `is_pointer_string_record` names only `net::Address`, `udp::Datagram`
  and `audio::AudioDevice`. It does not list `Error` or `ErrorLoc`.
- `mfb spec memory` (`src/docs/spec/memory/03_heap-values.md`, "`Error` and `ErrorLoc`")
  says they are **flat built-in records**. Their `String`/sub-record fields are inlined by
  block-relative offset, "so the whole value is a single pointer-free block", and "copying an
  `Error` is one `memcpy`". A null `source` is an offset-0 sentinel.

If that holds, an `Error` copy is a real copy and bug-601's aliasing cannot happen, whatever
the ownership model. **That is a reading, not a measurement** — this bug exists to run the
probes below and confirm it, and to correct the stale comment either way.

## The owner's hypothesis (to test, not to assume)

> there is only 1 of each so I *think* they are treated more like a RES than a LET or MUT.

That is: an `Error` value is produced once (by a raise, `FAIL`, or a `TRAP` binding) and then
moved rather than copied, so there may be no second binding that could share its block —
which would make the bug-601 shape unreachable for them even though the layout is the same.

## What to test

1. **Where `Error`/`ErrorLoc` get the pointer layout and non-copyability.** Find the route
   (not `is_pointer_string_record`) and whether `type_is_memcpy_copyable` is false for them.
2. **Can two bindings hold one `Error`?** Probe each way a second name can reach the value,
   then mutate or drop one and read the other:
   - `TRAP(e)` then `LET e2 = e` / `MUT e2 = e`;
   - an `Error` stored in a record field or a collection (`List OF Error`, if the language
     allows it), then copied out;
   - an `Error` passed to a `FUNC` parameter and returned;
   - an `Error` captured by a closure;
   - `FAIL e` re-raising a bound error, then reading the original binding.
   Read `e.message`, `e.code` and the `ErrorLoc` (file/line) from both names after each step.
3. **Mutation paths.** An `Error` has no in-place collection operation of its own, so the
   likely hazard is a *list* of errors (`removeAt`/`append` in place) or a record holding one —
   bug-601's exact shape. Try it if the type system permits `List OF Error`.
4. **If the language forbids the second binding** (the `RES`-like hypothesis holds), record
   the rule that forbids it, with the diagnostic or the spec citation, and close this as
   not-a-defect. If it permits it and the values share, reproduce the corruption and fix it.

## Relation to other bugs

- **bug-601 / bug-599**: the original aliasing and leak for the pointer-`String` records.
- **bug-573**: an unrelated leak — a raised error orphaned its `ErrorLoc` (fixed
  `982a52e17`); not aliasing.
- **bug-593**: an inline `TRAP`'s `Result` wrapper now has an owner; relevant because `TRAP`
  is one of the paths that binds an `Error`.
