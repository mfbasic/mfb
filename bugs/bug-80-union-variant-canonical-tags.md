# bug-80: a variant included at divergent positions in two unions is now rejected — support it with canonical tag assignment

Last updated: 2026-07-10
Effort: large (3h–1d)
Severity: MEDIUM (a legal-looking program is rejected)

`union_variant_tags` is keyed by variant **name** alone. The resolver permits the
same variant to appear at different positions in two different unions:

```basic
UNION A INCLUDES Base
UNION C INCLUDES Other, Base
```

Last-wins keying then collided two variants onto one tag, and `LET x AS A = W1[42]`
printed `V1:42` — the wrong variant. That was a silent miscompile.

bug-69 (commit 1b0572d4) closed the hole the only way it safely could within its
scope: `check_union_variant_tag` now **rejects** a variant whose tag would differ
across unions. A silent wrong answer became a clear compile error.

This bug tracks doing it properly: **accept** the program and give it correct code.

## Why the obvious fix is wrong

Per-union keying (`(union, variant) -> tag`) does not work. A single global tag per
variant is load-bearing for **union subtyping**: a value of type `A` must be
assignable where a `C` is expected, and the discriminant has to mean the same thing
in both, without knowing the static union at the read site. Per-union tags make the
discriminant context-dependent and break every widening.

## Goal

- A variant included at divergent positions in two unions compiles, and every
  discriminant read resolves to the right variant through any widening.
- The bug-69 rejection diagnostic is removed once the general case works.

## Fix Design

Assign a **globally canonical tag per variant type**, once, across the whole merged
program, independent of any union's declaration order:

- Collect every variant type reachable in the merged IR.
- Assign tags from a deterministic ordering (a stable sort of the fully-qualified
  variant type name, not declaration order, so the tag is reproducible across
  separate compilations and package boundaries).
- Each union's match/dispatch then tests the canonical tag, not a positional index.
- Verify the `.mfp` package format carries the canonical tag, so a variant crossing
  a package boundary keeps its identity. This is the part most likely to force a
  format change — check `src/target/package_mfp/` and the binary-repr version.

Watch: resource-union drop dispatch already iterates a `HashMap` and is
order-nondeterministic (memory note `union-drop-codegen-nondeterminism`). Canonical
tags are the natural fix for that too — do both together.

## Blast Radius

- `src/target/shared/code/validation.rs` (the bug-69 rejection), the union tag
  assignment, match/dispatch lowering, resource-union drop dispatch, `.mfp` encoding.
- Native goldens for every program using a union will shift if the tag numbering
  changes. Expect a large, mechanical golden regeneration.

## Phases

### Phase 1 — failing test

- [ ] The divergent-position program above; confirm it is rejected today with
      bug-69's diagnostic, and that removing the check restores the miscompile.

### Phase 2 — canonical assignment

- [ ] Global, deterministic tag per variant type; thread through match/dispatch.
- [ ] Carry the tag in `.mfp`; bump the binary-repr version if the layout changes.

### Phase 3 — remove the guard

- [ ] Delete `check_union_variant_tag`; keep an assertion that no two variants share
      a tag.

### Phase 4 — validation

- [ ] The divergent-position program runs correctly, including through a widening.
- [ ] Resource-union drop dispatch is deterministic (fixes the golden flake).
- [ ] Cross-package variant identity holds.
- [ ] `scripts/test-accept.sh`.

## Summary

Union variant tags are keyed by name and assigned by declaration order, so the same
variant in two differently-ordered unions collides. bug-69 turned the resulting
miscompile into a rejection; a global canonical tag per variant type is what makes
the program compile and run — and would also make resource-union drop dispatch
deterministic.
