# bug-69: four latent codegen soundness gaps that are safe only because an invariant holds elsewhere — FMA fusion, union-variant tag keying, peephole store-to-load overlap, and the RISC-V stream sniffer

Last updated: 2026-07-09
Effort: small (<1h)

A cluster of LOW / defense-in-depth codegen defects, each currently *unreachable* because a
non-local invariant holds — but each would silently miscompile if that invariant ever
changed. Batched because the right fix for all four is the same shape: add the guard (or a
debug assertion) that makes the code correct-by-construction instead of correct-by-coincidence.

**(1) FMA fusion doesn't guard product redefinition.** `fma_fusion.rs:redefined_between`
(`:104-111`) scans the span between the `fmul_d` and its consumer for redefinitions of the
multiply *operands* `a`/`b`, but never of the *product* `%p`. `use_counts` counts only reads
of `%p`. So `fmul_d %p,a,b ; <redefine %p> ; fadd_d %w,%p,c` would fold to
`fmadd_d %w,c,a,b`, dropping the intervening redefinition. Safe today only because
`emit_float_binary` (`builder_numeric.rs:990`) allocates a fresh `allocate_fp_register()`
product for every multiply, so `%p` is always single-def.

**(2) Union-variant tag/field maps keyed by variant name alone.** `types.rs:from_module`
(`:240-248`) and `add_package_type_export` (`:366-374`) store
`union_variant_tags: HashMap<variant_name, index>`. If one variant type belonged to two
unions at *different* expanded positions, the last `module.types` iteration to touch it wins
and the other union's construction tag / drop-dispatch order is taken from a foreign
position. Safe today only because `expanded_nir_union_variants` (`function_lowering.rs:14-19`)
prepends `INCLUDES` sets from index 0, keeping a variant's position stable across including
unions — and possibly because the type checker forbids a variant in two positionally-divergent
unions (unconfirmed).

**(3) Peephole store-to-load forwarding assumes non-overlapping 8-byte sp slots.**
`peephole.rs:forward_stores_to_loads`/`classify` (StoreSp arm, ~`:61-68`, `:152-200`) key
slots by exact offset-string equality and invalidate a slot only when its *source register*
is redefined — never when a *different* offset partially overwrites its byte range. So
`str x10,[sp,#8] ; str x11,[sp,#12] ; ldr x8,[sp,#8]` would forward `x10` even though bytes
12..16 were clobbered. Safe today only because every sp stack object is 8-byte-granular and
full-slot-aligned (`allocate_stack_object(_, 8)`), and sub-word/`StrD` stores flush the whole
map.

**(4) `stream_is_riscv` sniffs register names out of arbitrary field values.**
`regalloc/analysis.rs:stream_is_riscv` (`:215-222`) returns true if *any* field value equals
a RISC-V ABI name (`a0`, `t0`, `s1`, `ra`, …), scanning label/symbol strings too, not just
register-operand fields. A non-RISC-V stream carrying such a string in a non-register field
would select the wrong caller-saved clobber mask, and the FP-shuttle peephole could drop a
live shuttle. Safe today only because current codegen labels/symbols carry prefixes
(`loop_*`, `_mfb_*`) and no bare `a0`/`s0` operand appears in a non-register field.

The single correct behavior a fix produces: each of the four is correct by its own guard, so
a future change elsewhere (a reused product vreg, a variant in two divergent unions, a packed
sub-slot frame object, a bare-register-named label) cannot silently miscompile.

References (all under `src/target/shared/code/`, except (4)):

- (1) `fma_fusion.rs:104-111` (`redefined_between`), `:88-95` (consumer find); product
  freshness at `builder_numeric.rs:990`.
- (2) `types.rs:240-248`, `:366-374`; `variants_for_union` `:389-409`;
  `expanded_nir_union_variants` `function_lowering.rs:14-19`.
- (3) `peephole.rs:61-68`, `:152-200`; frame granularity via `allocate_stack_object(_, 8)`.
- (4) `regalloc/analysis.rs:215-222` (`stream_is_riscv`); `effect`/liveness use
  DEF_FIELDS/USE_FIELDS and are immune; ISA is `s11`-arena-base elsewhere.
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

None is reachable today (that is the point). Each is established by inspection + the invariant
that currently protects it:

- (1) would fire if a lowering fed a named/reassignable vreg as an `fmul_d` destination.
- (2) would fire if a variant type were a direct member of two unions with divergent expanded
  orders.
- (3) would fire if the frame allocator ever produced overlapping / unaligned sp stores.
- (4) would fire if a label/symbol were literally named `a0`/`s0`/`ra` on a non-RISC-V target.

- Observed: none (invariant holds).
- Expected: the code remains correct even if the invariant is later relaxed.

Contrast: the sibling correct-by-construction paths — the `a`/`b` redefinition guard already
present in (1); `effect()`/liveness restricting to register-operand fields in (4); the
`branch_le` full-slot stores that make (3)'s equality keying sound today.

## Root Cause

Each site trusts a non-local invariant without asserting it: (1) product single-def, (2)
variant-position stability, (3) full-slot non-overlapping sp stores, (4) register names appear
only in register-operand fields.

## Goal

- Each site enforces its invariant locally (a guard or a debug assertion), so a future change
  elsewhere converts a silent miscompile into a compile-time failure or a correct result.

### Non-goals (must NOT change)

- Current codegen output (all four are no-ops under today's invariants — expect byte-identical
  goldens).
- The invariants themselves (product freshness, `INCLUDES` prefix ordering, 8-byte frame
  granularity, label prefixes).

## Blast Radius

- Each `file:symbol` above — independent; can land as one commit or four.

## Fix Design

- (1) In the `i+1..j` scan, also bail if any instruction redefines `product` (add
  `|| v == &product` to the def-field predicate at `fma_fusion.rs:107`).
- (2) Key `union_variant_tags`/`union_variant_fields` by `(union_name, variant_name)` and have
  `variants_for_union` look up the per-union tag — or, if the type checker forbids
  positionally-divergent membership, add a debug assertion that re-inserting a variant name
  never changes its index.
- (3) Invalidate any recorded slot whose `[offset, offset+8)` range intersects an incoming
  store, not just an exact-offset match. (Only needed if the frame model ever allows
  overlapping/unaligned sp stores; otherwise a debug assertion of full-slot alignment.)
- (4) Restrict the scan to DEF_FIELDS/USE_FIELDS (as `effect` does), or thread the ISA in
  explicitly from the caller instead of sniffing operand strings.

Recommend the cheapest form for each: guards for (1) and (4) (small, no downside), debug
assertions for (2) and (3) unless the language already permits the triggering shape.

## Phases

### Phase 1 — confirm the invariants

- [x] Confirm the type checker's stance on a variant in two divergent unions (drives (2):
      real fix vs assertion) and the frame model's sp-store granularity (drives (3)).
      **Finding:** the resolver forbids diamonds / duplicate variants
      (`TYPE_DUPLICATE_VARIANT`) but *permits* a variant at divergent positions across
      two unions (`UNION A INCLUDES Base` vs `UNION C INCLUDES Other, Base`). This is
      **not** latent — it is a reachable, demonstrated miscompile (see Resolution).
      Frame model confirmed 8-byte-granular / 8-aligned sp objects, so (3) is latent.

### Phase 2 — add the guards/assertions

- [x] Land the four guards/assertions.

### Phase 3 — validation

- [x] Goldens byte-identical (all four no-ops under today's valid programs);
      2474 unit tests green including the new regressions. `artifact-gate.sh` /
      `test-accept.sh` / self-diff gate are run by the orchestrator.

## Validation Plan

- Regression test(s): where a real fix (not just an assertion) lands — a synthetic stream
  exercising the guard.
- Runtime proof: none needed (no behavior change today).
- Doc sync: none expected.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

Four codegen sites are correct only because an invariant holds somewhere else: FMA product
freshness, union-variant position stability, full-slot sp stores, and register-name locality.
Each is a latent miscompile waiting for that invariant to shift. The fix is to make each
correct-by-construction with a local guard or assertion; today's output is unchanged.

## Resolution

Landed on `main`. Each site now enforces its invariant locally; all four are byte-identical
for programs valid today.

**(1) FMA product-redefinition guard** — `fma_fusion.rs`. Added `|| v == &product` to the
`redefined_between` predicate so a redefinition of `%p` between the `fmul_d` and its consumer
aborts the fusion (`use_counts` cannot see a def, only reads). No-op today (the product is a
fresh single-def vreg). Regression test `does_not_fuse_when_product_redefined`.

**(2) Union-variant tag stability — REAL miscompile, not latent.** Phase 1 disproved the
assumed invariant: the resolver accepts a variant at divergent positions in two unions, and
the last-wins global `union_variant_tags[variant]` then collides two variants onto one tag
within a union. Demonstrated: `LET x AS A = W1[42]` printed `V1:42` (matched the wrong
variant). The single global tag per variant is load-bearing — it is *required* for union
subtyping (a value's discriminant must be context-free so a narrower union can flow into a
wider including union without re-tagging), so per-union tag keying is **not** a valid fix (it
would break coercion). Fully supporting divergent unions needs a global canonical tag
assignment guaranteeing per-union uniqueness — a larger redesign, out of scope. The
in-scope, sound enforcement: `validation.rs` now rejects a variant whose tag would change
across unions (`check_union_variant_tag`, in both `from_module` and
`add_package_type_export`), converting the silent miscompile into a clear compile error in
*all* builds. No valid program in the suite has divergent unions, so goldens are unchanged;
the repro is now rejected. Tests `stable_include_positions_resolve` (Ok) and
`divergent_positions_are_rejected` (Err). This is the one justified behavior change: a
previously-accepted-but-unsound shape is now rejected rather than miscompiled.

**(3) Peephole sp-slot overlap** — `peephole.rs`. `set_slot` now invalidates any recorded
slot whose 8-byte range overlaps the incoming store (`|a-b| < 8`), not just the exact-offset
match; non-numeric offsets fall back to exact-string keying. Under today's 8-aligned,
full-slot frame model no two distinct offsets are within 8 bytes, so it only ever removes the
exact match — byte-identical. Tests `forwards_disjoint_full_slot_store` (still forwards) and
`does_not_forward_partially_overwritten_slot` (guard fires).

**(4) `stream_is_riscv` field sniff removed** — `analysis.rs` / `peephole.rs` /
`regalloc/mod.rs` / `function_lowering.rs`. Deleted the operand-string sniffer; `is_riscv` is
now threaded explicitly from the codegen entry point (the active backend's
`register_model().arena_base() == "s11"`, the same signal `regalloc::allocate` already uses)
through `remove_fp_shuttles` → `integer_live_out`. Same value as the sniff on every real
stream — byte-identical — but a label/symbol spelled like a RISC-V register can no longer
select the wrong clobber mask. The `remove_fp_shuttles` unit tests pass `false` explicitly.
