# bug-653: a `PUBLIC FUNC` whose name collides with an `EXPORT FUNC` makes the EXPORT unusable from every consumer — and the package still builds green

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness / Footgun (silent at the package boundary)

Status: Open
(Filed as bug-648, which collided with `bug-648-a-trapped-poll-result-tombstones-a-live-list-element.md`; renumbered.)
Regression Test: none yet — see Phase 1 (proposed:
`tests/runtime/rt_package_public_export_name_collision.rs`, alongside
`tests/runtime/rt_package_private_type_collision.rs` and
`tests/runtime/rt_imported_overload_imported_field_argument.rs`)

When a package declares a package-internal `PUBLIC FUNC name(...)` in one file and an
`EXPORT FUNC name(...)` with a *different* signature in another, the exported function
becomes uncallable from every consumer. Inside the package both resolve fine by arity.
From outside, any `pkg::name(...)` either fails to type or fails to link.

**What makes this dangerous is that it is silent at the package boundary.** The package
compiles, its own `TESTING` suite passes, and the `.mfp` is written — every instrument the
package author has says green. The defect only appears in a *consumer*, which the package's
own tests are not. There is no diagnostic at package-build time naming the collision.

**The single correct behavior a fix produces** (settled by the spec, see Fix Design): given a
package with `EXPORT FUNC f(x AS String) AS String` and
`PUBLIC FUNC f(a AS Integer, b AS Integer) AS String`, the package builds, a consumer's
`pkg::f("x")` compiles, links, and returns the `EXPORT` overload's result, and a consumer's
`pkg::f(1, 2)` is a located compile error — the `PUBLIC` overload is hidden from importers.

References:

- `.ai/resources-packages.md` — the package/import subsystem; "Type-export closure feeds
  BOTH validation and codegen" was the starting point for the export-table investigation.
- `src/docs/spec/architecture/12_monomorphization.md` — documents
  `[[src/monomorph/helpers.rs:collect_imported_overloads]]`, including the rule this bug
  turns on ("`collect_imported_overloads` runs once at construction. For each distinct…").
- Found on 2026-09-15 while executing plan-138-D (`planning/plan-138-D-xml-xpath-subset.md`),
  whose spike was the first real *consumer* of `xml::textOf`.
- Real-world instance and its workaround: commit `049528d98`
  ("fix(packages/xml): xml::textOf was unusable from every consumer"), found with
  `git log --oneline -S 'PUBLIC FUNC textOf' -- packages/xml/src/scan.mfb`. That commit's
  message ends "The compiler behaviour underneath … is filed as its own bug document" —
  this is that document.
- Relatives, all package-boundary name-resolution defects:
  `tests/runtime/rt_package_private_type_collision.rs` (bug-624, a package PRIVATE type
  vs. a consumer type of the same name — the type-side analogue of this bug);
  `tests/runtime/rt_imported_overload_imported_field_argument.rs` (bug-631, imported
  overload resolution); `bugs/bug-628-unlisted-transitive-package-call-fails-unlocated-nir.md`
  (the same unlocated `NIR call target … does not resolve` failure mode seen below).

## Failing Reproduction

Compiler under test: `target/release/mfb` built from this worktree.
`./target/release/mfb --version` prints `MFBasic Compiler 0.1.0 / 2026-09-15 22:29:23 UTC /
Local Development`.

Scratch tree (built from scratch for this document, kept out of the repo):
`/tmp/bug648-minimal` — a package `dup` with two source files, plus a consumer `app`.

`/tmp/bug648-minimal/dup/src/lib.mfb`:

```basic
' The package's documented public API.
EXPORT FUNC f(x AS String) AS String
  RETURN "export:" & x
END FUNC

' Proof the package's own scope resolves both overloads by arity.
EXPORT FUNC both() AS String
  RETURN f("a") & "|" & f(1, 2)
END FUNC
```

`/tmp/bug648-minimal/dup/src/helper.mfb`:

```basic
' Package-internal, cross-file. Same NAME as the EXPORT above, different signature.
PUBLIC FUNC f(a AS Integer, b AS Integer) AS String
  RETURN "public:" & toString(a + b)
END FUNC
```

`/tmp/bug648-minimal/app/src/main.mfb`:

```basic
IMPORT dup
IMPORT io

FUNC main() AS Integer
  LET s AS String = dup::f("x")
  io::print(s)
  io::print(toString(len(dup::f("x"))))
  io::print(dup::both())
  RETURN 0
END FUNC
```

### Step 1 — the package builds and tests green (this is the silent part)

```
./target/release/mfb build /tmp/bug648-minimal/dup   # exit 0
./target/release/mfb test  /tmp/bug648-minimal/dup
```

- Observed, `build`: `Building dup (package) for macos-aarch64` /
  `Wrote package to /tmp/bug648-minimal/dup/dup.mfp`; `echo $?` prints `0`. No diagnostic,
  no warning about the colliding name.
- Observed, `test` (with a `TESTING` block added at
  `/tmp/bug648-minimal/dup/src/test_f.mfb` asserting `f("a") = "export:a"` and
  `f(1, 2) = "public:3"`): `Tests: 2  Pass: 2  Fail: 0`. Both overloads resolve by arity
  **inside** the package. This is why a package's own suite can never catch the bug.

### Step 2 — the consumer cannot use the export

```
cp /tmp/bug648-minimal/dup/dup.mfp /tmp/bug648-minimal/app/packages/dup.mfp
./target/release/mfb build /tmp/bug648-minimal/app   # exit 1
```

- Observed (verbatim, `mfb build /tmp/bug648-minimal/app`):

```
/tmp/bug648-minimal/app/src/main.mfb:5 error[2-203-0043 TYPE_UNKNOWN_VALUE]: value type could not be determined
               Initializer for binding `s` does not have a known type.
/tmp/bug648-minimal/app/src/main.mfb:7 error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: function call argument type does not match parameter type
               Call to `toString` has argument type(s) (Unknown), expected Integer, Float[, Byte], Fixed[, Byte], Boolean, String, Byte, Scalar, or List OF Byte.
/tmp/bug648-minimal/app/src/main.mfb:7 error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: function call argument type does not match parameter type
               Call to `len` has argument type(s) (Unknown), expected String, List OF T, Set OF T, or Map OF K TO V.
```

  `echo $?` prints `1`, and `ls /tmp/bug648-minimal/app/build/` reports
  `No such file or directory` — no executable is produced.

- Expected: the executable builds and prints `export:x`, `8`, `export:a|public:3`
  (which is exactly what the Contrast cases below produce).

### Second failure mode — a plain call position, unlocated

The diagnostic depends on the *call form*, not the arity. Replacing `main.mfb` with
`io::print(dup::f("x"))` (no `LET`) — tree `/tmp/bug648-callpublic` — and running
`./target/release/mfb build /tmp/bug648-callpublic/app` gives:

```
error: NIR call target 'dup.f' does not resolve
```

No file, no line, no error code. The same output appears for `dup::f(1, 2)` and
`dup::f("x", "y")` (all three run through the same command). This is the same class of
unlocated late failure as `bugs/bug-628-unlisted-transitive-package-call-fails-unlocated-nir.md`,
and it is the load-bearing clue: **the callee is still the bare name `dup.f`** — it was
never rewritten to the mangled export the `.mfp` actually carries.

### Contrast cases (these work today, and bound the bug)

| Variant | Tree | Package build | Consumer build | Run |
| --- | --- | --- | --- | --- |
| `EXPORT f(String)` + `PUBLIC f(Integer, Integer)` | `/tmp/bug648-minimal` | ✓ exit 0 | ✗ exit 1, `TYPE_UNKNOWN_VALUE` | — |
| `EXPORT f(String)` + `PUBLIC g(Integer, Integer)` (internal renamed) | `/tmp/bug648-control` | ✓ exit 0 | ✓ `Wrote executable` | `export:x` / `8` / `export:a|public:3`, exit 0 |
| `EXPORT f(String)` + `EXPORT f(Integer, Integer)` (both exported) | `/tmp/bug648-twoexport` | ✓ exit 0 | ✓ `Wrote executable` | `export:x` / `8` / `export:a|public:3`, exit 0 |

Each row was produced by `mfb build <pkg>`, `cp <pkg>/dup.mfp <app>/packages/dup.mfp`,
`mfb build <app>`, then running `<app>/build/app.out`.

The third row is the **isolator**. Two `EXPORT` overloads of the same name are written to
the `.mfp` under the *same* mangled spellings as the failing case, and the consumer calls
them without trouble. So mangling alone is not the defect: the defect is that one of the
two mangled siblings is `PUBLIC` and therefore never reaches the export table.

### Real-world instance

`packages/xml` shipped `EXPORT FUNC textOf(n AS Node) AS String` in `src/lib.mfb` and
`PUBLIC FUNC textOf(bytes AS List OF Byte, from AS Integer, stop AS Integer) AS String` in
`src/scan.mfb`. `xml::textOf` — a documented public API function — could not be called by
any consumer, in any of four call shapes (`/tmp/xml-lenrepro`). Renaming the internal helper
to `sliceText` fixed every symptom with no other change (`git show 049528d98`); the package
passed its 158 `TESTING` cases both before and after. Letter C's three-way differential
oracle (2,140 W3C conformance tests plus fuzzing, per the `049528d98` commit message) also
never caught it — for the same reason: none of those instruments is a consumer.
At this worktree's HEAD the helper is already `sliceText`
(`git show HEAD:packages/xml/src/scan.mfb | grep -n sliceText` → line 68), so the repo is
not currently broken; the *compiler* behaviour is what this document tracks.

| Environment | Details | Result |
| --- | --- | --- |
| macOS aarch64 (this worktree) | `mfb 0.1.0`, build `2026-09-15 22:29:23 UTC` | fails ✗ |
| Other targets | not exercised — the mechanism below is in the shared front end and `.mfp` writer, so it is expected to be target-independent (unverified) | unknown |

## Root Cause

### The verdict on the open question: the exported overload is PRESENT in the `.mfp`, but under its MANGLED name — and the `PUBLIC` sibling is absent entirely, so nothing is "shadowed"

This was settled by decoding the export table of three `.mfp` files built by the commands
above (a scratch `MFPC` decoder walking the 16-byte header, the 24-byte section table, the
section-2 string pool, the section-6 export table and the section-8 function table, per
`wire/src/mfpc.rs:encode_sections`). Cross-checked against
`strings -a -n 1 /tmp/bug648-minimal/dup/dup.mfp`.

| Package | EXPORT table rows | FUNCTION table |
| --- | --- | --- |
| `EXPORT f(String)` + `PUBLIC f(Integer, Integer)` | **1**: `f$String` | `f$String` private=false; `f$Integer$Integer` private=**true** |
| control, `PUBLIC` renamed to `g` | **1**: `f` (bare) | `f` private=false; `g` private=true |
| both `EXPORT` | **2**: `f$String`, `f$Integer$Integer` | both private=false |

Note the control's single row is the **bare** `f`, while the failing case's single row is
the **mangled** `f$String`. That one-character difference is the whole bug.

Corroboration from `packages/xml/xml.mfp` (same decoder): 16 exports; `textOf` present
**bare** post-rename, and `stringify` present as six `$`-mangled rows — which consumers
resolve fine precisely because all six are `EXPORT`.

### The chain, cited

1. **Mangling is visibility-blind.** `src/monomorph/lower.rs:Monomorphizer::new` collects
   every `HirItem::Function` into `function_overloads`, keyed by bare name, with no
   visibility filter (`sed -n '34,80p' src/monomorph/lower.rs` — the `HirItem::Function`
   arm pushes unconditionally). When that vector holds more than one entry, both go through
   `src/monomorph/helpers.rs:overload_concrete_name` → `mangle_name`. So the *exported*
   `f` is renamed `f$String` **because a `PUBLIC` sibling exists**.
2. **The writer then drops the sibling that justified the mangling.**
   `src/binary_repr/writer.rs:lower_function` sets `FUNCTION_FLAG_PRIVATE` for anything
   whose `visibility != "export"` (`sed -n '755,762p' src/binary_repr/writer.rs`), and
   `src/binary_repr/sections.rs:is_exported_function` filters on exactly that flag
   (`function.kind == FUNCTION_BINARY_REPR && function.flags & FUNCTION_FLAG_PRIVATE == 0`).
   Result: **one** export row, carrying a mangled name whose partner is gone.
3. **The consumer's un-mangler requires two or more rows.**
   `src/monomorph/helpers.rs:collect_imported_overloads` groups the rows returned by
   `crate::binary_repr::read_package_exports` by `export.name.split('$').next()`, then:
   `if exports.len() < 2 { continue; // Non-overloaded imports resolve by their bare name. }`.
   With one row the `binding.base → binding.name$…` rewrite is never registered, so
   `src/monomorph/lower.rs:resolve_imported_overload` returns `None` at its very first line
   (`let candidates = self.imported_overloads.get(callee)?;`) and the callee stays `dup.f`.
4. **The signature table is keyed by the mangled name, so the bare callee misses.**
   `src/manifest/package.rs:external_package_function_types_from_files` inserts each
   signature under `format!("{package_name}.{}", export.name)` — here `dup.f$String`.
   `src/ir/lower.rs:lower_facts` seeds the checker's function type/param/return maps from
   that map, so the lookup for `dup.f` finds nothing: the call types `Unknown`, and
   `src/ir/shape.rs:check_initializer_known` cascades that into
   `TYPE_UNKNOWN_VALUE` ("Initializer for binding `s` does not have a known type."). In a
   plain call position no signature is consulted at all and the bare `dup.f` survives to the
   NIR link step, producing the unlocated `NIR call target 'dup.f' does not resolve`.

Why the contrast cases are immune: with the internal helper renamed (row 2), step 1 never
fires — there is one `f`, so it is written bare and the bare lookup in step 4 hits. With
both exported (row 3), step 2 keeps both rows, so step 3's `len() >= 2` holds, the rewrite
is registered, and `resolve_imported_overload` picks `f$String` by arity.

### Adjacent, not this bug

`src/binary_repr/writer.rs:external_function_metadata` keys `external_function_ids` /
`external_function_returns` by `{package}.{export_name}` as well — a genuine bare-name
collision hazard, but not this defect, since mangling keeps those keys distinct.

## Goal

- With the failing tree unchanged, `mfb build /tmp/bug648-minimal/dup` exits 0,
  `mfb build /tmp/bug648-minimal/app` exits 0, and `app.out` prints `export:x`, `8`,
  `export:a|public:3`.
- A consumer calling the `PUBLIC` overload (`dup::f(1, 2)`) is rejected with a located
  diagnostic (file, line, error code). No call form emits the unlocated
  `error: NIR call target 'dup.f' does not resolve`.
- The same holds for a return-type sibling (`EXPORT g(String) AS String` +
  `PUBLIC g(String) AS Integer`).
- Adding or removing a `PUBLIC` sibling does not change the package's export table:
  the exported symbol name and its ABI entry are identical with and without it.
- Both contrast rows keep working.

### Non-goals (must NOT change)

- **The `.mfp` wire format.** No new section, no new flag, no reordering. A package
  whose sources contain no such collision must produce a byte-identical `.mfp`.
- **Intra-package resolution.** `EXPORT` and `PUBLIC` both being visible inside the package
  (`src/resolver/mod.rs:visible_from` — `Visibility::Export | Visibility::Public => true`)
  is correct and stays. The package's own calls to both overloads keep resolving.
- **`PUBLIC` stays off the consumer-visible surface.** A consumer must not gain the ability
  to call a `PUBLIC` function.
- **Existing overload behavior for two `EXPORT`s of one name** (contrast row 3).
- **Forbidden:** rejecting the collision, or documenting "don't reuse an EXPORT name" —
  the spec makes the overload set legal. Renaming is the workaround applied in
  `049528d98`, not the fix.

## Blast Radius

Found by `grep -rn "imported_overloads" src/`, `grep -rn "read_package_exports\|\.exports()" src/`
and `grep -rn "fn is_exported_function\|FUNCTION_FLAG_PRIVATE" src/binary_repr/`.

- `src/monomorph/helpers.rs:collect_imported_overloads` — the `exports.len() < 2`
  early-continue. **Fixed by this bug** (or made unnecessary by a writer-side fix).
- `src/monomorph/lower.rs:Monomorphizer::new` (`function_overloads`) — the visibility-blind
  mangling decision. **Fixed by this bug** if the fix is taken here.
- `src/binary_repr/writer.rs:lower_function` + `src/binary_repr/sections.rs:is_exported_function`
  — the `FUNCTION_FLAG_PRIVATE` filter that strips the sibling. **Fixed by this bug** if
  the fix is taken on the writer side.
- `src/manifest/package.rs:external_package_function_types_from_files` — keys signatures by
  the possibly-mangled export name. **Latent, same hazard, not observed to fail** on its
  own: it is correct given a correctly-named export row. Out of scope unless the chosen fix
  changes what the writer emits.
- `src/resolver/packages.rs` (the `visible` set, ~line 144) — already tolerates both
  spellings: it inserts *both* `base` and the full `name` for a `$`-bearing export
  ("Monomorphization rewrites a call to an overloaded import to the mangled
  `base$signature` spelling … so accept either"). **Unaffected** — this is why the bug
  surfaces as a *typing* failure rather than an "unknown member" rejection, and it is the
  precedent for accepting both spellings.
- `src/ir/shape.rs` (~line 1004, `read_package_exports` for
  `validate_imported_function_signature`) — validates exported signatures' *types* only;
  never resolves a call. **Unaffected.**
- `src/binary_repr/writer.rs:external_function_metadata` — bare-name keying, see
  "Adjacent" above. **Latent, out of scope**: distinct keys today because of mangling.
- **The type-side analogue is already fixed**: bug-624 (`tests/runtime/rt_package_private_type_collision.rs`)
  fixed the same "a non-exported package symbol perturbs the consumer's view" hazard for
  TYPES. That fix does not cover functions; this is the function-side sibling and should
  cite it.
- **Other packages in the tree — audited, all clean.** For each of the ten packages under
  `packages/` (`cli`, `json_schema`, `jwt`, `libsnd`, `logger`, `mustache`, `sqlite3`,
  `timezones`, `xml`, `yaml`), the set of `^EXPORT (FUNC|SUB) <name>` names was intersected
  with the set of `^PUBLIC (FUNC|SUB) <name>` names via
  `comm -12` over `grep -rhoE` output across `packages/<p>/src`. Every intersection is
  empty, `packages/xml` included (`sliceText` since `049528d98`). **No package in the tree
  is currently broken by this bug** — but nothing prevents the next one, which is the point
  of the fix.

## Fix Design

The spec decides this; there is no open choice.

- **The collision is legal.** `mfb spec language functions` (Overloading): several `FUNC`s may
  share a name if their signatures differ, "Overloads may be declared across the files of one
  package", and two declarations collide only when name, parameter types and return type all
  match. Visibility is not part of a callable's identity. Rejecting the collision at package
  build (the former option (b)) would reject a legal program.
- **Importers see only EXPORTs.** `mfb spec language modules-and-packages`: `PUBLIC` is
  "hidden from importers"; the PUBLIC/EXPORT distinction matters "only for what is written
  into the compiled .mfp package (the exported-symbol flag)".
- **So an export's symbol must not depend on PUBLIC siblings.** The ABI index names each
  exported function by its concrete symbol (`src/binary_repr/sections.rs:from_project` —
  `AbiExport { name: function.name, … }`), and `mfb repo check-abi` treats a changed or
  dropped symbol as a breaking change. A consumer-side fix alone (the former option (a))
  would leave `f` ↔ `f$String` flipping whenever a hidden helper is added or removed.

**The fix:** in `src/monomorph/lower.rs:Monomorphizer::new`, an `EXPORT` function's concrete
name is decided from its `EXPORT` siblings only — parameter-overload mangling when two or more
EXPORTs share the name, return-type disambiguation when two or more EXPORTs share the
parameter types (built-in-named overrides stay force-mangled). Non-exported functions keep
the whole-set rule, so they are always mangled when any sibling exists and cannot collide
with an export's name. Intra-package calls resolve through `overload_names`, which maps each
declaration to whatever concrete name it was given, so they are unaffected.

Result: the single export is written bare (`f`), exactly as if the helper did not exist; the
consumer's existing bare-name path resolves it; `pk::f(1, 2)` type-checks against the only
visible `f` and is rejected at its call site. No `.mfp` byte changes for any package without
a collision.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/runtime/rt_package_public_export_name_collision.rs`, modelled on
      `tests/runtime/rt_package_private_type_collision.rs`: source and `.mfp` forms of the
      collision, the return-type sibling, a consumer calling the PUBLIC overload (located
      error), export-table identity with/without the PUBLIC sibling (`mfb pkg info`), and the
      two-EXPORT guard. Confirm each fails for the documented reason.
- [x] Audit every package in `packages/` for a `PUBLIC` name colliding with an `EXPORT`
      name. **Done: all ten packages clean** — see Blast Radius for the command.

Acceptance: the new tests fail for the documented reason and the guard passes.
Commit: 978e30225 (RED: 5 of 6 failed — `TYPE_UNKNOWN_VALUE`, unlocated
`NIR call target 'pk.f' does not resolve`, export table `f$String` vs `f`; guard passed)

### Phase 2 — the fix

- [x] Decide an `EXPORT` function's concrete name from its `EXPORT` siblings only
      (`src/monomorph/lower.rs:Monomorphizer::new`).
- [x] Re-run the Phase 1 tests and every contrast row.
- [x] **Found while verifying:** the importer's rejection of `pk::f(1, 2)` read
      "Call to `pk.f` has 1 argument(s), expected 1 to 1." — `src/ir/shape.rs` did not count
      an excess positional argument. Pre-existing (main's compiler, same output against the
      control package). Fixed; unit test
      `ir::shape::tests::excess_positional_arguments_are_counted_in_the_arity_detail`; one
      disproved golden line corrected
      (`tests/syntax/functions/user-function-default-args-invalid`, `combine(1, 2, "!", 4)`
      said "has 3").

Acceptance: Phase 1's tests pass; the guard unchanged; nothing in Non-goals moved.
Commit: 115d9b8c7 (export naming), 1d4638050 (arity count)

### Phase 3 — regenerate expected outputs + full validation

- [x] Rebuild every package in `packages/` and confirm each `.mfp` is byte-identical: all ten
      SAME (`shasum -a 256` of each `.mfp`, main's compiler vs. the fix).
- [x] Run the full suite plus the byte-identity and determinism gates.
- [x] Re-run the `/tmp/bug648-minimal` and `/tmp/bug648-callpublic` reproductions: both build;
      `app.out` prints `export:x` / `8` / `export:a|public:3`. The real-world case (scratch
      `packages/xml` with `sliceText` renamed back to `textOf`) exports `FUNC textOf` and the
      `/tmp/xml-lenrepro` consumer runs all four call shapes.
- [x] Update `src/docs/spec/architecture/12_monomorphization.md` (the parameter-overload
      producer row) and `src/docs/spec/language/06_functions.md` (Overloading) with the
      visibility rule.

Acceptance: full suite green; no `.mfp` deltas; the reproduction passes everywhere it failed.
Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_package_public_export_name_collision.rs`, built from
  source and from `.mfp`.
- Runtime proof: the consumer executable prints `export:x` / `8` / `export:y` /
  `export:a|public:3`.
- Loud-failure proof: a consumer call to the PUBLIC overload is a located error.
- ABI proof: `mfb pkg info` Exports identical with and without the PUBLIC sibling.
- Doc sync: `src/docs/spec/architecture/12_monomorphization.md`,
  `src/docs/spec/language/06_functions.md`.
- Full suite: `cargo test --no-fail-fast`, including the byte-identity and
  build-determinism gates.

## Summary

A hidden `PUBLIC` overload renamed its `EXPORT` sibling's symbol (`f` → `f$String`), and the
`.mfp` writer then dropped the only other row that would have let an importer map the name
back. The spec makes the overload set legal and PUBLIC invisible to importers, so the fix
makes an export's symbol depend only on its exported siblings. A package author previously got
a green build, a green test suite and a written `.mfp` while shipping an export nobody could
call.

Out of scope, noted for a separate plan: a required consumer-side doc-example gate per package
(`packages/xml/check-doc-examples.sh` is the only instrument that is a real consumer).
