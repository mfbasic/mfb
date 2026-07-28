# bug-17: x86 `f2i_nearest` clobbers rax without preserving it, unlike its packed sibling and the other GPR-shuttling conversions (latent / consistency)

Last updated: 2026-07-08
Effort: small (<1h)

The x86-64 scalar round-to-nearest conversion `f2i_nearest`/`fcvtas_x_from_d`
(`src/arch/x86_64/encode/emitter.rs:763-778`) stages the `0.5` bit pattern in
**rax** via `enc_mov_imm64(0, 0x3FE0_0000_0000_0000)` (`:769`; register 0 = rax)
and OR-s it into `dst`. The comment (`:760-761`) asserts *"rax is free (never
allocated: excluded from the scratch pool)"*. But rax is excluded from the
allocatable pool precisely because it carries **ABI-fixed** values (call/syscall
returns, `x0`→rax), and the packed sibling `frinta_v` (`:1005/1016`,
`push rax`/`pop rax`), `div_seq` (`:1804`, `preserve_dividend`), and `msub`
(`:465`) all explicitly **preserve** rax/rdx for that reason. `f2i_nearest` alone
does not, making it inconsistent with the ops the authors judged needed
preservation.

The single correct outcome a fix produces: `f2i_nearest` does not leave rax
clobbered across its emission if any caller could hold a live value there — i.e.
it matches the preservation discipline of its siblings, or it is proven that rax
is never live across it and the sibling `push`/`pop` is documented as
over-cautious.

Severity LOW / **latent**: no concrete trigger was constructed — the op's own
result goes to a vreg (not rax), and an ABI return in rax is normally consumed
into a vreg on the immediately following instruction, so an unrelated
`f2i_nearest` is not scheduled across a live rax. Filed because the asymmetry with
the sibling ops is a real inconsistency a maintainer should resolve one way or the
other.

References:

- `src/arch/x86_64/encode/emitter.rs:760-778` (`f2i_nearest`/`fcvtas_x_from_d`;
  rax staged at `:769`).
- Siblings that preserve rax/rdx: `:1005,1016` (`frinta_v` push/pop rax), `:1804`
  (`div_seq` `preserve_dividend`), `:465` (`msub`).
- `.ai/compiler.md` — caller-saved register lifetimes across helper calls.
- Found during goal-01 review of `src/arch/x86_64/**`.

## Failing Reproduction

None constructed (latent). The hazard: a round-to-nearest `toInt(Float)` emitted
while a live ABI-fixed value sits in rax (e.g. a preceding `_mfb_*`/syscall result
not yet consumed) — the `movabs rax, bits(0.5)` at `:769` destroys it, and the
later read of that rax value gets `0x3FE0000000000000`.

- Observed (if triggerable): the pre-existing rax value is overwritten.
- Expected: rax preserved across the op (as `frinta_v` does), or a proof it is
  never live here.

Contrast: `frinta_v`, `fcvtzs_v`, `scvtf_v` push/pop the GPR scratch;
`f2i_trunc`/`f2i_floor`/`f2i_ceil` use no GPR scratch. Only the scalar nearest
path touches rax unguarded.

## Root Cause

The arm assumes rax is unconditionally free, but rax's exclusion from allocation
does not mean it is never *live* — it is the ABI return register. The sibling ops
account for that; `f2i_nearest` does not.

## Goal

- `f2i_nearest` either preserves rax (matching `frinta_v`) or stages the `0.5`
  constant through a register proven dead here (e.g. the `dst` GPR before it holds
  the final result), removing the inconsistency.

### Non-goals (must NOT change)

- The rounding result for any input.

## Blast Radius

- `f2i_nearest`/`fcvtas_x_from_d` only.

## Fix Design

Bracket the arm with `push rax`/`pop rax` (as `frinta_v` does), or restructure to
stage the `0.5` bit pattern in `dst` (which already holds the sign bits at that
point and is a genuine scratch) instead of rax, eliminating the rax touch
entirely. The latter avoids the stack traffic and is preferred if the bit-twiddle
can be reordered.

## Phases

### Phase 1 — audit

- [x] Confirm no current schedule places a live rax across this op (latent).
- [ ] Decide: preserve rax vs. re-stage through `dst`.

### Phase 2 — the fix

- [ ] Apply the chosen preservation/re-staging.

### Phase 3 — validation

- [ ] `scripts/artifact-gate.sh`; confirm `toInt` round-to-nearest runtime results
      unchanged on x86-64.

## Validation Plan

- Regression test(s): existing `toInt(Float)` round-to-nearest coverage must stay
  green + byte-identical (behavior is unchanged; only rax discipline hardens).
- Full suite: `scripts/artifact-gate.sh`.

## Summary

A latent consistency gap: the scalar round-to-nearest conversion is the lone
GPR-shuttling float op that does not preserve rax. Resolve by matching the
siblings or by re-staging the constant through `dst`.
