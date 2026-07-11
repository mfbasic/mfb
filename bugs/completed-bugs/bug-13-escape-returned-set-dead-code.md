# bug-13: `escape::solve` accumulates a `returned` resource set that is never read (dead code, silenced with `let _ = &returned;`)

Last updated: 2026-07-08
Effort: small (<1h)

In `src/escape.rs::solve`, the ownership fixpoint maintains a
`returned: HashSet<String>` (`escape.rs:281`) and inserts into it in the
`Target::Returned` arm (`:300-306`). The set is then explicitly discarded —
`let _ = &returned;` (`:314`) — and never consulted again. The actual ownership
decision uses the separately-computed `returned_collections` (`:271`) and
`membership` (`:280`). The `returned` set is leftover from an earlier design that
tracked "is this resource returned" per-resource; it was superseded by the
`returned_collections`/`membership` scheme but the accumulation (and a
`let _ = &returned;` to silence the unused warning) was left behind.

The single correct outcome a fix produces: the dead `returned` set, its
`Target::Returned` insert branch, and the `let _ = &returned;` silencer are
removed, with the fixpoint's `membership` result — the only thing the decision
consumes — provably unchanged.

Severity LOW (dead code, no behavioral effect). It is worth removing because the
`let _ = &returned;` marker is a code smell that actively disguises known-dead
computation and makes the fixpoint harder to reason about.

References:

- `src/escape.rs:268-312` (`solve` fixpoint) — `:281` (`returned` decl), `:300-306`
  (`Target::Returned` insert + `changed`), `:314` (`let _ = &returned;`).
- `src/escape.rs:271-276` (`returned_collections`, genuinely used at `:328-347`),
  `:280` (`membership`, the consumed output).
- Found during goal-01 review of `src/escape.rs`.

## Failing Reproduction

Not a runtime defect — the generated ownership decisions are correct today. The
"reproduction" is the static contradiction:

```
# src/escape.rs::solve
:281  let mut returned: HashSet<String> = HashSet::new();   // written...
:302      if returned.insert(resource) { changed = true; }   // ...only written
:314  let _ = &returned;                                     // ...and discarded
```

- Observed: `returned` is written every fixpoint iteration but never read; the
  compiler's unused warning is suppressed with `let _ = &returned;`.
- Expected: no dead set; the fixpoint tracks only what the decision consumes
  (`membership`).

## Root Cause

`returned` is a vestige of a superseded per-resource "is-returned" scheme. Its
only live effect is that its inserts flip `changed` (`:303`), which can extend the
fixpoint by extra iterations — but those iterations cannot change `membership`,
because `membership` is written *only* by the `Target::Var` arm (`:292-298`) and
that arm's progress depends solely on `membership` + `incoming`, never on
`returned`. So `returned` influences neither the loop's fixed point for
`membership` nor any downstream decision.

## Goal

- Delete `returned`, its `Target::Returned` insert branch, and the
  `let _ = &returned;` line; keep the `Target::Var` arm's `changed` bookkeeping so
  the fixpoint still terminates on the real (`membership`) fixed point.

### Non-goals (must NOT change)

- Do not change `returned_collections` (`:271`) or `membership` (`:280`) — they
  are the live inputs to the decision and must be byte-for-byte equivalent.
- Do not change any computed owner / float target.

## Blast Radius

- `escape::solve` only. `returned_collections` and `membership` are unaffected.
  An exhaustive read of `solve` confirms `returned` has no reader.

## Fix Design

Remove the `returned` declaration and the `Target::Returned` arm's body (the arm
can become a no-op `Target::Returned => {}` or be folded so the loop no longer
inspects it beyond `membership` propagation). Confirm the `changed` flag is still
driven by the `Target::Var` inserts so the fixpoint converges identically. Prove
equivalence by asserting the acceptance suite (which exercises resource
ownership/close ordering) is byte-identical.

## Phases

### Phase 1 — audit (no behavior change)

- [x] Confirm `returned` has no reader beyond `let _ = &returned;` (done above).
- [x] Confirm `membership` is independent of `returned` (done above).

Acceptance: audit recorded.
Commit: —

### Phase 2 — the removal

- [ ] Delete `returned`, its insert branch, and `let _ = &returned;`.

Acceptance: compiles with no unused warning; `membership`/owners unchanged.
Commit: —

### Phase 3 — validation

- [ ] `scripts/test-accept.sh` — must be byte-identical (dead-code removal).

Acceptance: zero golden movement.
Commit: —

## Validation Plan

- Regression test(s): none new — the existing resource-ownership acceptance
  fixtures are the guard; they must not move.
- Runtime proof: none — behavior is unchanged by construction.
- Doc sync: none.
- Full suite: `scripts/test-accept.sh`.

## Summary

A superseded per-resource "returned" set is accumulated and thrown away behind a
`let _ = &returned;`. Removing it is safe (the consumed `membership` is
independent of it) and should not move any golden.
