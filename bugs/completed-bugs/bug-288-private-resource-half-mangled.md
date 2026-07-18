# bug-288: `PRIVATE RESOURCE` is accepted by the parser but half-mangled, making it unusable with cascading spurious errors

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness / Footgun

Status: Fixed 2026-07-18
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

## Resolution

Took the report's preferred option: `parse_top_level_resource` rejects `PRIVATE`
with a targeted `MFB_PARSE_UNEXPECTED_TOKEN` ("PRIVATE is not permitted on a
RESOURCE declaration") and synchronizes, instead of letting the half-applied
rename through.

Rejected rather than fully supported because spec 13's visibility-carrying items
are LET/MUT/FUNC/SUB/TYPE/UNION/ENUM — RESOURCE is not among them, so a
file-private resource type is not something the language offers.

**`EXPORT` is deliberately still accepted.** The report frames this as "RESOURCE
is not in the list of visibility-carrying items, so the parser should not accept
`PRIVATE` on it at all", which read literally would also forbid `EXPORT` — but
`EXPORT` is meaningful and already honored (`collect_native_resources` reports
`exported` from it, and the `native-resource-link-valid` fixture depends on it).
Only `PRIVATE` is rejected.

No new diagnostic code was added; the existing parse code carries the specific
message, matching how bug-292's fix was done.

Verified: the repro previously failed with `RESOURCE_CLOSE_SIGNATURE` naming a
mangled `#…$Db` plus repeated `SYMBOL_UNKNOWN_TYPE`, and now emits one diagnostic
with the caret on `PRIVATE`. Fixture:
`tests/syntax/resources/private-resource-invalid`. The 47 link/native fixtures and
74 resource/trap fixtures still pass, confirming `EXPORT`/`PUBLIC` are unaffected.
