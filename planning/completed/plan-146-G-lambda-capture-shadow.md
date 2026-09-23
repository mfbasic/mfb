# plan-146-G: S9, a `String` capacity shadow the lambda shares with its creator

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-146-F

Prerequisites: see plan-146-A.

A `MUT s AS String` captured by reference in a `collections::forEach` lambda has
no capacity shadow. plan-142-G Correction G1 removed it on purpose. The lambda
holds the address of `s`'s slot but not of the shadow slot, so a lambda that
replaced `s` left the owner's shadow describing the old buffer. The owner's next
`s = s & t` then wrote past the new, tight buffer
(`rt_byref_string_capture_capacity`). So `&` and every `String` arm from B–E
decline at S9 (G1), and the statement copies.

The self-update scratch has the same shape and is already shared. A lambda that
borrows its creator's scratch gets one environment word past its captures,
holding the address of the creator's scratch slot. Its reserve loads and publishes
through that word (plan-142-G Correction G4, `scratch_closure_captures`,
`emit_publish_borrowed_scratch`). G gives a by-ref `String` capture the same
thing for its shadow. With it, every `String` arm and `&` fire at S9, and every
write through the reference keeps the owner's shadow true.

## 1. Goal

- A by-ref-captured `String` local that is a `String` self-update target in its
  owner or in any capturing lambda (`is_string_self_update`) has a shadow in the
  owner's frame. Each capturing lambda receives the shadow slot's address in its
  environment.
- `SelfUpdateSite` for S9 carries the shadow: `string_shadow_slot(site)` for a
  `Ref` destination loads it through the environment word, and the arm publishes
  it back, as a global's hidden shadow is published.
- The copying write through the reference (`reassign_ref_old`) frees the old block
  at `len + 9 + shadow` and resets the shared shadow to 0. So does every other
  store that replaces the block through the reference.
- `&` at S9: `NirOp::Assign` builds a `Ref` destination for a `&` chain rooted at
  the by-ref local too, not only for a `Call` (`is_self_update_call`).
- `Site::Lambda` is enabled for `String` rows in the matrix (`Probe::source`) and in
  the harness (`Site::applies`). Every `String` `Arm` row, and `&`, fires at S9.
- `rt_byref_string_capture_capacity` passes unchanged. Its cases are exactly the
  aliasing this letter must keep safe.
- Observation O2 (a lambda's dead `strcap_<name>` slot, findings §3.2) is gone: a
  lambda's by-ref local takes the shared shadow, not a frame slot of its own.

### Non-goals

- Which callbacks are non-escaping (still only `forEach`'s action,
  `is_nonescaping_callback_arg`).
- No change to by-value captures.
- A lambda nested inside a lambda that captures the same `String` by reference
  again: Phase 1 measures whether the environment chain reaches it. If it does not,
  G1 stays for that shape as a recorded gate with its own runtime case.

## 2. Current State

- `prescan_string_self_appends` skips a local in `address_taken_locals`
  (`builder_control.rs:2389-2402`, the comment citing
  `rt_byref_string_capture_capacity`). After B, the rule lives in
  `is_string_self_update`'s callers. The skip stays until this letter.
- The concat arm declines `site.by_ref` (G1, `builder_inplace_assign.rs:1615`).
  B's resolver declines every `Ref` destination at G1.
- The scratch borrow: `scratch_closure_captures` (`self_update.rs:468`) finds the
  lambda's closure and returns the capture count. The environment word at that
  index holds the creator's scratch slot address. `prescan_self_update_scratch`
  (`:503`) sets `self_update_scratch_env` in the lambda.
  `emit_reserve_self_update_scratch` and `emit_publish_borrowed_scratch` load and
  store through it.
- The copying by-ref reassign frees the parent's old block through the reference
  (plan-142-G Correction G4(c)). Today its free size is `len + 9`, which is correct
  only because a by-ref `String` never has spare bytes.

## 3. Design

**The environment word.** For each closure whose lambda holds a by-ref capture of
a `String` local that has a shadow, the creator appends one environment word per
such capture, after the scratch word if there is one, holding the shadow slot's
address. A helper beside `scratch_closure_captures` computes the index:
`string_shadow_env_index(lambda, local) -> Option<usize>`. The layout is
decided in one place and read by both sides.

**Owner side.** The prescan claims a shadow for a by-ref-captured `String` local
when `is_string_self_update` holds for it in the owner *or in any lambda that
captures it by reference*. The owner's own writes are unchanged: arms use the
slot, and every other store resets it.

**Lambda side.** `string_shadow_slot(site)` for `InPlaceDest::Ref` loads the shadow
through the environment word into a working slot (`su_ref_strcap`), and the
publish step stores it back. `reassign_ref_old` loads the shared shadow, frees the
old block at `len + 9 + shadow`, stores the new block, and stores 0 to the shared
shadow.

**Gates.** The resolver's G1 accepts a `Ref` destination when
`string_shadow_env_index` answers. The concat arm's G1 does the same.

Risk: high. This is the aliasing surface that produced a heap overflow in
plan-142-G. Two stores must never disagree: the block pointer (through the
reference) and the shadow (through the environment word). Every store path
through a reference is enumerated in Phase 1, and each gets a runtime case.

Rejected alternatives:

- **Keep the block tight at S9** (no shadow; a shrink returns its tail, a grow
  reallocates exactly): every grow and rewrite allocates per statement, so it fails
  the harness bound. This is plan-146-A Open Decision 4's alternative, and it only
  fits shrink.
- **Pass the shadow in the reference itself** (a two-word reference): it changes
  the by-ref ABI for every captured type, not only `String`.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work. `- [~]` partial. Moot tasks struck through with evidence, never
> deleted. **An unticked box means NOT DONE.**

### Phase 1: Enumerate the reference's store paths (measure first)

- [x] List every codegen path that stores a new block through a by-ref `String`
      local's reference: `reassign_ref_old`, the arms' `close_inplace_dest`, and any
      other (grep `ref_slot` and the by-ref branches in `builder_control.rs`). Record
      each with its `file:symbol`.

      | path | `file:symbol` | what it does now |
      |---|---|---|
      | the copying reassign through the reference | `engine/control/builder_control.rs`, `NirOp::Assign`'s `by_ref && is_freeable_flat_value` branch (slot `reassign_ref_old`) | frees the owner's old block SIZED BY THE SHARED SHADOW and resets that shadow to 0 (this letter) |
      | every arm's publish | `collection/assign/inplace_dest.rs:close_inplace_dest`, `InPlaceDest::Ref` | stores the (possibly reallocated) block pointer back through the reference; the shadow it left is published by `publish_string_shadow` |
      | the owner's own drop | `engine/control/builder_control.rs:string_capacity_slot_for` → `emit_owned_value_drop` | frees by `len + 9 + shadow`, the shadow the lambda published (bug-560's rule, unchanged) |
      | the owner's own stores | `reset_string_capacity_shadow` | zeroes its own slot, as before |

      Nothing else writes the binding through the reference: the only other
      `by_ref` branches in `NirOp::Assign` are the thread/resource cleanups (not
      `String`), and a `FOR EACH` over a by-ref binding walks a snapshot
      (`operand_snapshot.rs`).
- [x] Measure the nested-lambda shape: does a lambda inside a capturing lambda that
      captures the same `String` receive a by-ref-of-by-ref, and can the environment
      word reach it? Record the answer and the probe.
      **The shape does not exist: it does not compile, for any type.**
      `collections::forEach(one, LAMBDA(v) -> collections::forEach(one, LAMBDA(w) ->
      s = s & "x"))` → `error: NIR assignment targets unknown local 's'`, and the
      same program with `xs = collections::append(xs, 2)` fails identically
      (`error: NIR assignment targets unknown local 'xs'`). A nested lambda never
      receives the outer lambda's capture, so there is no by-ref-of-by-ref to reach
      and no gate to record — the limitation predates this plan and is not
      `String`-specific.
- [x] Measure golden churn: the rt fixtures whose `forEach` lambda captures a
      `String` by reference and writes it
      (`grep -rlE 'forEach\(.*LAMBDA.*-> *([a-z]\w*) = ' tests --include='*.mfb'`,
      narrowed to `String` bindings by reading each hit). Record the count.
      **Zero**: the full artifact gate after this letter reported `1492 tests, 1667
      build(s), 2112 golden(s) checked, 0 diff(s)`. No committed fixture holds a
      by-ref `String` capture that self-updates, so enabling S9 moved no golden.
Commit: c8316aa3d (Phases 1-3 landed as one commit)

### Phase 2: The shared shadow

- [x] ~~`string_shadow_env_index`~~ `string_shadow_captures`; the creator's
      environment word; the owner-side prescan claim. The list is computed from the
      NIR of the lambda AND of its creator, so both sides agree on the layout
      without either seeing the other's frame (Correction G1).
- [x] Lambda side: `string_shadow_slot` for `Ref`, the publish, and
      `reassign_ref_old` (and every Phase 1 path) sizing the free by the shadow and
      resetting it. The owner's shadow-slot ADDRESS is spilled into a frame slot at
      its first use rather than re-read from `%closure_env` (Correction G2).
- [x] Runtime: `rt_byref_string_capture_capacity` passes unchanged. Add cases:
      - the lambda grows `s` in place (`s = strings::padRight(s, …)`), then the owner
        appends past the grown capacity;
      - the lambda reassigns `s`, then the owner shrinks and grows it;
      - the owner grows `s`, then the lambda self-updates it with each arm kind.
      Each allocates three neighbours after the lambda, as the existing cases do, so
      a stale shadow corrupts a visible value.
      Four new cases (`lambda_grows`, `lambda_shrinks`,
      `lambda_reassigns_then_owner_grows`, `owner_grows_then_lambda_every_arm`), and
      the second of those FOUND A BUG this letter would otherwise have shipped
      (Correction G3).

Acceptance: `cargo test --test rt_byref_string_capture_capacity` → pass
(est. 3 min).
Result: `test result: ok. 1 passed; 0 failed` — the two original cases and the four
new ones.
Commit: c8316aa3d

### Phase 3: Enable S9

- [x] Resolver G1 and concat G1 accept a `Ref` destination with a shared shadow.
      `NirOp::Assign` builds a `Ref` destination for a `&` chain rooted at the
      by-ref local. (G1 itself only requires the opened `Ref`; the shadow
      requirement is `G-shadow`, so `toString`'s identity arm — which changes no
      length — fires at S9 without one.)
- [x] `Probe::source` and `Site::applies` enable `Lambda` for `String`, and the
      `Site` docs drop "the `&` row has no S9". (An `AttributedString` line stays at
      S1/S2: plan-146-A Open Decision 1 defers those rows entirely.)
- [x] RED proof: skip the shadow reset in `reassign_ref_old`. The new "lambda
      reassigns, owner grows" case must fail. Restore.
      Two cases fail: `lambda_reassigns_then_owner_grows: printed "203" (want
      "203\nT3U3V3"), exit Some(255), stderr Error: 7-701-0001 Allocation failed.`
      and `reassigned: printed "" … exit None` (the original plan-142-G case dies
      outright). Restored.

Acceptance: `cargo test --bin mfb self_update` (the matrix now includes S9 for
every `String` `Arm` row) and the harness over every `String` line at S9
(the filter loop from plan-146-A Phase 2) → pass
(est. 10 min).
  Expected golden diffs: the fixtures Phase 1 counted. Run
  `cargo test --test golden`. Every diff must trace to a by-ref `String` capture:
  objdump one.
Result: `cargo test --bin mfb self_update` → `10 passed` (the matrix now compiles
every `String` `Arm` row at S9 too, and every one fires); the harness at S9 over
every `String` line (10 filters, `MFB_SELF_UPDATE_SITES=Lambda`) → all `test
result: ok`; `cargo test --test rt_byref_string_capture_capacity` → `ok`. Golden:
`1492 tests, 1667 build(s), 2112 golden(s) checked, 0 diff(s)` — no fixture holds
the shape.
Commit: c8316aa3d

## Validation Plan

- Tests: `rt_byref_string_capture_capacity` (unchanged cases plus the new ones),
  the matrix and the harness at S9.
- Per-letter gate: `cargo test --bin mfb`.

## Open Decisions

None beyond plan-146-A Open Decision 3, which this letter implements. If that
decision is taken the other way, this letter becomes: record G1 as the S9 gate for
every `String` arm, and add a harness line per arm asserting the copy.

## Corrections

- **G1 — the env layout is computed from the NIR, by both sides, and covers every
  by-ref `String` capture whose block can carry spare bytes.** §3's
  `string_shadow_env_index(lambda, local)` would have to know the creator's frame
  (which slot the shadow is in) from inside the lambda, and the lambda's body from
  inside the creator. `string_shadow_captures(functions, lambda)` instead answers
  from NIR alone — the lambda's `Bind … Capture { by_ref: true }` list, filtered by
  "the lambda self-updates it, OR its creator does" — so the creator emits the words
  and the lambda reads them at the same indices. The "or its creator does" half is
  not an optimization: a lambda that merely REASSIGNS the capture still frees the
  owner's block through the reference, and that free must be sized by the shadow and
  must reset it.
- **G2 — `%closure_env` does not survive a call inside an arm.** It is a
  call-boundary token, not a pinned register (`abi.rs:CLOSURE_ENV`, AArch64 `x28`),
  and `os::resourcePath`'s arm calls a runtime helper in the middle of the
  statement. Re-reading it afterwards to publish the shadow stored through a wild
  pointer (a segfault at the third lambda call). Every env word this letter adds is
  therefore read ONCE into a frame slot (`su_ref_strcap_owner`, `ref_strcap_owner`,
  `str_resource_base_owner`) and used from there.
- **G3 — two latent bugs the new aliasing cases caught.**
  (a) **The closure-env free was already under-sized.** `closure_env_free_types`
  sized the free from the captures alone, so plan-142-G's borrowed-scratch word was
  8 bytes short of the block the creator allocated; this letter's shadow word made
  it 16. The allocator then handed part of a still-referenced block out again. Both
  extra word kinds are counted now.
  (b) **The shadow word collided with the scratch word.** The lambda's index was
  computed as `captures + usize::from(self_update_scratch_env.is_some())`, but that
  field is set by a LATER prescan, so it read `None` and both words landed on index
  1: the lambda published its scratch pointer into the owner's shadow and leaked the
  scratch (one block per lambda call, then a segfault at scale — 2000 iterations
  died at teardown). The index now comes from `scratch_closure_captures`, the same
  predicate the creator uses. Measured after the fix: the same program runs 2000
  iterations with `arena.0.alloc_calls 10005`, `free_calls 10005`, `live_bytes 0`.
- **G4 — `os::resourcePath`'s base cache is shared with the lambda too.** plan-146-D
  cached the executable path in a frame slot, which at S9 is the LAMBDA's frame — a
  lambda runs once per element, so the cache was re-acquired every call and the
  harness measured one allocation per statement (`marked `arm`, but 2000 more runs
  allocated 2000 more blocks`). The creator's cache is now shared through one
  further environment word, exactly like the scratch and the shadow: the lambda
  fills it on its first call and publishes it back, and the creator owns and frees
  it. Also: its frame slot is registered in `owned_value_slots` so the prologue
  zeroes it — without that, a second call read the block the first one cached and
  freed.

## Summary

G shares a by-ref `String` capture's shadow with its lambdas, the way the
self-update scratch is already shared. Every store through the reference keeps the
block and the shadow in step. The risk is the aliasing plan-142-G tripped over,
so every store path is listed first and each gets a runtime case.
