# bug-914: the same import alias bound to different packages in two files breaks imported-overload resolution (`TYPE_UNKNOWN_VALUE`), depending on file sort order

Last updated: 2026-10-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed (bs24-agent-h, bug-staging-24)
Regression Test: tests/runtime/rt_import_alias_per_file_overloads.rs (`cargo test --release --test rt_import_alias_per_file_overloads`)

## Cause (coordinator, bug-staging-24)

- Rule: an imported overload set is resolved against the package the CALLING FILE's own import binding names (import aliases are per file).
- Where: `src/monomorph/helpers.rs:collect_imported_overloads` accumulates one `binding -> package` map over all files and keys overloads `"{binding}.{base}"` project-wide; `resolve_imported_overload` consults it without the calling file.
- Fix: the overload index is keyed by (file, binding) (or by resolved package, looked up through the calling file's binding), and resolution passes the calling file.

Import bindings are per file, so `IMPORT pA AS lib` in one file and
`IMPORT pH AS lib` in another are independent. But the monomorphizer collects
imported overload sets in a project-wide `binding → package` map, so the later
file's binding overwrites the earlier one: a call `lib::show(1)` to `pA`'s
**overloaded** `show` is looked up among `pH`'s exports, finds no overload set,
and is left unresolved — the build fails with `TYPE_UNKNOWN_VALUE`. Which file
breaks depends only on the sorted order of the file paths.

**Correct behavior:** overload sets are resolved against the package the calling
file's own binding names; the repro prints `int:1 pH` regardless of file names.

References:

- `src/docs/spec/architecture/12_monomorphization.md` "Imported overloads across the package boundary": "`collect_imported_overloads` runs once at construction. For each distinct `(binding, package)` import pair it: …" — the code keeps one package per *binding*, not per pair.
- `src/docs/spec/language/13_modules-and-packages.md` — import bindings are file-scoped.

## Failing Reproduction

Packages: `pA` exports overloaded `show(Integer)`/`show(String)`/`show(List OF Integer)`/`show(List OF String)` (`show(1)` → `"int:1"`); `pH` exports `hello() AS String` → `"pH"`. Project /tmp/hardening/B11/c7 declares both via `mfb pkg add file://…`.

`src/a.mfb`:
```
IMPORT pA AS lib

FUNC fromA() AS String
  RETURN lib::show(1)
END FUNC
```
`src/z.mfb`:
```
IMPORT pH AS lib

FUNC fromH() AS String
  RETURN lib::hello()
END FUNC
```
`src/main.mfb`:
```
IMPORT io
FUNC main AS Integer
  io::print(fromA() & " " & fromH())
  RETURN 0
END FUNC
```
```
/tmp/hardening/mfb build -q . && ./build/c7.out
```
- Observed: `./src/a.mfb:4 error[2-203-0043 TYPE_UNKNOWN_VALUE]: value type could not be determined / RETURN value does not have a known type.`
- Expected: `int:1 pH`

Contrasts (all verified):
- rename `z.mfb`'s alias to `other` → `int:1 pH`;
- rename `src/z.mfb` → `src/0z.mfb` (sorts before `a.mfb`) → `int:1 pH`;
- a non-overloaded `pA` function (`lib::idI(1)`) through the clashing alias works.

## Root Cause

`src/monomorph/helpers.rs:collect_imported_overloads` accumulates
`let mut bindings: HashMap<String, String>` over **all** files
(`bindings.insert(binding, package)`), so one package survives per alias; the
overload map is then keyed `"{binding}.{base}"` project-wide. `resolve_imported_overload`
(`src/monomorph/lower.rs`) consults that key without the calling file, so the
earlier file's alias resolves against the wrong package's exports.

## Goal

- The repro prints `int:1 pH` for every file ordering.

### Non-goals

- The `binding.name` (not `package.name`) rewrite target the post-monomorph resolver maps per file must stay.

## Blast Radius

- `resolve_function_reference`'s imported branch (`imported_overloads`, value references `LET f AS FUNC… = lib::show`) reads the same map — same hazard, not separately run.
- If both packages export overload sets of one base name, candidates come from the wrong package; a call whose signature exists in both would rewrite to `lib.show$Integer`, which the per-file resolver then maps correctly — not observed to miscompile.

## Fix

`collect_imported_overloads` keys each overload set by `package.base` (the package,
never a binding) and stores each candidate's export name; the monomorphizer keeps
every file's own import bindings (`file_imports`) and reaches a `binding.member`
set through the binding of the file being lowered (`imported_overload_set`), both for
calls (`resolve_imported_overload`) and for value references
(`resolve_function_reference`). The rewrite target is `binding.export` with that same
binding, so the post-monomorph per-file resolver mapping is unchanged (the
Non-goal). A global initializer is now lowered with its own file current
(`into_project`), so its imported-overload call resolves through its file's binding —
before, it had no file at all and its diagnostics fell back to the first file.

RED (before): `rt_import_alias_per_file_overloads` failed with
`a.mfb:2 error[2-203-0043 TYPE_UNKNOWN_VALUE]`. GREEN (after): passes for both file
orders (`z.mfb` after `a.mfb`, `0z.mfb` before it), including an overloaded call in
a global initializer and a same-named overload set (`show`) in both packages.

Spec: `architecture monomorphization` ("Imported overloads across the package
boundary").

Goldens expected to move: none (a project with one binding per package resolves to
the same target as before).
