# plan-24-C: PRIVATE file-scoping via `#<hash>#name` mangling

Last updated: 2026-07-05
Effort: medium

Make `PRIVATE` a true file-local boundary: two files may each declare a `PRIVATE` symbol
with the same name without colliding, and a file-local `PRIVATE` shadows a project `PUBLIC`
of the same name *within that file* (with a warning). Achieved by physically renaming each
`PRIVATE` top-level declaration to an untypeable, file-unique internal name
`#<pathhash>#<name>` and rewriting in-file references to it, so every downstream stage
(resolve, monomorph, IR, NIR, native symbols) sees globally-unique names and needs no
per-file visibility bookkeeping. Depends on plan-24-A.

It complements:

- `./mfb spec language modules-and-packages` (`src/docs/spec/language/13_modules-and-packages.md`)
- `./mfb spec diagnostics error-codes` (new shadow WARNING; `src/docs/spec/diagnostics/02_error-codes.md`)
- Existing sigil machinery: `src/internal_name.rs` (`INTERNAL_SIGIL = '#'`, `display_name`).

## 1. Goal

- A `PRIVATE` top-level decl `foo` in file `F` is internally renamed to `#<hash(F.path)>#foo`
  (composed with the existing overload `$Type…` suffix for overloaded privates).
- References to `foo` *inside F* resolve to the mangled private; references in other files
  never see it (they resolve to a project `PUBLIC foo` if one exists, else error).
- When a file-local `PRIVATE foo` shadows a project `PUBLIC foo` (same name, and — for
  functions — after overload matching selects both), emit a WARNING at the private decl.
- Diagnostics demangle `#<hash>#foo` back to `foo` (extend `internal_name::display_name`).
- `<hash>` = 64-bit FNV-1a of the project-relative `AstFile.path` (already normalized with
  `/`), rendered as fixed hex. Compile-time collision check across all files: two distinct
  paths hashing equal is a hard error.
- Applies to every PRIVATE top-level decl kind: FUNC/SUB, TYPE/UNION/ENUM, MUT/LET/RES,
  RESOURCE, FuncAlias.

### Non-goals (explicit constraints)

- Does not change PUBLIC/EXPORT resolution or the `.mfp` export surface.
- Does not introduce per-file *namespaces* (files still don't create qualifier scopes); this
  only scopes PRIVATE names.
- No change to value/layout/ABI. Mangled names are compiler-internal symbol strings only.

## 2. Current State

- Default is now `Public` (plan-24-A); `PRIVATE` is explicit and file-local via
  `visible_from` (`Private => file.path == owner`), `src/resolver/mod.rs:491`,
  `src/syntaxcheck/mod.rs:1742`.
- BUT `insert_top_level` (`src/resolver/mod.rs:429-475`) keys top-level symbols by NAME ONLY,
  so two same-name PRIVATE decls in different files collide with
  "Top-level symbol already declared" — the core limitation this plan removes.
- `#` sigil + demangle already exist: `src/internal_name.rs` (`internalize`, `display_name`,
  `strip_sigil`). Monomorph already renames + rewrites callees (`mangle_name`
  `src/monomorph/helpers.rs:296`, `into_project` reference rewriting) — the machinery to reuse.
- `AstFile.path` is project-relative, `/`-normalized (`src/ast/types.rs:10`,
  `src/ast/manifest.rs:192`). FNV-1a already used in-tree (map hashing).

## 3. Design Overview

A single early pass — **`scope_privates`** — runs right after parse/augmentation and
BEFORE the first `resolve_project` (so every later stage sees mangled names):

1. Compute `hash(path)` per file (FNV-1a → 16-hex). Build `path→hash`; hard-error on any
   hash collision between distinct paths (`PRIVATE_PATH_HASH_COLLISION`, internal/ICE-class).
2. For each file, collect its PRIVATE top-level decl names → mangled `#<hash>#name`.
3. Rename those decls in place; rewrite every *reference within the same file* (call callees,
   type names, constructor names, binding reads) that binds to a local private → mangled name.
   References that don't match a local private are left unmangled (resolve to project PUBLIC).
4. Shadow detection: if a file has a local `PRIVATE foo` and the project also has a `PUBLIC
   foo` visible, emit the WARNING (`PRIVATE_SHADOWS_PUBLIC`) at the private decl's line. For
   functions, key the shadow on overload identity (same name+signature) so a private overload
   that merely *adds* an arity doesn't warn.

Reference rewriting is the correctness-heavy part: it must rewrite exactly the references
that bind to a top-level name (not locals, params, fields, or package-qualified `pkg::x`).
Reuse the resolver's notion of "binds to a top-level function/type/binding" — the cleanest
implementation piggybacks on the resolver: resolve as today, but when a name binds to a
file-local PRIVATE symbol, record the rename and apply it. Because `insert_top_level` keys by
name, first fix it to scope PRIVATE entries by (name, file) so same-name privates coexist.

`visible_from` becomes almost vestigial for privates (mangled names are physically distinct),
but keep it as the guard that a bare `foo` in file G cannot bind to `#<hashF>#foo`.

Correctness risk: the reference-rewrite must be complete (miss one ref → dangling call) and
must not over-rewrite (rewrite a local variable named `foo` → wrong symbol). Bound the risk
by rewriting only *top-level-binding* references the resolver already classifies.

## 4. Detailed Design — mangling & demangle

- Mangle: `format!("{SIGIL}{hash}{SIGIL}{name}")` → `#a1b2c3d4e5f6a7b8#foo`. Overloaded
  private funcs then get the existing `$Type…` suffix appended by monomorph:
  `#…#foo$Integer`.
- Demangle (`internal_name::display_name`): if `name` matches `#<hex>#<rest>`, return `<rest>`
  (then existing overload-suffix demangling applies). Add a unit test that
  `display_name("#a1b2c3d4e5f6a7b8#foo") == "foo"`.
- Hash: FNV-1a/64 over `path.as_bytes()`, `{:016x}`. Deterministic and machine-independent
  (path is project-relative), so native goldens stay reproducible.

## Layout / ABI Impact

None. Only internal symbol *strings* change, and only for explicitly-PRIVATE decls. PUBLIC/
EXPORT names are untouched, so the `.mfp` export surface and any cross-package linkage are
unchanged. Native output for programs with no PRIVATE decls is byte-identical.

## Phases

### Phase 1 — Resolver: scope PRIVATE by (name, file)

Remove the collision so same-name privates coexist; foundation for the rename.

- [ ] `src/resolver/mod.rs:429-475` `insert_top_level` — do not report a collision between
      two `Private` symbols in different files; keep collisions for PUBLIC/EXPORT and for
      same-file duplicates.
- [ ] Tests: `tests/visibility-private-samename-valid` (two files each `PRIVATE FUNC foo`,
      each calls its own → runs) currently fails to build; must build after this phase +
      Phase 2.

Acceptance: two same-name PRIVATE decls in different files no longer emit a collision error
(build proceeds to the rename phase). `scripts/test-accept.sh` green.
Commit: —

### Phase 2 — `scope_privates` pass: mangle + rewrite refs

- [ ] New module `src/scope_privates.rs` (or fold into monomorph pre-pass): path hash +
      collision check; per-file PRIVATE rename map; reference rewrite for calls/types/
      constructors/binding reads that bind to a local private.
- [ ] Wire it into the pipeline before `resolver::resolve_project` in `src/cli/build.rs:187`.
- [ ] Extend `src/internal_name.rs` `display_name` to demangle `#<hex>#name`.
- [ ] Add ICE-class diagnostic `PRIVATE_PATH_HASH_COLLISION` (table + spec) — should never
      fire; guards the hash.
- [ ] Tests: same-name private test from Phase 1 now runs; a private-calls-private-in-
      other-file case errors (not visible); demangle unit test.

Acceptance: `tests/visibility-private-samename-valid` builds and runs; a cross-file private
reference is rejected; diagnostics show `foo`, not the mangled name. `scripts/test-accept.sh`
green (goldens for explicit-PRIVATE tests now show mangled internal names — confirmed intended).
Commit: —

### Phase 3 — Shadow warning

- [ ] Add WARNING rule `PRIVATE_SHADOWS_PUBLIC` (`src/rules/table.rs` + spec registry).
- [ ] Emit it in `scope_privates` when a file's local PRIVATE shadows a project PUBLIC of
      the same name (overload-identity aware for functions).
- [ ] Tests: `tests/visibility-private-shadows-public-valid` (compiles + runs, warning
      present in output); a non-shadowing private (unique name) emits no warning.

Acceptance: shadow case emits exactly one `PRIVATE_SHADOWS_PUBLIC` warning at the private
decl; non-shadow case is silent. `scripts/test-accept.sh` green.
Commit: —

## Validation Plan

- Function tests: n/a (language mechanism). Covered by `tests/visibility-private-*`.
- Runtime proof: two files each with `PRIVATE FUNC helper` (different bodies) both called →
  program prints both results (proves no collision and correct binding).
- Doc sync: `src/docs/spec/language/13_modules-and-packages.md` (PRIVATE = file-local,
  internally file-scoped; shadow-warning behavior) + `src/docs/spec/diagnostics/02_error-codes.md`
  (new warning + ICE code).
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- Mangle in a standalone pass vs. inside monomorph's existing rename — recommend a standalone
  `scope_privates` pass before the first resolve, so pre-monomorph resolve already sees unique
  names (simpler visibility story). (§3)
- Hash width 64-bit vs 32-bit — recommend 64-bit (`{:016x}`); collision check makes either
  safe but 64-bit needs no practical worry. (§4)

## Summary

The engineering risk is the reference-rewrite completeness in `scope_privates` (miss →
dangling symbol; over-reach → wrong binding); bound it to resolver-classified top-level
references and cover with the same-name/shadow tests. Everything downstream stays
visibility-agnostic because names are unique after the pass. PUBLIC/EXPORT and native output
for private-free programs are untouched.
