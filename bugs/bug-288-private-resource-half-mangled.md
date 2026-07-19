# bug-288: `PRIVATE RESOURCE` is accepted by the parser but half-mangled, making it unusable with cascading spurious errors

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness / Footgun

Status: Open
Regression Test: tests/ (new) — `PRIVATE RESOURCE` is rejected with a targeted diagnostic (or fully supported)

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
