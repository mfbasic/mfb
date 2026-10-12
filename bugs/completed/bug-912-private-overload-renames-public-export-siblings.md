# bug-912: a `PRIVATE` overload renames every same-named declaration in its file — the `PUBLIC`/`EXPORT` overloads vanish from other files and from the package export table

Last updated: 2026-10-10
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Fixed
Regression Test: tests/rt-behavior/scope/private-overload-beside-export-rt (runtime: PUBLIC overload callable cross-file, EXPORT overload callable by an importer, in-file selection across both, plus both non-goals); tests/syntax/packages/package-private-overload-export (`.info` export table lists `FUNC r`, `pick$Integer$AS$String`, `pick$Integer$AS$Boolean`); unit tests `ast::scope_privates::tests::a_public_overload_beside_a_private_overload_keeps_its_name`, `ast::scope_privates::tests::a_public_func_beside_a_private_non_function_is_still_renamed`, `monomorph::lower::tests::a_private_overload_is_one_set_with_its_files_public_sibling`, `resolver::tests::a_private_and_a_public_function_of_one_signature_in_one_file_are_duplicates`, `resolver::tests::a_private_overload_or_another_files_private_is_not_a_duplicate`

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

## Fix (2026-10-10)

Root cause as found: confirmed as written above, with one addition. Not renaming the
PUBLIC/EXPORT sibling is not enough on its own, because every reference to `r` inside the
file is still rewritten to `#<hash>$r` before any type is known, and the overload set is
keyed by name in the monomorphizer — so the file's own calls to the PUBLIC overload would
have stopped resolving. The fix has three parts:

- `src/ast/scope_privates.rs:scope_privates` — a non-`PRIVATE` `FUNC` is no longer renamed
  when every `PRIVATE` declaration of its name in the file is itself a `FUNC`. References
  are rewritten exactly as before. A mixed-kind pair (`FUNC shared` beside `PRIVATE LET
  shared`, a `PRIVATE TYPE`/`RESOURCE`/alias) is not overloadable and keeps the old
  rename, so it still reports the duplicate it reported before (bug-881's territory).
- `src/monomorph/lower.rs:Monomorphizer::new` — the overload set of a file-scoped name
  gains that file's own PUBLIC/EXPORT declarations of the plain name, each keeping the
  name it was declared under; `resolve_overload` keys the chosen member's symbol by the
  member's own name. A PRIVATE member is named over that whole file-scoped set
  (`#<hash>$r$String` when a same-file sibling exists); the PUBLIC/EXPORT members are
  named by the bare name's set alone, so a PRIVATE helper never renames them. A
  template/plain mix across the two visibilities reaches the PUBLIC declaration through
  `template_name` / `sole_file_sibling`.
- `src/resolver/mod.rs:insert_function` — the two declarations no longer share a
  symbol-table key, so the same-signature duplicate check is made explicitly against the
  same file's counterpart. The diagnostic is byte-identical to before in both declaration
  orders (measured with the bug-881 repro on the pre-fix and post-fix compilers); its
  mangled spelling is still bug-881's to fix.

Not changed: `PRIVATE_SHADOWS_PUBLIC` is still emitted for a same-file PUBLIC+PRIVATE
overload pair (pinned by `shadow_warning_reports_the_line_of_each_decl_kind`); whether it
should be, now that the pair is an overload set and not a shadow, belongs to bug-881's
owner decision.

Existing goldens expected to change: none. `scripts/artifact-gate.sh target/release/mfb
all` reported `0 diff(s)` over 2326 goldens, and `scripts/test-accept.sh` over the 17
fixtures whose sources use `PRIVATE` passed — no existing fixture declares a PRIVATE
function beside a same-file PUBLIC/EXPORT function of the same name.
