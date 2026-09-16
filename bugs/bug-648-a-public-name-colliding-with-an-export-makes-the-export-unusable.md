# bug-648: a `PUBLIC FUNC` whose name collides with an `EXPORT FUNC` makes the EXPORT unusable from every consumer — and the package still builds green

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness / Footgun (silent at the package boundary)

Status: Open
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

**The single correct behavior a fix produces:** given a package with
`EXPORT FUNC f(x AS String) AS String` and `PUBLIC FUNC f(a AS Integer, b AS Integer) AS String`,
a consumer's `pkg::f("x")` compiles, links, and returns the `EXPORT` overload's result —
*or* `mfb build <package>` refuses the collision with a located diagnostic naming both
declarations. What must not remain possible is today's outcome: a successful package build
whose `.mfp` advertises an export no consumer can reach.

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

- With the failing tree unchanged, `mfb build /tmp/bug648-minimal/app` exits 0 and the
  resulting `app.out` prints `export:x`, `8`, `export:a|public:3` — *or* `mfb build
  /tmp/bug648-minimal/dup` exits non-zero with a located diagnostic naming both the
  `EXPORT` and the `PUBLIC` declaration of `f`. Silence at the package boundary followed by
  a consumer-side failure is not an acceptable outcome for either option.
- `mfb build /tmp/bug648-callpublic/app` never emits an unlocated
  `error: NIR call target 'dup.f' does not resolve`; whatever it reports carries a file, a
  line and an error code.
- Both contrast rows keep working, byte-for-byte where the artifact is unchanged.

### Non-goals (must NOT change)

- **The `.mfp` wire format.** No new section, no new flag, no reordering.
  `tests/cli/cli_build_determinism.rs` and the byte-identity gate
  (`.ai/testing-gates.md`) must stay green; a package whose sources contain no such
  collision must produce a byte-identical `.mfp`.
- **Intra-package resolution.** `EXPORT` and `PUBLIC` both being visible inside the package
  (`src/resolver/mod.rs:visible_from` — `Visibility::Export | Visibility::Public => true`)
  is correct and stays. The package's own `TESTING` cases that call both overloads by arity
  must keep passing.
- **`PUBLIC` must not become part of the package's consumer-visible surface.** Option (c)
  below is the one route that would change this; it is an open decision, not a licence.
  Whatever ships, a consumer must not gain the ability to call a `PUBLIC` function.
- **Existing overload behavior for two `EXPORT`s of one name.** Row 3 of the contrast table
  works today; a fix must not route it through a new path that changes which overload is
  selected, and must not start sending genuinely non-overloaded imports through overload
  matching.
- **The tempting wrong fix, forbidden:** "document that a `PUBLIC` name must not collide
  with an `EXPORT` name" and leaving the compiler silent. Renaming is the *workaround*
  already applied in `049528d98`; it is not the fix. Equally forbidden: changing
  `packages/xml` further, or weakening/removing the consumer-side repro so the broken path
  stops being exercised.

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
  of option (b).

## Fix Design

Three candidate shapes, in ascending order of blast radius. The correctness risk in all
three is the same: not perturbing row 3 of the contrast table, and not turning a
genuinely-unmangled single export into an overload lookup.

**(a) Consumer-side — drop the `exports.len() < 2` early-continue** in
`collect_imported_overloads`, so a *mangled* single export still registers its
`binding.base → binding.base$sig` rewrite. Smallest diff, `.mfp` format untouched, and it
keeps `PUBLIC` out of the consumer's surface. The guard rail: register the rewrite only when
the single row's name actually contains `$`; an unmangled single export must keep resolving
by its bare name, or every non-overloaded import starts paying overload matching.

**(b) Package-build-time rejection** — refuse a `PUBLIC` name that collides with an
`EXPORT` name, with a located diagnostic naming both declarations. Turns a silent
cross-boundary failure into a loud local one, which is the property the bug is really
about. Costs an error code and could break packages that build today.

**(c) Writer-side — emit the base name when a mangled export is the only surviving row
under its base.** Keeps the consumer untouched, but changes `.mfp` bytes for affected
packages and needs care where several `PUBLIC` siblings leave exactly one `EXPORT`.

(a) and (b) are not exclusive and combine well: (a) makes existing packages work, (b) stops
the footgun being re-armed. Rejected outright: exporting `PUBLIC` functions so both rows
land in the table — that would put package-internal helpers on the public surface, which
the Non-goals forbid.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/runtime/rt_package_public_export_name_collision.rs`, modelled on
      `tests/runtime/rt_package_private_type_collision.rs`: build the package from source
      *and* from its `.mfp`, then build and run a consumer. Cases: the failing collision
      (both call forms — `LET`-bound and plain, so both diagnostics are pinned), plus the
      two contrast rows as guards. Confirm the collision case fails with
      `TYPE_UNKNOWN_VALUE` / `NIR call target … does not resolve` and the guards pass.
- [x] Audit every package in `packages/` for a `PUBLIC` name colliding with an `EXPORT`
      name; record a verdict per package in the Blast Radius above. **Done: all ten
      packages clean** — see Blast Radius for the command.
- [ ] Decide the Open Decision below before Phase 2 — it changes which file Phase 2 touches.

Acceptance: the new test fails for the documented reason and the two guards pass; the
per-package audit is written into this file.
Commit: —

### Phase 2 — the fix

- [ ] Implement the chosen option from the Open Decision.
- [ ] If (a): in `src/monomorph/helpers.rs:collect_imported_overloads`, register the rewrite
      for a single `$`-bearing export; leave unmangled single exports on the bare-name path.
- [ ] If (b): emit a located diagnostic at package build; add the error code to
      `src/docs/spec/diagnostics/02_error-codes.md` (it is build input — see
      `.ai/specifications.md`) and run `cargo test errorcode`.
- [ ] Re-run the Phase 1 test and every contrast row.

Acceptance: Phase 1's collision case passes (or fails loudly at package build per (b));
both guards unchanged; nothing in Non-goals moved.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Rebuild every package in `packages/` and confirm each `.mfp` is byte-identical unless
      the chosen option intends otherwise; any diff is a bug-hunt trigger, not a rebaseline.
- [ ] Run the full suite plus the byte-identity and determinism gates
      (`tests/cli/cli_build_determinism.rs`, `.ai/testing-gates.md`).
- [ ] Re-run `/tmp/xml-lenrepro` against a `packages/xml` temporarily reverted to the
      colliding `textOf` name, proving the original real-world case is fixed — then discard
      that scratch revert without committing it.
- [ ] Update `src/docs/spec/architecture/12_monomorphization.md` where it describes
      `collect_imported_overloads`, and `.ai/resources-packages.md`, if the resolution rule
      changes.

Acceptance: full suite green; `.mfp` deltas are exactly the intended change; the
reproduction passes everywhere it previously failed.
Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_package_public_export_name_collision.rs`, built from
  source and from `.mfp` (the `.mfp` is what the consumer actually decodes — the lesson
  `rt_package_private_type_collision.rs` records in its own header).
- Runtime proof: the consumer executable runs and prints `export:x` / `8` /
  `export:a|public:3` — the same three lines both contrast rows produce today.
- Loud-failure proof: no unlocated `NIR call target …` output remains for any call form.
- Doc sync: `src/docs/spec/architecture/12_monomorphization.md` (it cites
  `collect_imported_overloads` twice) and `.ai/resources-packages.md`; plus
  `src/docs/spec/diagnostics/02_error-codes.md` only if option (b) adds a code.
- Full suite: the project's acceptance/CI command set, including the byte-identity and
  build-determinism gates.

## Open Decisions

- **Which fix shape ships** — recommended **(a) + (b)**: (a) so existing packages'
  exports become reachable without touching the wire format, (b) so the footgun cannot be
  re-armed silently. Alternatives: (a) alone (leaves the collision legal but silent — an
  author still gets no signal that a `PUBLIC` name is perturbing their public surface);
  (b) alone (loud, but breaks packages that build today and does nothing for an already-
  published `.mfp`); (c) writer-side base-name emission (keeps the consumer untouched at
  the cost of `.mfp` byte churn). Explicitly **not** on the table: exporting `PUBLIC`
  functions so the consumer resolves by arity the way the package's own scope does —
  that puts internals on the public surface, which the Non-goals forbid. **This is the
  owner's call and gates Phase 2.**
- **Should a package's doc-example harness become a required gate?**
  `packages/xml/check-doc-examples.sh` compiles each `DOC EXAMPLE` as its own project
  against the built `.mfp`, which makes it the only instrument in that package that is a
  real *consumer* — and per `049528d98` it would have caught this. Every package having
  one would close the whole class, independently of which fix ships.

## Summary

The engineering risk is concentrated in one line —
`collect_imported_overloads`'s `if exports.len() < 2 { continue; }` — and in the decision
about whether the collision should be legal at all. The `.mfp` already carries the
exported overload; nothing is missing from the artifact, and no wire-format change is
required by the recommended option. The real cost of this bug is not the broken call: it
is that a package author gets a green build, a green test suite and a written `.mfp` while
shipping an export nobody can call, and only a consumer ever finds out.
