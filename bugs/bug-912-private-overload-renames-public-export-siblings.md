# bug-912: a `PRIVATE` overload renames every same-named declaration in its file — the `PUBLIC`/`EXPORT` overloads vanish from other files and from the package export table

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/scope/private-overload-beside-export-rt (new) — name suggested

When one file declares a `PUBLIC` or `EXPORT` function and a `PRIVATE` overload of
the same name with a *different* signature (a legal overload: "Visibility is not
part of a callable's identity, so an `EXPORT` and a `PUBLIC` or `PRIVATE`
declaration may share a name; an importer sees only the `EXPORT` members"),
file-private scoping renames **all** of them to `#<filehash>$name`. Effects:

- In a package, the export table lists `#39e5507f10a7e790$r` instead of `r`; an
  importer's `pE::r(1)` fails with `Package `pE` does not export `r``. The
  package builds with only a `PRIVATE_SHADOWS_PUBLIC` warning.
- In an executable, a `PUBLIC` function becomes uncallable from every other file.

**Correct behavior:** only the `PRIVATE` declaration(s) are file-scoped; the
`PUBLIC`/`EXPORT` siblings keep their names, the export table lists `r`, and the
importer prints `int`.

References:

- `src/docs/spec/language/06_functions.md` "Overloading" bullet (quoted above).
- `src/docs/spec/architecture/12_monomorphization.md` "An `EXPORT` counts only its `EXPORT` siblings … adding or removing that helper leaves the package's exports byte-for-byte unchanged." — violated: adding a PRIVATE helper renames the export.
- Related, distinct: bug-881 (same-signature PUBLIC+PRIVATE → `SYMBOL_DUPLICATE_TOP_LEVEL` with mangled name). This one is a *different-signature* overload, which the spec says is legal, and it miscompiles the export surface rather than erroring.

## Failing Reproduction

Package `pE` (/tmp/hardening/B11/pE), `src/lib.mfb`:
```
EXPORT FUNC r(x AS Integer) AS String
  RETURN "int"
END FUNC

PRIVATE FUNC r(x AS String) AS String
  RETURN "str"
END FUNC
```
```
/tmp/hardening/mfb build -q .          # warn PRIVATE_SHADOWS_PUBLIC, "Wrote package to ./pE.mfp"
/tmp/hardening/mfb pkg info pE.mfp | grep FUNC
```
- Observed: `  FUNC #39e5507f10a7e790$r`
- Expected: `  FUNC r`

Importer (/tmp/hardening/B11/c5, `mfb pkg add file:///tmp/hardening/B11/pE/pE.mfp`):
```
IMPORT io
IMPORT pE
FUNC main AS Integer
  io::print(pE::r(1))
  RETURN 0
END FUNC
```
- Observed: `./src/main.mfb:4 error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: … Package `pE` does not export `r`.`
- Expected: prints `int`.

Executable variant (/tmp/hardening/B11/pv): `src/a.mfb` has `FUNC r(x AS Integer) AS String` (PUBLIC, calls `r("s")`) and `PRIVATE FUNC r(x AS String) AS String`; `src/main.mfb` calls `r(1)`.
- Observed: `./src/main.mfb:3 error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]`.
- Expected: prints `intstr`.

With return-type overloads (`EXPORT r(Integer) AS String`, `EXPORT r(Integer) AS Boolean`, `PRIVATE r(Integer) AS Integer`) the exports become `#39e5507f10a7e790$r$Integer$AS$String` / `…$AS$Boolean`.

## Root Cause

`src/ast/scope_privates.rs:scope_privates` builds `rename: HashMap<name, mangled>`
from the file's PRIVATE items, then calls `rename_decl` on **every** item of the
file; `rename_decl` renames any item whose bare name is in the map, regardless of
its own visibility. `rewrite_item_refs` likewise rewrites every in-file reference
to `name`, so the file's own calls to the PUBLIC overload are also redirected to
the private namespace. The "shadows" model (a PRIVATE name replaces a PUBLIC name
within the file) is applied to overload *sets*, where both must coexist.

## Goal

- `pkg info` lists `FUNC r`; importer prints `int`; executable variant prints `intstr`.

### Non-goals

- Two files each declaring `PRIVATE FUNC helper()` must keep not colliding.
- The PRIVATE-shadows-PUBLIC rule across files (a PRIVATE in file A hiding a PUBLIC of file B within A) is unchanged.

## Blast Radius

- `rewrite_item_refs` call/identifier rewriting — same hazard (references to the PUBLIC overload in the same file are redirected); reproduced via the executable variant.
- `PRIVATE TYPE`/`RESOURCE` beside a PUBLIC one of the same name — not overloadable, so out of scope (bug-881).
