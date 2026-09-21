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

- [x] In `lower_for_each` (`func_for_each.rs:117`), lower argument 0 owned when the
      callback is a lambda with a by-ref capture of the same local; free it after the loop.
      Done at the operand seam rather than in `func_for_each.rs` (Correction G2): a call
      argument rooted at a local (`acc`, `acc.items`) that a sibling closure captures
      by reference is snapshotted (`want_arguments_the_call_can_free`,
      `operand_snapshot.rs`) — a statement-scope copy freed with the statement, as
      bug-665 does for a global. The same reason extends F's `FOR EACH` rule to a field
      iterable (`FOR EACH v IN r.items`) whose body captures `r` by reference.
- [x] Decide the `concat` shadow question and record it. **It cannot live in the
      parent frame** — Correction G1: the lambda sees only the address of the parent's
      binding slot, never the parent's shadow slot. `s = s & t` through a reference
      is a recorded gate (the `concat` arm keeps declining `by_ref`), with its own
      case (`string_concat` in `rt_lambda_capture_self_update`: correct and balanced,
      via the fallback). Deciding it found a heap overflow — fixed first
      (`1f204c22a`, Correction G1).
- [x] Runtime case `tests/runtime/rt_lambda_capture_self_update.rs` (+ stanza): the
      self-walking `forEach` above, visit order and final list. Five programs
      (append, prepend, removeAt, `List OF String`, and a walk over another list);
      they held before G too (the fallback leaked instead of freeing), and they are
      what keeps the snapshot honest once Phase 2 frees.

Acceptance: `cargo test --test rt_lambda_capture_self_update` → pass (est. 3 min).
Verified 2026-09-21 (with Phase 2): `test result: ok. 2 passed; 0 failed`.
Commit: —

### Phase 2 — `InPlaceDest::Ref` and S9

- [x] `InPlaceDest::Ref` open/close; `SelfUpdateSite` built with it for a `by_ref`
      local; G1 no longer declines a `Ref` destination. `InPlaceDest::Ref { ref_slot,
      block_slot }` (`inplace_dest.rs`): `open_inplace_ref_dest` copies the parent's
      block pointer into `block_slot`, `close_inplace_dest` stores it back through the
      reference; `try_inplace_self_update` brackets the arms with them. The Assign
      dispatch builds it only for a statement shaped `f(name, …)` over a self-update
      builtin (`is_self_update_call`), so no other by-ref assignment gains a slot.
- [x] Add S9 to `ENABLED_SITES` and the harness; add a
      `perf.mfb_free`-balance case showing no leaked parent block.
      `Site::Lambda`: the matrix probe runs `x = <call>` in a `forEach` lambda and
      looks for the arm's marker in the lifted `$lambda*` function. The harness turns
      each statement into `collections::forEach(one, LAMBDA(each1 AS Integer) -> x =
      …)` inside the `N` loop, so statements interleave exactly as at S1; each such
      call allocates its closure, so every program has an idle twin with `one` empty
      and the bound is on the difference (Correction G3). Balance cases
      (`a_self_update_through_the_reference_leaves_no_parent_block_behind`, seven
      programs, `arena.0.alloc_calls == free_calls`, `live_bytes == 0`): RED with the
      pre-G binary on all seven, e.g. "self_append: … 18 allocated / 15 freed / 224 B
      live". Three more pieces were needed for S9 to be flat (Correction G4): a
      reassignment through a reference frees the parent's old block (the fallback
      leaked it); a lambda borrows its creator's self-update scratch through a hidden
      closure-env word (its own would be allocated once per call); and a lambda no
      longer copies a by-value capture per call (`70b817ad5`).

Acceptance: `cargo test --bin mfb self_update && cargo test --test rt_inplace_self_update --test rt_lambda_capture_self_update`
→ pass at S1, S7, S9 (est. 20 min).
Verified 2026-09-21: `cargo test --bin mfb self_update` → `test result: ok. 4 passed`;
`cargo test --test rt_inplace_self_update` → `test result: ok. 1 passed; 0 failed`
(518.08s; 190 case/site pairs — 64 at S1, 63 at S7, 63 at S9);
`cargo test --test rt_lambda_capture_self_update` → `test result: ok. 2 passed`.
Commit: —

### Phase 3 — Expected outputs

- [x] `tests/rt-error/functions/lambda-mut-foreach-valid` and
      `tests/rt-behavior/functions/byref-capture-rt` must pass unchanged in output;
      regenerate their `.ncode` goldens only if committed (measured 2026-09-20: neither has one).
      Re-measured: `ls` of both `golden/` dirs shows `.ast`/`.ir`/`.run`/`build.log`
      only; both pass in the acceptance run below.

Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` green on
`tests/rt-behavior/functions` and `tests/rt-error/functions` (est. 10 min).
Verified 2026-09-21: `scripts/test-accept.sh target/debug/mfb target/accept-actual
'rt-behavior/functions/*' 'rt-error/functions/*' 'rt-behavior/collections/*'
'rt-behavior/closures/*' 'rt-behavior/lambda*'` → `acceptance tests passed (93 test(s)
ran)`.
Commit: —

## Validation Plan

- Tests: `rt_lambda_capture_self_update`; matrix + harness at S9.

## Corrections

- **G1 (Phase 1): the `concat` shadow cannot follow a reference — and a stale one
  overflowed the heap.** The capacity shadow is a slot of the frame that owns `s`;
  a by-ref capture carries the address of `s`'s slot only. Worse, a lambda that
  reassigned `s` through the reference left the owner's shadow describing the old
  buffer, so the owner's next `s = s & t` wrote past the new, tight one
  (`Error: 7-701-0001`, or a crash). Fixed in its own commit (`1f204c22a`,
  `rt_byref_string_capture_capacity`): a by-ref-captured local gets no shadow. So
  `String` has no S9 form: the matrix and harness skip the `&` row there, and the
  gate has its runtime case.
- **G2 (Phase 1): the owned argument is the operand snapshot.** Rather than a new
  owned-lowering in `func_for_each.rs`, the rule sits where bug-665's does
  (`operand_snapshot.rs`), so every call — not only `forEach` — handing a callee a
  way to reassign a local while it borrows that local is covered, `acc.items` as
  well as `acc`.
- **G3 (Phase 2): the S9 template.** A lambda is one statement, so the harness
  cannot put a multi-statement line in one lambda over `N` elements; running each
  statement's `N` calls back to back would change what a line computes
  (`x = math::acos(x) ; x = math::clamp(x, …)` would apply `acos` `N` times before
  any `clamp`, leaving its domain). One `forEach` over a one-element list per
  statement keeps S1's interleaving; the closure each call allocates is measured
  by an idle twin (`one` empty) and subtracted — the bound itself is unchanged.
- **G4 (Phase 2): three more allocation sources, fixed.** S9 first failed for
  `sort`, `filter`, `union`, `mapValues`, the math arm and bulk `append`, one block
  per call each: (a) the scratch arms and the math arm allocated the lambda's own
  self-update scratch on every call — a lambda now borrows its creator's
  (`scratch_closure_captures`: one env word past the captures holds the address of
  the creator's scratch slot, and the lambda's reserve loads and publishes through
  it); (b) a by-value capture (`ys` in `union(x, ys)`) was deep-copied into the
  lambda on every call — fixed in its own commit (`70b817ad5`,
  `rt_lambda_capture_not_copied_per_call`); (c) the copying fallback through a
  reference leaked the parent's old block — it now frees it, which is safe because
  G2's snapshot rule leaves nothing walking that block.

## Summary

One new destination kind and one alias rule in `forEach`; arms unchanged.
