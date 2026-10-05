# bug-911: a user template declared outside the first source file is instantiated in the first file's scope — wrong import alias (silent miscompile), PRIVATE helpers unreachable, wrong file in diagnostics and `ErrorLoc`

Last updated: 2026-10-05
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/generics/template-in-later-file-scope-rt (new) — name suggested

The monomorphizer emits every concrete instantiation of a user template into the
project's **first** source file (`files[0]`, by sorted path) and unions every file's
imports into that file with "the first file's own bindings win on any alias clash".
A template body is therefore resolved, type-checked and lowered in the scope of
whichever file sorts first — not the file that declares it. Four observable
consequences, all reproduced (macOS AArch64, /tmp/hardening/mfb):

1. **Silent miscompile.** If the first file binds the same import alias to a
   different package, the template body calls the *first file's* package.
2. A template body that calls/reads/constructs a `PRIVATE` declaration of its
   own file fails to compile (`SYMBOL_UNKNOWN_IDENTIFIER` / `TYPE_MEMBER_NOT_VISIBLE`,
   with the internal `#<hash>$name` spelling) — but only when its file is not
   first; renaming the file so it sorts first makes it build and print `42`.
3. Post-monomorph diagnostics inside the template body cite `files[0]` with the
   template's line number, showing an excerpt of an unrelated file.
4. Runtime `Error.source.filename` for a failure inside the body names `files[0]`.

**Correct behavior:** an instantiation is checked and lowered in its declaring
file's scope (imports, file-private names), and diagnostics / `ErrorLoc` name the
declaring file. Build output must not depend on source-file sort order.

References:

- `src/docs/spec/architecture/12_monomorphization.md` "Diagnostics inside a built-in package's body": "Every instantiation is emitted into the project's first source file, so its `HirFunction::file` names that file while its statements keep the line numbers of the package source." — the spec handles only *built-in* template bodies (`instantiated_at`); user templates in later files are not covered.
- `src/docs/spec/language/13_modules-and-packages.md` — PRIVATE is a file-local boundary; import bindings are per file.
- `src/docs/spec/architecture/02_frontend.md` "Compiler-Owned Prelude": "the monomorphizer emits generated instantiations into that first file".
- Related but distinct: bugs/completed/bug-107 (monomorph's own `report` used `files.first()`), bug-642 (built-in body lines).

## Failing Reproduction

### A. Silent wrong-package call (two packages, same alias in two files)

Packages: `pA` exports `idI(x AS Integer) AS Integer` returning `x`; `pF` exports
`idI(x AS Integer) AS Integer` returning `x + 1000`. Executable `c6` declares both
(`mfb pkg add file://…/pA.mfp`, `mfb pkg add file://…/pF.mfp`). Project dir:
/tmp/hardening/B11/c6.

`src/a.mfb`:
```
IMPORT pA AS lib

FUNC fromA() AS Integer
  RETURN lib::idI(1)
END FUNC
```
`src/z.mfb`:
```
IMPORT pF AS lib

FUNC viaTemplate OF T(x AS T) AS Integer
  RETURN lib::idI(7)
END FUNC

FUNC direct() AS Integer
  RETURN lib::idI(7)
END FUNC
```
`src/main.mfb`:
```
IMPORT io
FUNC main AS Integer
  io::print("fromA (pA) = " & toString(fromA()))
  io::print("direct (pF) = " & toString(direct()))
  io::print("viaTemplate (pF expected) = " & toString(viaTemplate(TRUE)))
  RETURN 0
END FUNC
```
```
/tmp/hardening/mfb build -q . && ./build/c6.out
```
- Observed:
  ```
  fromA (pA) = 1
  direct (pF) = 1007
  viaTemplate (pF expected) = 7
  ```
- Expected: `viaTemplate (pF expected) = 1007` — the template's own file binds `lib` to `pF`.

The same hijack with built-ins: `a.mfb` `IMPORT strings AS k`, `z.mfb`
`IMPORT collections AS k` and a template `has OF T(xs AS List OF T, x AS T)` calling
`k::contains(xs, x)` fails with `./src/a.mfb:4 TYPE_CALL_ARGUMENT_MISMATCH: Call to
`k.contains` has argument type(s) (List OF Integer, Integer), expected String, String.`

### B. PRIVATE helper unreachable (single project, /tmp/hardening/B11/min1)

`src/a.mfb`: `FUNC helperA() AS Integer` / `RETURN 1` / `END FUNC`
`src/z.mfb`:
```
PRIVATE FUNC hid() AS Integer
  RETURN 41
END FUNC

FUNC usesHid OF T(x AS T) AS Integer
  RETURN hid()
END FUNC
```
`src/main.mfb`: `io::print(toString(usesHid("q")))` in `main`.
- Observed: `./src/a.mfb:6 error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: … Callable `#2eb5ac35a29c8d59$hid` is not a top-level function.`
- Expected: builds and prints `41`. Contrast: `mv src/z.mfb src/0z.mfb` (now first) → builds and runs.
- Same with a `PRIVATE LET base` (`Identifier `#…$base` is not declared in this scope`) and a `PRIVATE TYPE Hid` constructed in the body (`TYPE_MEMBER_NOT_VISIBLE: Constructor `#…$Hid` is not visible from this file.`).

### C. Wrong file in compile diagnostic

`src/z.mfb`: `FUNC less OF T(x AS T, y AS T) AS Boolean` / `RETURN x < y`; `main` calls `less(TRUE, FALSE)`.
- Observed: `./src/a.mfb:2 error[2-203-0001 TYPE_BINARY_OPERATOR_MISMATCH]` with an excerpt of `a.mfb` lines 1–2 (`FUNC helperA() …`).
- Expected: `./src/z.mfb:2`, or the call site with an "instantiated by this call" note (as built-in bodies get).

### D. Wrong file in runtime `ErrorLoc`

`src/z.mfb`: `FUNC boom OF T(x AS T) AS Integer` / `LET d = 0` / `RETURN 10 / d`; main traps `boom(1)` and prints `e.source.filename & ":" & toString(e.source.line)`.
- Observed: `src/a.mfb:3`; `--nir` shows `('boom$Integer', 'src/a.mfb')`.
- Expected: `src/z.mfb:3`.

## Root Cause

`src/monomorph/lower.rs:Monomorphizer::into_project` pushes every generated
instantiation into `files.first_mut()` and unions all files' imports into it,
keeping the first file's binding on an alias clash (comment in that function:
"the first file's own bindings win on any alias clash"). The instantiation's
`HirFunction::file` therefore becomes `files[0]`, and every later stage keyed on the
function's file — the post-monomorph resolver's per-file import bindings and
PRIVATE-visibility checks, `ir::verify`'s constructor/field visibility
(`src/ir/verify/compat.rs` "Constructor … is not visible from this file"), IR
lowering's `IrFunction::file` (→ `ErrorLoc.filename`) — uses the wrong file.
PRIVATE names are already globally unique after `src/ast/scope_privates.rs:scope_privates`,
but visibility is still enforced per file, so the moved body loses access.

## Goal

- Each instantiation carries its declaring template's file (and that file's import
  bindings), so A prints `1007`, B prints `41` regardless of file order, C/D cite `src/z.mfb`.

### Non-goals (must NOT change)

- Mangled symbol names and package export tables.
- Built-in template bodies' `instantiated_at` call-site attribution.

## Blast Radius

- `into_project` emission for user TYPE templates (constructor/field visibility of
  PRIVATE types in a later file) — same hazard, partly reproduced via `PRIVATE TYPE Hid`.
- `HirFile::imports` union into `files[0]` — does NOT leak to the first file's
  own code: `a.mfb` calling `strings::contains` with only `z.mfb` importing
  `strings` is still rejected by the first (pre-monomorph) resolve
  (`SYMBOL_UNKNOWN_IMPORT`, verified).
