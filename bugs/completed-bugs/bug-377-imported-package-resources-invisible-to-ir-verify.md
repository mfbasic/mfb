# bug-377: an imported package's resource types are invisible to `ir::verify` — every resource rule silently skips them in the consuming project

Last updated: 2026-07-22
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness (resource leak / double-close, silently unchecked)

Status: Fixed (2026-07-22)
Regression Test: `tests/imported_resource_scope_drop.rs` (4 tests) — the
imported-package sibling of `tests/native_resource_scope_drop.rs`. It consumes
the committed `sqlite3.mfp` the runtime fixture already ships, so it needs no
package build, install, or signature.

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

## Outcome (2026-07-22)

**Severity was understated: it was a leak AND a double free, and the write-up
found only one of the four defects.** `ir::verify`'s starved registry was real,
but three independent failures rode on the same "a decoded package carries no
`native_resources`" gap, each in a different layer. Fixing only the one named
here would have left the handle leaking.

**Phase 1 answer (leak vs double free): BOTH.** Codegen emitted *no* scope-drop
close for an imported resource — `RES music AS SoundFile = libsnd::openSound(…)`
produced zero `resource_cleanup` blocks, against a full close-and-reclaim
sequence for the built-in equivalent. So every imported handle leaked; and
because nothing was closed by the drop path, an explicit double close was a
genuine double free rather than a benign second call.

The four defects, in the order they had to be fixed:

1. **`code::validation` registered the close op unprefixed** (`<package>.<close>`)
   while `merge_packages` identity-prefixes every imported symbol, so
   `resource_cleanup_symbol`'s lookup missed and no cleanup was ever registered.
   This is the leak. `0aeffaf0d`
2. **`ir::verify`'s registry was starved** — the defect this document describes.
   Seeded from each imported package's `RESOURCE_TABLE`, spelled the way the
   *source* IR calls it (unprefixed), since verify runs before the merge.
   `d233d209a`
3. **The close thunk never set `RESOURCE_CLOSED_BIT`**, because the thunk's
   close-op set *also* came from the empty `native_resources`. Latent until (1)
   made the drop path fire; with it, `native-link-import-sqlite-rt` died in
   `libsqlite3` on a freed handle (`EXC_BAD_ACCESS` at `0x1`). `df583aca3`
4. **A re-exported close op did not resolve at all.** A package's `CLOSE BY` op
   reaches the importer under two spellings: the internal dotted target, which
   the merge identity-prefixes, and a re-exported `EXPORT FUNC close AS link::op`,
   which `merge_package` qualifies as `<package>.close` with *no* prefix. Only
   the first resolved, so sqlite3's `Db` — the shape plan-link-update.md §5a
   documents — still leaked after (1). `d360200a6`

(4) also settled a design point: the drop call site and the thunk's own "am I a
close op?" test must agree about which function is the closer, or the drop path
closes a handle whose thunk never set the flag. They now both resolve through
one `resolve_closer_symbol` and match on the resolved **symbol**, not a name.

The `explicit_type` half of the goal needed one more thing: `build` handed verify
empty external-signature maps, so an *inferred* `LET music = libsnd::openSound(…)`
lowered to a bind of unknown type and the RES axis could never fire. Handing over
*every* imported signature was tried first and is wrong — it tells verify a name's
type without that type's definition, and `LET result = thread::waitFor(t)` then
resolved to the imported union `ReturnChoice` whose variants are still absent, so
`check_match_exhaustive` read it as an *open* type and demanded a `CASE ELSE` from
an exhaustive match. The map is restricted to functions returning an imported
RESOURCE type, so signature and definition arrive together. `07219a948`

### Verified

- `LET music = libsnd::openSound(…)` (inferred) and
  `LET db = sqlite3::create(…)` → rejected with 2-203-0082.
- A double close of an imported resource → rejected with 2-203-0055
  `TYPE_USE_AFTER_MOVE` (see the limitation below).
- `RES db AS Db = sqlite3::create(…)` with no explicit close → one close and
  record reclaim at scope exit, where there were none.
- `examples/audio` builds and now emits 15 close call sites.
- `cargo test` green (3188 + all integration targets); full acceptance
  **1073 tests, zero mismatches**, twice.

### Known limitation — a wrapper-FUNC closer

Compile-time double-close detection keys on the call target being the
*registered* close op. That holds for the §5a re-export shape (sqlite3's
`EXPORT FUNC close AS sqliteLink::close`), which is why `sqlite3::close(db)`
twice is rejected. It does **not** hold when a package wraps its closer in an
ordinary exported function — libsnd's `closeSound`, which takes the `RES` and
calls `sndLink::closeFile` internally. From the importer's side that is an
ordinary call, and ordinary calls do not move ownership, so
`libsnd::closeSound(music)` twice still compiles.

It is no longer a double free: with (3), the second call reaches a thunk that
sees `RESOURCE_CLOSED_BIT` set and fails with a defined `ERR_RESOURCE_CLOSED`.
Making it a compile-time rejection would mean teaching the importer that a
wrapper *is* a close op, which needs a package-level declaration that does not
exist today. Worth a separate bug if the wrapper shape spreads.

## Summary

`ir::verify` cannot see that an imported package's type is a resource, so
every resource rule silently skips it in the consuming project — including
double-close detection. The rules are fine; the registry is starved. This is
upstream of bug-376 and unfixable by it: the annotated form, which bug-376
never gated, fails identically.

**Resolved 2026-07-22 — but the starved registry was one of four.** The same
"a decoded package carries no `native_resources`" gap starved three other
layers, and the one this document names was not the one causing the worst
symptom. Codegen emitted no scope-drop close at all, so every imported handle
leaked; and the close thunk never set `RESOURCE_CLOSED_BIT`, so the moment the
drop path started firing it became a double free into the C library. See
Outcome.
