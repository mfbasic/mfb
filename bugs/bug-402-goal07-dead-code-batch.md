# bug-402: dead code with forbidden "later phase" `#[allow(dead_code)]` justifications (goal-07 batch)

Last updated: 2026-07-28
Effort: small (<1h)
Severity: LOW
Class: Dead-code

Status: Open
Regression Test: none (removal is validated by the compiler: the fields/items
below have no readers, so deleting them must still build).

A batch of dead production items surfaced during the goal-07 full-source review.
Each is written but never read, and (where annotated) carries an
`#[allow(dead_code)]` whose justification is the "consumed by a later phase" form
that AGENTS.md explicitly forbids ("never 'consumed by a later phase'. Else
delete."). Batched per the goal's same-class rule; distinct-root-cause defects stay
in their own documents.

References: found during goal-07; sites listed below.

## Items

### (1) `src/resolver/mod.rs:179-183` — `LinkFnSig.return_type` / `return_resource` unused
Both fields are written in `collect_top_level_symbols` (`mod.rs:248-249`) but never
read anywhere in the resolver — the only `LinkFnSig` reads are `params`,
`param_resource`, and `line` (`resolution.rs:586-601`). They carry
`#[allow(dead_code)]` with the rationale "Consumed by later native-resource phases
(producer typing); recorded now." — the forbidden "later phase" justification.
- Evidence: `grep -rn "\.return_resource\|\.return_type" src/resolver/` shows only
  the write sites, no reads.
- Fix: delete both fields (and their writes), or — if a concrete near-term consumer
  exists — wire it and replace the justification with a load-bearing reason.

## Goal

- No production field/item is retained solely on a "consumed by a later phase"
  justification; each is either read by real code or deleted.

### Non-goals

- No behavioral change; pure removal of unread state.

## Blast Radius

- Each item is isolated (write sites + the `#[allow]`); removal is compiler-checked.
- Additional dead-code items found later in goal-07 are appended here as they
  surface.
