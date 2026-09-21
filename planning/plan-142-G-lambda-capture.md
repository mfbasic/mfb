# plan-142-G: In place for a `MUT` captured by a non-escaping lambda (S9)

Last updated: 2026-09-20
Effort: medium (1h–2h)
Depends on: plan-142-F

Prerequisites: see plan-142-A.

Inside `collections::forEach(xs, LAMBDA(v AS T) -> acc = collections::append(acc, v))`,
`acc` is a by-ref local: its slot holds a pointer to the parent's slot
(`src/ir/lower.rs:5167`; codegen `builder_control.rs:501-520`). Every arm declines
it (G1, `inplace_dest.rs:149-151`), and the fallback writes the new block through
the pointer (`:1386-1396`) **without freeing the parent's old block** (`:1309`,
`!by_ref`). After G, the self-update is in place through the reference and nothing
leaks.

Only `forEach`'s action argument is non-escaping
(`is_nonescaping_callback_arg`, `builtins/mod.rs:730-731`), so S9 is exactly the
`forEach` lambda.

## 1. Goal

- `S9` in `ENABLED_SITES`; every `Arm` row fires at S9; harness S9 template
  (`collections::forEach(src, LAMBDA(v AS T) -> x = <statement>)`) flat in `N`.
- `forEach(acc, LAMBDA(v AS T) -> acc = collections::append(acc, v))` — the lambda
  mutating the list `forEach` walks — visits the entry-time elements and ends with
  the doubled list (runtime case).

### Non-goals

- Which callbacks are non-escaping (still only `forEach`).
- `String` self-concat through a by-ref capture: in scope (the `concat` arm is in
  `SELF_UPDATE_ARMS`), provided its capacity shadow can live in the parent frame —
  Phase 1 decides; if it cannot, it is a recorded gate with its own case.

## 3. Design

- **`InPlaceDest::Ref { ref_slot }`**: open = load the parent-slot pointer from
  `ref_slot`, load the block pointer through it into a scratch slot; the arm runs
  on the scratch slot as a `Direct` destination; close = store the scratch slot's
  (possibly reallocated) pointer back through the parent-slot pointer. The same
  open/close shape as `STATE`'s `open_inplace_state_dest` / `close_inplace_dest`
  (`inplace_dest.rs:427`, `:609`).
- **The walked collection.** `forEach` holds `acc`'s block pointer and count
  (`func_for_each.rs:153-191`) while the lambda runs. When `forEach`'s argument 0
  is the same binding the lambda captures by reference, `forEach` lowers argument 0
  owned (one copy per call, freed after), exactly F's rule for `FOR EACH`.
  Otherwise an in-place grow would free the block `forEach` is walking.

Risk: the `forEach`/lambda alias rule. Before G it is harmless only because the
fallback leaks; G turns the leak into a free, so the rule must be in place first.

## Phases

### Phase 1 — `forEach` owns an argument its lambda writes

- [ ] In `lower_for_each` (`func_for_each.rs:117`), lower argument 0 owned when the
      callback is a lambda with a by-ref capture of the same local; free it after the loop.
- [ ] Decide the `concat` shadow question and record it.
- [ ] Runtime case `tests/runtime/rt_lambda_capture_self_update.rs` (+ stanza): the
      self-walking `forEach` above, visit order and final list.

Acceptance: `cargo test --test rt_lambda_capture_self_update` → pass (est. 3 min).
Commit: —

### Phase 2 — `InPlaceDest::Ref` and S9

- [ ] `InPlaceDest::Ref` open/close; `SelfUpdateSite` built with it for a `by_ref`
      local; G1 no longer declines a `Ref` destination.
- [ ] Add S9 to `ENABLED_SITES` and the harness; add a
      `perf.mfb_free`-balance case showing no leaked parent block.

Acceptance: `cargo test --bin mfb self_update && cargo test --test rt_inplace_self_update --test rt_lambda_capture_self_update`
→ pass at S1, S7, S9 (est. 20 min).
Commit: —

### Phase 3 — Expected outputs

- [ ] `tests/rt-error/functions/lambda-mut-foreach-valid` and
      `tests/rt-behavior/functions/byref-capture-rt` must pass unchanged in output;
      regenerate their `.ncode` goldens only if committed (measured 2026-09-20: neither has one).

Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` green on
`tests/rt-behavior/functions` and `tests/rt-error/functions` (est. 10 min).
Commit: —

## Validation Plan

- Tests: `rt_lambda_capture_self_update`; matrix + harness at S9.

## Corrections

## Summary

One new destination kind and one alias rule in `forEach`; arms unchanged.
