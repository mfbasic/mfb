# bug-377: an imported package's resource types are invisible to `ir::verify` — every resource rule silently skips them in the consuming project

Last updated: 2026-07-21
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness (resource leak / double-close, silently unchecked)

Status: Open
Regression Test: none yet (needs an installed-package fixture; see Phases)

A package's exported `RESOURCE` type is **not** carried into the consuming
project's IR. `ir::verify` builds its resource registry from
`project.native_resources` (`src/ir/verify/mod.rs:703`), which holds only the
*current* project's `RESOURCE T CLOSE BY ...` declarations. An imported
package's resource type is therefore not in `resource_closers`, so
`close_op_for` returns `None`, so `is_resource_or_resource_union` returns
**false** — and every resource rule keyed on it silently does nothing.

The consuming project's IR knows the *type name* but not that it is a
resource. For `LET music = libsnd::openSound("x.ogg")` the bind op is:

```json
{ "op": "bind", "name": "music", "type": "SoundFile STATE FileInfo",
  "value": { "kind": "call", "type": "SoundFile STATE FileInfo",
             "target": "libsnd.openSound", ... } }
```

…while the same file's top level is `"types": []` with no native-resource
table. The type is right there; nothing says it is a resource.

Found 2026-07-21 while fixing bug-376. It is **not** part of bug-376 and is
not caused by it — see Attribution.

## Failing Reproduction

Needs a project with an installed package that exports a resource. Using
`examples/audio` (which depends on `jzaun#libsnd` 1.4.0, exporting
`SoundFile STATE FileInfo`):

```
cp -r examples/audio /tmp/audiotest
cat > /tmp/audiotest/src/main.mfb <<'EOF'
IMPORT libsnd

FUNC main AS Integer
  RES music AS SoundFile STATE FileInfo = libsnd::openSound("x.ogg")
  libsnd::closeSound(music)
  libsnd::closeSound(music)
  RETURN 0
END FUNC
EOF
cd /tmp/audiotest && mfb build .
```

- Observed: `Wrote executable to ./build/audio.out` — **exit 0, no
  diagnostic, for a double close.**
- Expected: a double-close diagnostic, as the same shape gets for a builtin
  resource (`tests/syntax/resources/ownership-resource-double-close-invalid`).

Second confirmed instance — the RES ownership axis:

```
LET music = libsnd::openSound("x.ogg")   -> compiles clean (exit 0)
```

Expected `2-203-0082 TYPE_RESOURCE_REQUIRES_RES` naming `music` and
`SoundFile`. With a *builtin* resource the identical shape is now correctly
rejected (bug-376); with an imported one it is not.

Both reproduce with and without bug-376's fix.

## Attribution — why this is not bug-376

bug-376's goal list included:

> `LET music = libsnd::openSound(path)` reports at the `LET`, not as a
> downstream `TYPE_UNKNOWN_VALUE` on `.state`.

That bullet is **not met by bug-376's fix, and cannot be** — it was
mis-attributed. bug-376 was the `explicit_type` gate on the RES-axis check.
Proof that this is a different defect:

- The **annotated** form `LET music AS SoundFile = libsnd::openSound(...)`
  fails identically (no 2-203-0082, only the displaced
  `TYPE_UNKNOWN_VALUE`). That path carries `explicit_type: true`, so it took
  the *same* code path before and after bug-376's change. bug-376 cannot have
  affected it, and did not fix it.
- The failure is not gate-shaped at all: `is_resource_or_resource_union`
  returns false, so the rule's precondition is never satisfied regardless of
  any gate.

bug-376 is genuinely fixed for builtin and same-project resources; that fix
is verified by `resource-let-binding-inferred-invalid` and
`resource-let-binding-wrapper-invalid`.

## Root Cause

`src/ir/verify/mod.rs:703`:

```rust
let resource_closers = project
    .native_resources
    .iter()
    .map(|r| (r.name.clone(), r.close_function.clone()))
    .collect();
```

`project` here is the consuming project's `IrProject`. Its
`native_resources` never includes an imported package's exported resources,
and its `types` list is empty for imported types. `close_op_for` falls back
only to `builtins::resource::builtin_resource_close_function`, which covers
`File`/`TlsSocket`/etc. but nothing from a package.

So the fix is upstream of the rules: the consuming project's IR must carry
the imported packages' resource declarations (name + close function), or
`ir::verify` must be given access to the resolved package interfaces.

## Blast Radius

Everything keyed on `is_resource_or_resource_union` / `close_op_for` is
inert for imported resources. Confirmed by reproduction:

- `TYPE_RESOURCE_REQUIRES_RES` (the RES ownership axis) — confirmed silent.
- Double-close / use-after-close tracking — confirmed silent.

Unverified but keyed on the same predicate, so presumed affected (**must be
enumerated in Phase 1, not assumed**):

- scope-drop close obligations / leak tracking
- `TYPE_RESOURCE_ELEMENT` and collection-of-resource rules
- record-field-holds-resource rejection
- STATE agreement checks

The runtime consequence is the serious part: a consuming project can double
close or leak an imported package's handle with **no diagnostic at any
layer**. Whether codegen also omits scope-drop closes for imported resources
is **unknown and must be settled first** — if it does, this is a silent leak
in every program using a package resource; if it does not, the double-close
is a genuine double-free.

## Goal

- An imported package's resource type is recognized as a resource by
  `ir::verify` in the consuming project, so every existing resource rule
  applies to it exactly as it does to a builtin resource.
- `LET music = libsnd::openSound(...)` is rejected with 2-203-0082 at the
  `LET`.
- A double close of an imported resource is rejected.

### Non-goals

- Changing any resource *rule*. The rules are correct; they are starved of
  the fact that the type is a resource. Do not special-case imported types
  inside individual rules.
- bug-376's `explicit_type` ungating — already landed and independent.

## Phases

### Phase 1 — settle severity and scope (no behavior change)

- [ ] Determine whether codegen emits scope-drop closes for imported
      resources. This decides leak-vs-double-free and sets the real severity.
- [ ] Enumerate every rule keyed on `is_resource_or_resource_union` /
      `close_op_for` and confirm per rule whether it is inert for imported
      resources.
- [ ] Build an acceptance fixture that consumes a package exporting a
      resource. Check whether the harness can install/vendor a package
      hermetically (see memory `acceptance-golden-harness-mechanics` on
      registry fixtures needing a hermetic `MFB_HOME`); if not, that
      machinery is a prerequisite, not an afterthought.

### Phase 2 — carry package resources into the consuming IR

- [ ] Populate the consuming project's resource registry from the resolved
      package interfaces (name + close function).
- [ ] Version-guard any IR binary-format change and prove no `.ncode`
      golden shifts.

### Phase 3 — validation

- [ ] New fixtures green; `cargo test` + full acceptance green.
- [ ] Re-run both reproductions above.

## Summary

`ir::verify` cannot see that an imported package's type is a resource, so
every resource rule silently skips it in the consuming project — including
double-close detection. The rules are fine; the registry is starved. This is
upstream of bug-376 and unfixable by it: the annotated form, which bug-376
never gated, fails identically.
