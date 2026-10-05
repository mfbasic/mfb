# bug-924: Overloaded (or `PRIVATE`) `main` is reported as "must declare an entry point named `main`"; the "exactly one" diagnostic is unreachable

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Coordinator note: repro measured by the review agent only (HEAD 28a15a153 build); the coordinator did not re-run it.
Regression Test: tests/syntax/project/entry-overloaded-main-invalid (new; name suggested)

The manifest spec's entry-point table lists "more than one matching function →
must declare exactly one entry point". In a real build that row can never fire.
`validate_entry_point` runs on the **concrete (monomorphized) HIR**, where
overloads have already been renamed to their concrete mangled names, so neither
overload of `main` is named `main`. The user then gets the opposite message:

```
FUNC main() AS Integer … END FUNC
FUNC main(args AS List OF String) AS Integer … END FUNC
→ error[2-200-0011 PROJECT_ENTRY_INVALID] Executable project must declare an entry point named `main`.
```

The same happens for a single `PRIVATE FUNC main()`, or for two `PRIVATE` mains in
different files. Private names are mangled too, so a declared `main` is "missing".
The spec does not say a `PRIVATE` entry is ineligible.

**Correct behavior:** two `main` overloads report the documented
"Executable project must declare exactly one entry point named `main`; found
multiple matching declarations." at the second declaration. A `PRIVATE main`
either works as the entry or gets a specific diagnostic ("the entry point must
not be PRIVATE"). The spec states which.

References:

- `src/docs/spec/tooling/01_project-manifest.md` *Entry Point Validation* table.

## Failing Reproduction

```
mkdir -p /tmp/e/src && cd /tmp/e
printf '{"name":"e","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src"}]}' > project.json
printf 'FUNC main() AS Integer\n  RETURN 0\nEND FUNC\nFUNC main(args AS List OF String) AS Integer\n  RETURN 1\nEND FUNC\n' > src/main.mfb
mfb build
printf 'PRIVATE FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n' > src/main.mfb
mfb build
```

- Observed (both): `./project.json:1 error[2-200-0011 PROJECT_ENTRY_INVALID] … Executable project must declare an entry point named `main`.`
- Expected: the "exactly one entry point" message at `src/main.mfb:4` for the overloads, and a specific message (or success) for the PRIVATE case.

Contrast: `main()` plus `main(a AS Integer)`, where one overload has an invalid
parameter, also reports "must declare an entry point named `main`" rather than
the documented "parameter `a` must have type List OF String".

## Root Cause

`src/cli/build/mod.rs` calls
`validate_entry_point(&options.location, &manifest, &concrete_hir)`. The audit
path (`src/audit/mod.rs:run`) does the same. In `src/manifest/entry.rs:validate_entry_point`,
`function.name != entry` then skips every renamed overload
(`src/monomorph/helpers.rs:overload_concrete_name`) and every PRIVATE-mangled
name. The unit test `validate_entry_point_rejects_multiple_matching_declarations`
(`src/manifest/mod.rs`) passes only because it validates the **pre-monomorph**
HIR (`crate::hir::elaborate(&ast)`), which is not what production feeds it.

## Goal

- The documented multiple-match message is produced by `mfb build` for overloaded `main`, and a CLI test pins it.

### Non-goals (must NOT change)

- The single-entry success path, and the `ir::EntryPoint` it returns.

## Blast Radius

- `src/manifest/entry.rs:validate_entry_point`: match on the declared source name (pre-mangle) or run on the elaborated HIR.
- `src/audit/mod.rs:run`: same call, so the fix covers both.
- `src/cli/init.rs` (calls it on a fresh project): unaffected.

## Open Decisions

- Is a `PRIVATE FUNC main` a legal entry point? Recommend: diagnose it explicitly.
