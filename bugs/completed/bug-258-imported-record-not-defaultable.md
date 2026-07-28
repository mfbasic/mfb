# bug-258: an imported package's record is never "defaultable" on the source path

Last updated: 2026-07-16
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (false rejection)

Status: Fixed
Regression Test: `tests/rt-behavior/resources/resource-state-import-rt` (a package-qualified
`STATE` type — the shape that surfaced it)

An imported package's record type is rejected in every position that requires a
**defaultable** type, however ordinary the record is:

```
error[2-203-0060 TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE]: uninitialized mutable
binding requires a defaultable type
  Mutable binding `c` cannot omit its initializer because type
  `pkg.Cursor` does not have a defined default value.
```

`Cursor` is `{ pos AS Integer }` — as defaultable as a record gets. The same
rejection hits `RES f AS File STATE pkg::Cursor`, which is what makes this
plan-52's problem too: a binding package that exports both a stateful resource
and its STATE record (exactly `bindings/libsnd`'s `SfFile` + `FileInfo`) cannot
be consumed.

The correct behavior a fix produces: **an imported record is defaultable exactly
when its fields are** — the same rule its own package gets.

References:

- Found while proving plan-52-D's cross-package stateful return
  (`planning/plan-52-D-stateful-returns.md` §4). Not caused by it: reproduces on
  `MUT c AS pkg::Cursor` with no resource, no STATE, and no plan-52 change.
- `./mfb spec architecture` — the two verification paths (source vs. package).

## Failing Reproduction

A package exporting an all-Integer record, and a consumer naming it in any
defaultable position:

```basic
' package pkg
EXPORT TYPE Cursor
  pos AS Integer
END TYPE
```

```basic
' consumer
IMPORT pkg
FUNC main AS Integer
  MUT c AS pkg::Cursor          ' rejected: "does not have a defined default value"
  RETURN 0
END FUNC
```

`LET c AS pkg::Cursor = pkg::make()` works, because an initialized binding never
asks the question. Only defaultable *positions* — an uninitialized `MUT`, a
resource `STATE` — hit it, which is why this survived: nothing in-tree names an
imported record in one.

## Root Cause

`is_defaultable` (`src/ir/verify/mod.rs`) resolves a record through the local
table and treats a miss as "not defaultable":

```rust
let result = match self.record_field_lists.get(type_) {
    Some(fields) => fields.iter().all(|(_, ft)| self.is_defaultable(ft, seen)),
    None => false,          // <- an imported type lands here
};
```

On the **source path** `build` lowers with deliberately empty external maps:

```rust
let source_ir = ir::lower_project_with_external_functions(
    &concrete_ast, entry.clone(), &HashMap::new(), &HashMap::new(),
);
```

so an importer's `record_field_lists` holds only its own types, and every
`pkg.Record` misses. The answer was not "this record has no default" but "I have
never heard of this record" — and the code could not tell those apart.

## The Fix

Tell the checker which path it is on — the one question its type tables cannot
answer for themselves — and let a miss mean different things on each:

```rust
None => self.imported_types_unknown,
```

`collect_source_diagnostics` sets the flag; `check` (the package path) and every
unit test do not. So:

- **Source path** (flag set): a miss means "imported, cannot say" → do not reject.
  This is the stance the verifier already takes a few hundred lines up, for the
  same reason:

  > Only a POSITIVELY known data type rejects: an unknown name may be an external
  > package's resource (e.g. sqlite3's Db), which the source lowering has no table
  > for.

  A typo cannot ride in on it: syntaxcheck rejects an unresolvable name with
  `SYMBOL_UNKNOWN_TYPE` (verified — `MUT w AS Widget` is still rejected) before
  defaultability is ever consulted.

- **Package path** (flag clear): a miss means genuine absence → still rejects. The
  merged IR carries the full type table and every name is decoded from an id that
  must exist in it (`decode_type_name` errors on an unknown id), and ir::verify is
  the *sole* rejecter for decoded `.mfp` with no syntaxcheck behind it. A crafted
  package naming an undefaultable STATE must not get a free pass.

Two narrower attempts were tried and rejected:

- `None if type_.contains('.') => true` — only covers the *qualified* spelling, but
  an imported record is equally unknown under its bare name (`STATE Cursor`), which
  is the spelling an imported signature actually arrives in.
- `None => true` unconditionally — simpler, but it relaxes the package path too,
  where the checker has the full table and is the last line of defence. It also
  broke `rejects_mut_unknown_record_not_defaultable`, whose contract is exactly
  that: on IR with a complete type table, an unknown name is not defaultable.

## Notes

Fixing it the other way — teaching the source path the imported type table — is a
much larger change to `build`'s lowering (it deliberately lowers with empty
external maps), and the package path already covers the gap.
