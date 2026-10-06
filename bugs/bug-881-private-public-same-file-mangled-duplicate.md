# bug-881: a PUBLIC and a PRIVATE declaration of one name in the same file report `SYMBOL_DUPLICATE_TOP_LEVEL` with the internal mangled name

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: tests/rt-error/scope/private-public-same-file (new) — name suggested

**Owner decision (2026-10-05):** same file, same name and signature is a duplicate declaration. It stays
`SYMBOL_DUPLICATE_TOP_LEVEL`, but the detail names the symbol as written (`Top-level symbol `f`
was already declared in src/main.mfb:1.`), never the mangled `#…$f`. `PRIVATE_SHADOWS_PUBLIC`
shadowing applies only when the `PUBLIC` declaration is in **another** file of the package, and
§13's sentence says so explicitly.

When one file declares both `FUNC f` (PUBLIC by default) and `PRIVATE FUNC f`, the build fails
with `SYMBOL_DUPLICATE_TOP_LEVEL` whose detail names the file-scope mangled symbol:
"Top-level symbol `#c2515768f1aa7dc6$f` was already declared in src/main.mfb:1." The user never
wrote `#c2515768f1aa7dc6$f`. The `PRIVATE_SHADOWS_PUBLIC` warning the spec promises for a PRIVATE
that shares a name with a project PUBLIC is not shown either.

Correct behavior: at minimum the detail names `f` as written. Whether same-file PUBLIC+PRIVATE is
an error (duplicate) or the specified shadow-with-warning is an owner decision — the spec sentence
below does not exclude the same file; today it is an error.

References:

- src/docs/spec/language/13_modules-and-packages.md: "Where a file's own `PRIVATE` declaration shares a name with a project `PUBLIC` declaration, the `PRIVATE` one wins *within that file* and the compiler emits a `PRIVATE_SHADOWS_PUBLIC` warning."
- src/docs/spec/diagnostics/01_rule-codes.md "Located detail text rewrites whole known mangled generic tokens to source spellings" — private file-scope mangling has no such rewrite.
- Found during B05 diagnostics sampling (trying to trigger `PRIVATE_SHADOWS_PUBLIC`).

## Failing Reproduction

```
FUNC f() AS Integer
  RETURN 1
END FUNC
PRIVATE FUNC f() AS Integer
  RETURN 2
END FUNC
FUNC main() AS Nothing
END FUNC
```

- Observed: `main.mfb:4 error[2-201-0010 SYMBOL_DUPLICATE_TOP_LEVEL]: top-level symbol is declared more than once` / `Top-level symbol `#c2515768f1aa7dc6$f` was already declared in src/main.mfb:1.`
- Expected: detail `Top-level symbol `f` was already declared in src/main.mfb:1.` (or the shadow warning, per owner decision).

Contrast (measured): PUBLIC `f` in `a.mfb`, PRIVATE `f` in `main.mfb` → `warn[2-201-0017 PRIVATE_SHADOWS_PUBLIC]`, builds, the file's call prints `2`.

## Root Cause

`src/ast/scope_privates.rs` renames every reference to the PRIVATE name inside its file to
`mangle_private(hash, name)` — including the same-file PUBLIC declaration of that name — so the
resolver (`src/resolver/mod.rs`, the "Top-level symbol `{name}` was already declared" emitters)
sees two declarations of the mangled name and prints it raw. The warning is pushed as a
`PendingDiagnostic` and is not rendered once the resolver error aborts the build.

## Goal

- No diagnostic detail ever contains a `#<hash>$name` private mangling.

### Non-goals (must NOT change)

- Cross-file PRIVATE shadowing and its warning.

## Blast Radius

- The three `was already declared in` emitters in `src/resolver/mod.rs` — any of them can receive a mangled private name.
