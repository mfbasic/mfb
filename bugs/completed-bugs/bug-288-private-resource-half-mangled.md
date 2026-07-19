# bug-288: `PRIVATE RESOURCE` is accepted by the parser but half-mangled, making it unusable with cascading spurious errors

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness / Footgun

Status: Fixed
Regression Test: tests/rt-behavior/native/native-private-resource-rt — `PRIVATE RESOURCE` builds, links and runs

The parser accepts `PRIVATE` on a `RESOURCE` declaration, but `scope_privates`
handles it inconsistently: the resolver registers a RESOURCE name as a *type* (it
appears in type positions like `AS RES Db`, `RES db AS Db`, LINK signatures), yet
`scope_privates` renames the declaration without adding it to `private_types` (so no
type-string reference is rewritten) and never rewrites LINK-block contents or the
resource's own `CLOSE BY` reference. The result is a mangled declaration name that
no reference resolves to — a wall of `SYMBOL_UNKNOWN_TYPE` plus
`RESOURCE_CLOSE_SIGNATURE` errors. Per spec, RESOURCE is not in the list of
visibility-carrying items, so the parser should not accept `PRIVATE` on it at all.

The single correct behavior a fix produces: `PRIVATE RESOURCE` is either rejected
with a clear diagnostic (spec-conformant, simplest) or fully supported (type-string
+ LINK + CLOSE BY rewrites) — not silently half-applied.

References:

- `src/docs/spec/language/13_modules-and-packages.md:31` (visibility-carrying items:
  LET/MUT/FUNC/SUB/TYPE/UNION/ENUM — RESOURCE not listed).
- Found during goal-06 review of `src/scope_privates.rs`.

## Failing Reproduction

Take the in-tree sqlite fixture and prefix both `RESOURCE` decls with `PRIVATE`:

- Observed: build fails with `RESOURCE_CLOSE_SIGNATURE` naming the mangled
  `#c2515768…$Db`, plus repeated `SYMBOL_UNKNOWN_TYPE: Type 'Db' is not …`.
- Expected: a single targeted diagnostic ("PRIVATE is not permitted on RESOURCE"),
  or a clean build with the resource actually private.

## Root Cause

`src/scope_privates.rs:114-123` (`item_name_vis` — `Resource` returns
`is_type: false`) and `:204` (`Item::Resource`/`Item::Link` skipped by
`rewrite_item_refs`): the resource name is renamed but not treated as a private
type, so type-position references, LINK-block type strings, and the `CLOSE BY`
reference are never rewritten to the mangled name.

## Goal

- Preferred: syntaxcheck rejects `PRIVATE` on `RESOURCE` (and, if desired, on other
  non-visibility items) with a targeted diagnostic.
- Alternative (full support): `item_name_vis` returns `is_type: true` for Resource
  and `rewrite_item_refs` rewrites the close function and LINK-block type strings.

### Non-goals (must NOT change)

- Non-private RESOURCE behavior.
- The private mangling of the genuinely visibility-carrying items.

## Blast Radius

- `scope_privates.rs` (`item_name_vis`, `rewrite_item_refs`) — fixed here.
- `Item::Link` is skipped by the same rewrite; if full support is chosen, LINK type
  strings referencing a private resource need rewriting too — audit during fix.

## Fix Design

Recommend the spec-conformant rejection: add a syntaxcheck diagnostic for `PRIVATE`
on RESOURCE (cheap, removes a broken feature). Full support is larger (type-string
+ LINK + CLOSE BY rewrites) and only worth it if private resources are a desired
feature. Rejected alternative: leaving it as-is — the current half-mangling is a
footgun that produces confusing errors.

## Phases

### Phase 1 — failing test
- [ ] Test that `PRIVATE RESOURCE` currently produces the cascading errors.
### Phase 2 — the fix
- [ ] Add the rejection diagnostic (or full rewrite support).
### Phase 3 — validation
- [ ] Full suite green.

## Validation Plan

- Regression: `PRIVATE RESOURCE` yields the targeted diagnostic (or compiles+runs).
- Doc sync: confirm 13_modules-and-packages.md's visibility list matches the
  enforced behavior.

## Summary

A parser/scope mismatch leaves `PRIVATE RESOURCE` broken; the low-risk fix is to
reject it per spec. Full support is possible but larger and optional.

## Attempted fix, reverted 2026-07-18 — read this before trying again

The report's **preferred option (reject `PRIVATE` at parse time) is wrong.** It was
implemented, and reverted, because it deletes a feature the compiler deliberately
models.

`src/ir/lower.rs:638` maps resource visibility across all three variants:

```rust
visibility: match resource.visibility {
    Visibility::Export => "export",
    Visibility::Public => "public",
    Visibility::Private => "private",
}
```

and two tests assert that arm on purpose:

- `ir::tests::lower_pipeline_tests::lowers_native_link_functions_resources_and_aliases`
  — its fixture comment says it exists to cover "arms of `native_resources`'
  visibility mapping", pairing `PUBLIC RESOURCE Db` with `PRIVATE RESOURCE Cache`.
- the `demoLink` pipeline test asserts
  `r.name == "Stmt" && r.visibility == "private"`.

`src/scope_privates.rs`'s own `BROAD` fixture also declares
`PRIVATE RESOURCE Handle` and asserts it is mangled.

So rejecting the spelling makes `visibility == "private"` unreachable and requires
gutting three tests that cover it intentionally. The report's basis — that spec 13
lists only LET/MUT/FUNC/SUB/TYPE/UNION/ENUM as visibility-carrying — does not
account for RESOURCE visibility being modelled and lowered downstream. Either the
spec list is incomplete or the IR support is vestigial, and **that question has to
be settled before this bug can be fixed.**

If the answer is "private resources are real", the fix is the report's
*alternative*: make `scope_privates` fully support them (register the name in
`private_types`, and rewrite type-string references, LINK block contents, and the
resource's own `CLOSE BY` target) — or, more cheaply, stop renaming resource
declarations altogether so the name stays consistent and `PRIVATE` carries only
visibility. The latter needs a check that two files declaring the same private
resource name cannot collide.

What is definitely still true is the reported defect: today the declaration is
renamed while every reference is not, which is a guaranteed build failure. Nothing
about that has changed.

## Resolution — the *alternative* was taken: private resources are fully supported

The question the reverted attempt left open ("either the spec list is incomplete or
the IR support is vestigial, and that has to be settled first") is settled in favour
of **the feature being real**. The evidence is what the previous note already
assembled and did not act on: `ir::lower` maps `Visibility::Private` for resources,
two IR pipeline tests cover that arm on purpose, and `scope_privates`' own `BROAD`
fixture declares `PRIVATE RESOURCE Handle`. Nothing in the tree treats the support
as dead. So the spec's visibility list in `13_modules-and-packages.md` is what is
incomplete, and rejecting the spelling would have deleted a working feature to
satisfy a stale list.

The report's root-cause analysis was correct as far as it went, but the rename had
to reach **five** positions, not the two it named. Each was found by building the
report's own reproduction and reading the next error:

1. `item_name_vis` now reports `is_type: true` for `Item::Resource` — a RESOURCE
   name is registered by the resolver as a type, so this is what made any
   type-position reference get rewritten at all.
2. `rewrite_item_refs` gained an `Item::Resource` arm rewriting `CLOSE BY`, which
   names a *function* and so follows `rename`, not the type map.
3. …and an `Item::Link` arm rewriting LINK signature types: parameter types, return
   types, return STATE types, and each `CSTRUCT`'s `maps_to`. (The C-side `CSTRUCT`
   name is local to the block and not nameable by ordinary code, so it is left
   alone.)
4. `Statement::Let`'s `state_type` — the STATE clause of a local `RES x AS T STATE S`
   binding was the one type position the statement rewriter skipped. This one does
   not surface as an unresolved name but as `TYPE_STATE_MISMATCH`, which is why the
   original report did not attribute it here.
5. `Param::state_type`, in both ordinary functions and LINK blocks, for the same
   reason.

A sixth layer sat outside `scope_privates` entirely: `plan::lower::is_user_type_name`
tests that a type name is purely alphanumeric, which a mangled `#<hash>$Db` is not.
A private *record* survives this because it is found in `type_storage` first, but a
private *resource* has no storage entry and falls through to the predicate — so the
build failed with `native plan has no storage class for type '#…$Db'` even after the
AST was fully consistent. The predicate now strips the internal sigil and file-hash
prefix before testing the remainder.

### Verification

The regression test is the report's exact reproduction — the sqlite fixture with
`PRIVATE` on the RESOURCE — promoted to
`tests/rt-behavior/native/native-private-resource-rt`. It is an rt-behavior test
because compiling was never the whole claim: it links against real sqlite3, opens an
in-memory database through the private resource, writes and reads both a numeric and
a String STATE field, and prints. The golden IR records `visibility: "private"` and
six mangled references, so the very arm the reverted fix would have made unreachable
is now covered end to end by an executing test.

Full `cargo test` green — including the two IR tests and the `scope_privates` test
the previous attempt would have had to gut. Acceptance 1001/1001.
