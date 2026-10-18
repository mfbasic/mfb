# bug-1108: a by-field constructor that leaves out a field with no default reports errors about a `LET`/`MUT` binding the user never wrote

Last updated: 2026-10-18
Effort: small (<1h)
Severity: MEDIUM
Class: Other (diagnostics quality; regression from bug-1021)

Status: Open
Regression Test: `tests/runtime/rt_record_constructor_partial_fields.rs:undefaultable_unknown_and_positional_short_lists_are_still_refused` (its first assertion pins today's wording and must change with the fix)

Found by the bug-staging-24 coordinator reviewing bug-1021, and placed in the next phase.

bug-1021 (owner decision) lets a by-field constructor leave fields out; an omitted field
takes the default an uninitialized `MUT x AS T` gets, by calling a synthesized function
whose body is `MUT <field> AS T` / `RETURN <field>` (`src/ir/lower.rs:omitted_field_default`),
so there is one definition of a type's default. For a field whose type has NO default, the
refusal then comes from that synthesized binding.

**The single correct behavior a fix produces:** one located diagnostic that says the
constructor leaves out field `f` of `Held`, whose type `RES fs::File` has no default, so
the field must be named; no message mentions `LET`, `MUT` or a binding.

## Reproduction (bug-staging-24 with bug-1021)

```
IMPORT fs

TYPE Held
  label AS String
  f AS RES fs::File
END TYPE

FUNC main AS Integer
  LET a = Held[label := "x"]
  RETURN 0
END FUNC
```

```
./src/main.mfb:9 error[2-203-0082 TYPE_RESOURCE_REQUIRES_RES]: resource must be bound with RES
               binding `f` holds resource `fs.File`; bind it with `RES`, not `LET`/`MUT`.
./src/main.mfb:9 error[2-203-0060 TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE]: uninitialized mutable binding requires a defaultable type
               Mutable binding `f` cannot omit its initializer because type `RES fs.File` does not have a defined default value.
```

Main `76fbacb3e` (before bug-1021) reports one `TYPE_CONSTRUCTOR_ARITY_MISMATCH`.

## Where

The decision "this omitted field has no default" is made only by the verifier's
`src/ir/verify/resources.rs:is_defaultable` on the synthesized binding. A fix asks the same
predicate about the constructor's omitted field and reports it there, against the
constructor, before (or instead of) checking the synthesized function.
