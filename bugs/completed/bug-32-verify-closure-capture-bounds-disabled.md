# bug-32: `ir::verify_semantics` disables the closure capture-index bounds check when a body function is targeted by closures of differing capture arity

Last updated: 2026-07-08
Effort: medium (1h–2h)

`ir::verify_semantics` enforces, as one of its stated soundness guarantees, that
every `Capture{index}` in a closure body addresses a slot within the closure's
captured-slot count. That check is `check_value_captures`
(`src/ir/verify/mod.rs:3558-3576`), which is **skipped entirely** when the slot
count is `None` (`:3561-3563`). The slot count comes from `closure_slot_count`
(`:659-666`), which returns `None` whenever the body function is observed with
**more than one distinct** capture count (`counts.len() != 1` → `None`).

An attacker-crafted package can therefore emit two `Closure` nodes that name the
**same** body function but carry **different** capture-vector lengths — e.g.
`Closure{name:"$lambda_7", captures:[a]}` and
`Closure{name:"$lambda_7", captures:[a,b]}`. `collect_closures` records
`closure_counts["$lambda_7"] = {1, 2}`; `closure_slot_count` sees the ambiguity and
returns `None`; `check_value_captures` then performs **no bounds check** on that
body. The body may contain `Capture{index: 9999}`, and codegen loads the closure
environment at index 9999 — an out-of-bounds read past the heap-allocated env.

The single correct behavior a fix produces: an ambiguous / attacker-inducible
closure shape does **not** disarm the capture-bounds check; a body function reached
by closures of differing capture arity is either bounded conservatively or rejected.

Severity HIGH: a soundness hole in the decoded-package trust boundary that yields
an OOB env read. As with bug-31, practical exploitation is gated by the `.mfp`
signature check (requires a malicious/compromised signer) and the OOB depends on
codegen using the capture index as an env offset (it does), but this pass is
exactly the defense-in-depth layer meant to catch such IR.

References:

- `src/ir/verify/mod.rs:657-666` (`closure_slot_count` — `None` on ambiguity),
  `:3558-3576` (`check_value_captures` — early-return on `None` slots),
  `:258` (`collect_diagnostics`/`collect_closures` populate `closure_counts`).
- Contrast: a single closure site gives a known count and an out-of-range
  `Capture` index is correctly rejected via `VERIFY_TYPE` (`:3570-3574`).
- audit-1 PKG-02 (decoded-IR trust boundary; this pass is its mitigation).
- Found during goal-01 review of `src/ir/verify/**`.

## Failing Reproduction

Craft a `.mfp` containing two `Closure` nodes naming `$lambda_7` with capture
vectors of length 1 and 2, and a `$lambda_7` body that reads `Capture{index:9999}`;
import it.

- Observed: `closure_slot_count("$lambda_7")` returns `None`,
  `check_value_captures` skips, verification accepts the module, and codegen loads
  the env at slot 9999 (OOB).
- Expected: verification rejects the out-of-range capture (or the ambiguous body).

Contrast: with a single closure site for `$lambda_7`, an out-of-range capture is
rejected today.

## Root Cause

`closure_slot_count` conflates "unknown/unresolvable" with "ambiguous", mapping
both to `None`; `check_value_captures` treats `None` as "skip". The skip-if-unknown
discipline (sound for genuinely-unknowable shapes the front end never emits) is
unsound for the capture-bounds guarantee because ambiguity is attacker-inducible —
the real front end never emits two closures over one body with differing capture
arity.

## Goal

- The capture-bounds check is never disabled by an attacker-inducible ambiguous
  closure shape.

### Non-goals (must NOT change)

- The single-site bounds check (correct today).
- Front-end-produced closures (which always have a single consistent count).

## Blast Radius

- `closure_slot_count` and `check_value_captures`. On the package path, also reject
  a body function targeted by closures of differing capture arity (the front end
  never produces one).

## Fix Design

When counts are ambiguous, bound against the **minimum** observed count (the most
conservative slot bound) instead of skipping — any `Capture` index `>= min` is
rejected. Additionally, on the package path, reject outright a body function reached
by closures of differing capture arity, since it is a shape the front end cannot
produce (a strong structural signal of tampering).

## Phases

### Phase 1 — failing test + audit

- [ ] IR-verify test with two closures over one body (counts 1 and 2) and an
      out-of-range capture; assert rejection. Confirm it is accepted today.
- [x] Structural confirmation complete (above).

### Phase 2 — the fix

- [ ] Bound against the minimum count on ambiguity (and/or reject differing-arity
      closure bodies on the package path).

### Phase 3 — validation

- [ ] `scripts/test-accept.sh` — valid closures/goldens byte-identical; new
      rejection fixture added.

## Validation Plan

- Regression test(s): the ambiguous-closure OOB-capture rejection test + a
  valid-closure suite proving no false rejections.
- Runtime proof: importing the crafted `.mfp` is rejected.
- Full suite: `scripts/test-accept.sh`.

## Summary

An ambiguous closure shape disarms a soundness check; the fix bounds conservatively
(minimum count) or rejects the front-end-impossible shape, without affecting valid
closures.
