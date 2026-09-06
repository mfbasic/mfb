# bug-549: a `List OF <enum>` type-checks and then fails to build

Last updated: 2026-09-05
Effort: unknown — small if enums are simply missing from the payload classifier
Severity: MEDIUM
Class: Correctness (valid program does not build) / Diagnostics

Status: **OPEN.** Reproduced with the release compiler, not just in a harness.

## Reproduction

```basic
IMPORT collections
IMPORT io

ENUM Colour
  Red, Blue
END ENUM

FUNC main() AS Integer
  LET xs AS List OF Colour = [Colour.Red, Colour.Blue]
  IF collections::contains(xs, Colour.Blue) THEN
    io::print("yes")
  END IF
  RETURN 0
END FUNC
```

```
$ mfb build .
Building probe_enumlist (executable) for macos-aarch64
error: native collection packed payload does not support type 'Colour' while lowering bind xs AS List OF Colour
```

The front end accepts it — the failure is at lowering, from
`builder_collection_layout.rs:1859`.

## Why this is a bug and not a documented limitation

The spec says an enum IS comparable, in the same sentence as the primitives:

> Comparable types are `Integer`, `Float`, `Fixed`, `Money`, `Boolean`,
> `String`, `Byte`, `Scalar`, `Nothing`, **enum types**, and records whose
> fields are all comparable.
> — `src/docs/spec/language/04_types.md:469`

and it lists what a collection may NOT hold in the same place: "`List`, `Set`,
`Map`, unions, functions, lambdas, threads, resource handles, and the internal
fallible-result type are not comparable." Enums are on the other list. Nothing
in the spec says a collection cannot hold one.

The codegen agrees that an enum is comparable, which is what makes this look
like an omission rather than a decision:
`builder_collection_compare.rs`'s element-type `match` has an arm **specifically
for enums** — it looks the type up in the enum table and compares the two words
— and that arm is written to be reached from a collection payload. It cannot be,
because the payload classifier refuses the element type before the comparator
sees it.

## Two separate defects

1. **The program does not build.** Either the payload classifier should treat an
   enum as the word it is (the comparator already does), or the front end should
   reject `List OF <enum>` with a rule-coded diagnostic naming the restriction.
   One of those, not neither.
2. **The diagnostic is an internal one.** No rule code, no source line beyond
   the bind, and it names "native collection packed payload" — a concept a
   program author has no way to act on. Compare the rule-coded refusals the same
   area produces for a resource in a record.

## How it was found

`planning/tests.md` (the per-file coverage gate task).
`builder_collection_compare.rs` was at 73.67% and its uncovered arms are the
element types a realistic corpus does not contain; the enum arm was one of them,
and writing the program that should reach it produced this instead. The other
arms (`Boolean`, `Byte`, `Fixed`, `Set OF Integer`) all lower on all five
backends and are now covered by
`src/codegen/builtins/tests/collection_compare.rs`.

A test for this belongs in that file the day it builds — the row is written and
commented out with a pointer here, so it goes in rather than being
rediscovered.
