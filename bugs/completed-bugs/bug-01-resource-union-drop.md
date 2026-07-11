# bug-01 — Resource-union drop dispatch is non-deterministic

Last updated: 2026-06-29

`TypeModel::variants_for_union` iterates a `HashMap`, so the order it yields a
union's variants is **non-deterministic across process runs**. The order leaks
into native codegen: the resource-union tag-dispatch drop loop
(`emit_resource_union_cleanup_call`) emits its per-variant tag checks in that
order, so the same source compiled twice produces **different `.ncode`/binaries**
— the variant checks (`cmp_imm rhs 0/1`, `bl _mfb_rt_fs_fs_close` vs
`_mfb_rt_net_net_close`) come out swapped.

A correct implementation makes codegen a pure function of the program:
`variants_for_union` yields variants in a fixed canonical order (their
declaration/tag order), so two builds of the same source are byte-identical.

Found 2026-06-29 by `scripts/codegen-selfdiff.sh` during plan-00-C: the one
self-diff "failure" (`tests/resource-union-drop-valid`) reproduces with two
`-codegen direct` builds, so it is **not** a MIR regression — it is this
pre-existing non-determinism.

It complements:

- `./mfb spec memory thread-resource-plane` / `mfb spec language` (union layout
  and resource drop; canonical specs under `src/spec/**`) — no spec change is
  expected (this is a determinism fix, not a semantic one).

## 1. Goal

- `TypeModel::variants_for_union` returns a union's variants in a **deterministic
  canonical order** — their union-declaration order, i.e. ascending
  `union_variant_tags` index — on every run and every host.
- Native codegen for any union-using program is byte-identical build-to-build:
  `scripts/codegen-selfdiff.sh` reports `failures=0` (incl.
  `resource-union-drop-valid`), and two `-codegen direct` builds of the same
  fixture `diff`-clean.

### Non-goals (explicit constraints)

- No change to union **layout**, tag values, payload sizing, ABI, or
  thread-transfer rules. Tags stay the declaration index
  (`validation.rs:240`); only the *iteration order* of a lookup is pinned.
- No change to the resource-drop **semantics** (each variant is still closed by
  its own tag-matched close op; order between independent variants is
  observationally irrelevant — only the emitted instruction order changes).
- No language-surface change.

## 2. Current State

- `union_variant_unions: HashMap<String, HashSet<String>>` is built in
  `TypeModel::from_module` (`src/target/shared/code/validation.rs:207`,
  populated at :236), and `union_variant_tags: HashMap<String, usize>` stores
  each variant's **declaration index** (`validation.rs:240`).
- `variants_for_union` (`src/target/shared/code/validation.rs:378`) yields
  variants by iterating `union_variant_unions.iter().filter(...)` — **HashMap
  order**, hence non-deterministic.
- Consumers of that order:
  - `emit_resource_union_cleanup_call` via `resource_union_cleanup`
    (`builder_codegen_primitives.rs:919`) — the drop dispatch loop (**the
    observed symptom**).
  - `builder_collection_layout.rs:14`, `:430`, `:529` — union layout queries.
  - `builder_arena_transfer.rs:667` — thread-transfer variant walk.
  - `builder_values.rs:853`, `:960` — union construction / payload sizing.
  - Layout/sizing consumers are max/contains reductions (order-independent for
    their *result*), but they still must be audited to confirm no offset or
    emitted-instruction order depends on iteration order.

## 3. Design Overview

Pin the order at the single source — `variants_for_union` — so every consumer
inherits determinism without per-call-site changes. Collect the matching
variant names and sort by `self.union_variant_tags[name]` (the declaration
index), falling back to a name sort if a tag is somehow absent. This restores
the union's natural declaration order, which is almost certainly the order the
layout code already assumes (tag N ↔ index N), minimizing churn.

The correctness risk is entirely in the **golden/byte-identity audit**: any
union-using fixture whose current arbitrary order differs from tag order will
have its `.ncode`/`.mir`/binary change once, and those goldens must be
regenerated and confirmed to differ *only* by variant reordering (no offset or
size change).

## 4. Detailed Design

Change `variants_for_union` to return an ordered iterator/`Vec` sorted by tag:

```rust
pub(super) fn variants_for_union<'a>(&'a self, union: &'a str) -> impl Iterator<Item = &'a String> + 'a {
    let mut variants: Vec<&String> = self
        .union_variant_unions
        .iter()
        .filter(move |(_, unions)| unions.contains(union))
        .map(|(variant, _)| variant)
        .collect();
    variants.sort_by_key(|variant| {
        (self.union_variant_tags.get(*variant).copied().unwrap_or(usize::MAX), *variant)
    });
    variants.into_iter()
}
```

(Final form may return `Vec` if the borrow/lifetime on the closure is awkward;
behavior — tag-ascending, name-tiebreak — is what matters.)

## Layout / ABI Impact

None intended. Union tags, payload layout, and the `{tag, …}` shapes in
`mfb spec memory` are unchanged. The only observable change is the **order** of
already-present per-variant instructions in the drop/transfer/construct paths.
The Layout/ABI audit (Phase 2) is what proves this.

## Phases

1. **Test-first repro.** Add a determinism check that builds
   `resource-union-drop-valid` twice and asserts identical `.ncode` (or wire it
   into `codegen-selfdiff` expectations). Confirm it fails on `main`.
2. **Fix + audit.** Sort `variants_for_union` by tag; audit the six consumers to
   confirm none derive an offset/size from iteration order (layout reductions
   are order-free; the drop/transfer/construct loops only reorder emitted ops).
3. **Regenerate + verify goldens.** Regenerate any `.ncode`/`.mir`/binary
   goldens that change; diff each to confirm the change is **only** variant
   reordering. Re-run `codegen-selfdiff.sh` → `failures=0`.

## Validation Plan

- Determinism proof: two builds of `resource-union-drop-valid` produce identical
  `.ncode`; `scripts/codegen-selfdiff.sh` reports `failures=0` across the suite.
- Function tests: existing union/resource fixtures still pass; add a runtime
  proof that the union still drops the active variant's resource (observable via
  the existing close-call behavior).
- Doc sync: none expected (no spec/diagnostic change); note the determinism
  guarantee if `mfb spec` documents codegen reproducibility.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  (regenerate only the union-affected goldens, verified reorder-only).

## Open Decisions

- Canonical order — **by declaration/tag index** (recommended; natural, matches
  layout's tag↔index assumption) vs. alphabetical by variant name (also
  deterministic but reorders against tags). (§3)

## Summary

A one-function determinism fix at `variants_for_union`; the engineering is the
audit that pinning the order shifts only instruction *order* (never layout or
size) and the one-time golden regeneration. Captured in memory as
`union-drop-codegen-nondeterminism`.
