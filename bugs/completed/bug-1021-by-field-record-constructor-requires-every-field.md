# bug-1021: a by-field record constructor must name every field — `Ck[name := "a", value := "b"]` is an arity error when it should use the defaults for the fields it omits

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (language surface; owner-confirmed)

Status: Fixed (bs24-agent-i)
Regression Test: (proposed) tests/rt-behavior/types/record-constructor-partial-fields-rt (values of the omitted fields) and tests/syntax/types/record-constructor-partial-fields-invalid (the rejections below)

## Cause (coordinator, bug-staging-24)

- Rule: `<RecordType>[field := <value>, …]` may name SOME of the fields; an unnamed field takes its declared default (owner decision 2026-10-10), as a positional constructor already pads.
- Where: the by-field record-constructor check (the arity error for `Ck[name := "a", value := "b"]`) requires every field; find it from the error text in the resolver/type checker.
- Fix: the by-field constructor fills each unnamed field from its default, and refuses only a field with no default that is not named (located, coded); spec language (records) states it.

**Owner decision (2026-10-10):** `<RecordType>[field := <value>, …]` works with every field
present today. It must work with **some** of the fields as well: a field the constructor does
not name takes its default.

There is no way today to build a record in one expression without spelling every field. The
by-field constructor rejects a partial field list with `TYPE_CONSTRUCTOR_ARITY_MISMATCH`, and
`WITH` needs an existing value to copy.

## Today (measured by the coordinator, main 2f0052332, `/tmp/main-dbg-target/release/mfb`)

```
TYPE Ck
  name AS String
  value AS String
  port AS Integer
  secure AS Boolean
  ratio AS Float
END TYPE
```

| Form | Result |
| --- | --- |
| `Ck[name := "a", value := "b", port := 1, secure := TRUE, ratio := 2.0]` | builds; `[a] [b] 1 TRUE 2.00` |
| the same fields in another order | builds; same output |
| `Ck[name := "a", value := "b"]` | `TYPE_CONSTRUCTOR_ARITY_MISMATCH`: "Constructor `Ck` has 2 argument(s), expected 5." |
| `Ck[]` | `TYPE_CONSTRUCTOR_ARITY_MISMATCH`: "… has 0 argument(s), expected 5." |
| `Ck["a", "b"]` (positional, partial) | `TYPE_CONSTRUCTOR_ARITY_MISMATCH`: "… has 2 argument(s), expected 5." |
| `MUT d AS Ck` then `WITH d { name := "a", value := "b" }` (the two-statement workaround) | builds; `[a] [b] 0 FALSE 0.00` |
| `LET d AS Ck = WITH { name := "z" }` | `MFB_PARSE_EXPECTED_EXPRESSION`: the grammar requires a value after `WITH` (owner's measurement) |

So the wanted value already exists (the workaround row). Only the one-expression spelling is
missing.

## Correct behavior

`Ck[name := "a", value := "b"]` builds and equals the workaround row: `name = "a"`,
`value = "b"`, and every field it does not name holds that field type's default (`port = 0`,
`secure = FALSE`, `ratio = 0.00`).

"Default" is the value an uninitialized `MUT x AS T` gets for the field's type (language spec
§4, the defaultable-types table). A record field cannot declare its own default today
(`name AS String = "anon"` is a parse error); this bug does not add that. If per-field declared
defaults are added later, an omitted field takes its declared default, else its type's.

Rules:

1. **By-field form only.** The positional form (`Ck["a", "b"]`) keeps requiring every field and
   keeps `TYPE_CONSTRUCTOR_ARITY_MISMATCH`: a short positional list does not say which fields
   it leaves out.
2. **An omitted field must be defaultable.** If its type has no default (a `RES` field, a
   function value, a record containing one, …), the constructor is a compile error that NAMES
   the missing field or fields. Today `MUT h AS Held`, where `Held` has a `RES fs::File` field,
   reports `TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE`; the constructor's error should be as specific.
   The implementer picks the rule (reuse or a new one) and registers it per `.ai/compiler.md`.
3. **`Ck[]`** (no fields named) follows from the rule: it is the all-defaults record, the same
   value as `MUT d AS Ck`. The owner asked for "partial fields"; the empty list is the
   coordinator's reading of the limit case. **Confirm with the owner before shipping it** if
   there is any doubt (a bare `Ck[]` could also be read as a mistake).
4. A field named twice, an unknown field name, and a value of the wrong type keep today's
   diagnostics.
5. Mixing positional and by-field arguments keeps today's behavior (measure it first; do not
   widen it).
6. The same rule applies wherever a by-field constructor is accepted: a union-variant
   constructor, a nested constructor, a constructor inside a collection literal, a generic
   record (`Pair OF A, B[first := 1]`), and an imported package's exported record. Field
   visibility is unchanged: an importer still cannot name (or now, silently default) a field it
   is not allowed to construct (bug-848).
7. `WITH` is unchanged. `WITH { … }` without a base value stays a parse error; the constructor
   is where construction lives.

## Why this option (and not `WITH { … }`)

- The constructor names its type, so it works anywhere an expression does. `WITH { … }` has no
  type of its own and would work only where an expected type is supplied.
- It relaxes one check on a form that already parses; no new grammar.
- `WITH r { … }` keeps meaning one thing: a copy of `r` with changes.

## Cost to be aware of

Adding a field to a record no longer breaks its by-field constructors; they take the new
field's default. Positional constructors still break. That is the stated intent ("use the
defaults if not specified").

## Scope notes for the fix

- Front end: the constructor arity check (`TYPE_CONSTRUCTOR_ARITY_MISMATCH`) and the by-field
  binding in the shape/verify passes; lowering must materialize each omitted field's default the
  way the uninitialized-`MUT` path does (one source of truth for "the default of type `T`").
- Prove at runtime at `-O0` and `-O3`: every primitive, `String`, an enum, a nested record, a
  `List`/`Map`/`Set` field (default: empty), a union field if it has a default.
  `--debug` alloc == free for a constructor in a loop.
- Spec: language types (record constructors, §4 defaults) and the diagnostics entry; man: the
  `types` topic that shows record construction.
- Existing goldens: any fixture that pins the partial by-field arity error changes on purpose
  (list them; the owner decision is the proof).

## Fix

`lower_constructor_args` (`src/ir/lower.rs`), the by-field arm: a field the constructor does not name is
filled from `default_value_expression` — a HIR expression for the default of the field's type (the value an
uninitialized `MUT x AS T` holds), lowered with the field type expected so literals land in the type's own
representation. Types: Boolean, Integer/Byte/Fixed/Money (`0`), Float (`0.0`), String, Scalar, List/Set/Map
(empty), enum (first member), AttributedString (`astrings::fromString("")`), record (recursive, cycle-guarded).
No default for a function, `Result`, resource, thread, union or a record holding one: the field stays out of
the list and the existing arity rule fires. It fills only when every name written IS a field, so a stray name
keeps today's arity/argument diagnostics (no silent default for a typo). Positional lists are untouched.
`ir::verify::check_constructor` names the fields with no default in the arity message
("Field(s) `f` have no default value and must be given one."). `Ck[]` is the all-defaults record (the
coordinator's reading; owner confirmation still advised). Spec `language types`, man `types`, tour updated.

Tests: `tests/runtime/rt_record_constructor_partial_fields.rs`, both RED on main's compiler
(`omitted_fields_take_their_defaults_at_o0_and_o3`: build refused; `..._still_refused`: no field named),
GREEN after. Covers every primitive, String, enum, nested record, List/Map/Set, color::Color, generic record,
union variant, collection literal, loop, at -O0 and -O3 (the omitted-field rows equal the `MUT d AS Ck` row).
Not tested here: an imported non-builtin `.mfp` record, `--debug` alloc==free.

Expected golden movement: `tests/syntax/types/types-duplicate-field-invalid/golden/build.log` loses its
`TYPE_CONSTRUCTOR_ARITY_MISMATCH` block for `Point[x := 1, x := 2]` (y is now defaulted; the
`TYPE_DUPLICATE_FIELD` block stays) — the owner decision is the proof. types-constructor-invalid and
func_json_stringify_invalid accepted unchanged. Known unknown: `Error`/`ErrorLoc` fields (defaultable per
verify, no type-index entry here) are treated as having no default.
