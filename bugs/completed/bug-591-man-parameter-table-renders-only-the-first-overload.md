# bug-591: a multi-overload man page's Parameters table renders only overload 1

Last updated: 2026-09-12
Effort: small-to-medium (the renderer change is small; the census of affected
pages and their goldens is the work)
Severity: MEDIUM — shipped documentation states the wrong types and prose for
every overload after the first
Class: Documentation correctness / `mfb man` renderer

Status: **FIXED** (`8361ec0c6`).
Regression Test: `every_overloads_parameters_appear_in_its_parameters_table`,
`collections_get_shows_the_map_key_as_its_own_parameter_row`,
`a_parameters_table_with_no_disagreement_has_no_overloads_column`, and
`every_parameters_overload_number_names_a_rendered_signature` (`src/cli/man.rs`).

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

## Outcome

Fixed in `8361ec0c6`.

### Root cause, measured

`union_parameters` built the table by de-duplicating on parameter **name**, first
occurrence winning. That is correct for overload sets that only ADD parameters —
`process::spawn`'s `cwd`, `env` and `envReplace` — and silently wrong when two
overloads declare the same name in different forms. `collections::get`'s map
overload takes `index AS K`, "The key to look up", and was replaced on the page by
overload 1's `index AS Integer`, "The list index, zero-based".

It was an implementation choice from the renderer rewrite (`31276d8c4`). No spec
text or `.ai/man-content.md` rule mandates first-occurrence-wins, and the one test
pinning the union (`multi_overload_function_renders_overloads_and_union_parameters`)
uses `spawn` and asserts only that each name appears. It says nothing about
same-named parameters of different types, so it did not have to change.

### Fix

The same remedy bug-558 applied to the Errors table:

- Rows are grouped by how a parameter RENDERS: name, type, description, aliases
  and optional flag. Those are exactly the cells a reader sees.
- When any name has more than one rendered form, the table gains an **Overloads**
  column naming which numbered signature each row belongs to.
- When no name differs, the column is omitted and rows keep their old
  first-appearance order. Single-overload pages and additive overload sets
  render byte for byte as before.

`union_parameters` was removed rather than left dead.

### RED and GREEN on the same four tests

| test | pre-fix renderer | fix |
|---|---|---|
| `every_overloads_parameters_appear_in_its_parameters_table` | **FAILED** | ok |
| `collections_get_shows_the_map_key_as_its_own_parameter_row` | **FAILED** | ok |
| `a_parameters_table_with_no_disagreement_has_no_overloads_column` | ok | ok |
| `every_parameters_overload_number_names_a_rendered_signature` | ok | ok |

RED was run by rebuilding `man.rs` from the pre-fix commit with only the four new
tests added: `38 passed; 2 failed`, exit 101. The two that fail are the ones
asserting the bug. The two that pass in both columns are the containment pin and
the cross-reference pin — **they are supposed to pass in both**, and a pin that
only passed after the fix would be asserting the new behaviour rather than
guarding the old.

GREEN: `cargo test --bin mfb -- cli::man::tests` = **40 passed, 0 failed**, exit 0.
`rustfmt --check` exit 0.

The first test is **total over the registry**. Every multi-overload member's every
overload's every parameter must appear on one row with its own name and rendered
type. There is no list to maintain, so the next member to reuse a parameter name
in a new form is caught without anyone adding it here.

### Instrument

The artifact gate cannot see this — renderer output is not a golden. The
`cli::man::tests` module is the instrument.

### What it unblocks

bug-563's correction to the map key's description is now visible on
`mfb man collections get`, where it had been right in the descriptor and never
rendered.
