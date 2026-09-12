# bug-591: a multi-overload man page's Parameters table renders only overload 1

Last updated: 2026-09-12
Effort: small-to-medium (the renderer change is small; the census of affected
pages and their goldens is the work)
Severity: MEDIUM — shipped documentation states the wrong types and prose for
every overload after the first
Class: Documentation correctness / `mfb man` renderer

Status: Open
Regression Test: a `src/cli/man.rs` rendering assertion, in the shape bug-558
used for the Errors table.

Found while fixing bug-563. **Reproduced** on the release compiler at
`ceeefcf24`.

## Reproduction

```
mfb man collections get
```

The **Synopsis** correctly lists both signatures:

```
 1. `collections::get(value AS List OF T, index AS Integer) AS T`
 2. `collections::get(value AS Map OF K TO V, index AS K) AS V`
```

The **Parameters** table then renders overload 1 only:

```
│ value     │ List OF T │ collection │ The list or map to read from. Not modified. │
│ index     │ Integer   │ key        │ The list index, zero-based. Out of range    │
│           │           │            │ raises — use collections::getOr …           │
```

A reader who came for overload 2 is told its second parameter is named `index`,
typed `Integer`, and is "the list index, zero-based" — wrong on all three counts
for a map key of type `K`.

## Why it is not cosmetic

bug-563 corrected the map overload's key description in the descriptor, because
it had been copy-pasted from the list form. That fix is **correct and invisible**:
the renderer never shows overload 2's parameters, so the page still displays the
list prose against a signature that takes a key.

So the page does not merely omit information — it attributes overload 1's types
and prose to a member whose synopsis advertises two shapes. Any future
per-overload parameter fix has the same fate until this is closed.

## Blast radius

Every multi-overload member's page, not just `collections::get`. Census
`registry()` for functions with `implementations().len() > 1` and check which
have parameters that differ in type, name, or description across overloads —
those are the pages currently rendering something false. In `collections` alone:
`get`, `set`, `find`, `findLastIndex`, `sum`, `append`, `contains`, `getOr`.

## Prior art — copy its shape

bug-558 solved exactly this problem for the **Errors** table: it added an
"Overloads" column naming which numbered overload raises each error, and landed
with the pins `a_multi_overload_errors_table_names_the_overloads_that_raise_each_error`,
`every_errors_overload_number_names_a_rendered_signature`, and the containment
pin `a_single_overload_member_errors_table_is_unchanged`.

The Parameters table never got the equivalent. The same design probably applies
(an overload column, or one table per numbered signature), and the same
containment pin is required: **a single-overload member's page must be
byte-identical afterwards**.

## Goal

- A multi-overload member's Parameters table shows each overload's own
  parameters, with that overload's own types and descriptions.

### Non-goals (must NOT change)

- Do not change a single-overload member's page at all.
- Do not change the Synopsis, Errors or See-also sections — the renderer derives
  those and bug-558 already made Errors per-overload.
- Do not "fix" it by making the overloads share one parameter description; the
  descriptions differ because the parameters differ.

## Gate

1. the RED rendering assertion fails on the current renderer;
2. `mfb man collections get` shows the map key's own type and prose;
3. **containment**: single-overload pages byte-identical — `scripts/man-census.sh`
   and the man goldens are the instrument, and the artifact gate is blind to
   renderer output, so say so;
4. a POSITIVE pin that `scripts/man-run-examples.sh` still compiles and runs
   every example on the affected pages.
