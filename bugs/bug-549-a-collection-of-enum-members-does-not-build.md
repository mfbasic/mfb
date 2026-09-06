# bug-549: a `List OF <enum>` type-checks and then fails to build

Last updated: 2026-09-06 (FIXED)
Effort: unknown — small if enums are simply missing from the payload classifier
Severity: MEDIUM
Class: Correctness (valid program does not build) / Diagnostics

Status: **FIXED.** Reproduced with the release compiler, then closed — see
"The fix" below.

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

## The fix

An enum value IS its ordinal at run time: one word, exactly like an `Integer`,
with no block and no inline data. Six classifiers ask a question about a
collection element and each had an arm for every fixed-width scalar and none for
an enum:

    builder_collection_layout.rs   how WIDE is one       (emit_payload_length_to_stack)
                                   how to STORE it       (emit_copy_payload_to_collection)
                                   how to READ it back   (the packed-element reader)
    builder_collection_compare.rs  contains / find       (payload_matches_value)
                                   find over a value     (payload matches, cursor form)
                                   list = list           (payloads_match, pairwise)

Each gets the same arm the `Integer | Float | Fixed | Money` group has, behind a
new `CodeBuilder::is_enum_type` — the predicate four call sites were already
open-coding as `enum_members.keys().any(|(enum_type, _)| enum_type == other)`,
now written once.

Nothing about the comparator changed. It always had an enum arm; the payload
classifier refused the element type three steps earlier, so the arm could not be
reached from a collection at all.

Covered by `tests/rt-behavior/collections/enum-elements-rt` (in the in-process
corpus, so all five backends lower it every `cargo test`) and by the enum row in
`src/codegen/builtins/tests/collection_compare.rs`, which was written out and
commented in that file against this bug and is uncommented now. Every assertion
in the fixture is a VALUE, because a wrong width or stride would build and then
read the neighbouring element:

    list       [Red Blue Green ] len=3
    contains   Blue=TRUE Green=TRUE
    find       Blue=1 Green=2 Red=0
    find sub   1
    grown      [Green Red Blue Blue Green Red ]
    edited     [Green Blue Blue Green Red ]
    distinct   [Green Blue Red ]
    set edited len=2 Red=FALSE Green=TRUE
    map value  sky=Blue grass=Green
    map key    Red=1 Blue=2 Green=-1
    record     primary [Red Green Blue ]
    other enum len=2 Large=TRUE Small@1

The second defect the report names — the diagnostic being an internal message
with no rule code — is **not** fixed, and no longer applies to this program:
there is nothing left to refuse. It does still apply to the neighbouring
`native collection packed payload does not support type` refusals for the types
that genuinely cannot be collection elements, and bug-550 (`append([], x)`) is
another instance of the same reporting problem.
