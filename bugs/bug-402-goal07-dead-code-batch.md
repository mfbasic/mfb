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

### (2) `src/os/windows/mod.rs:14` — stale module-wide `#![allow(dead_code)]`
The module doc (lines 6-14) claims the writer's "public surface is unreferenced by
non-test code — hence the module-scoped `dead_code` allow below" and that
"plan-47-D removes it". Both are now false: `write_native_object_plan`,
`validate_native_object_plan`, and `write_linked_executable` are called from the
production build path (`src/target/win_x86_64/mod.rs:271,281,331`; plan-47/66
landed). The `#![allow(dead_code)]` at line 14 is a **file-level allow** (the class
goal-07 flags) that now blanket-suppresses dead-code detection for the entire
`windows` module (mod.rs, object.rs, link/*), masking any genuinely dead helper
added later.
- Fix: remove the module-wide `#![allow(dead_code)]` and the stale doc paragraph;
  add targeted `#[allow]` only if a specific still-dead item remains (with a
  load-bearing reason), else let the compiler flag it.

### (3) `src/os/linux/appimage/squashfs/mod.rs:382` — vacuous, unreachable guard
`if ref_offset(inode_at[0]) as usize > METADATA_BLOCK { ... }` intends to reject a
root-inode whose in-block offset exceeds the 8192-byte metadata block, but
`ref_offset(x)` returns `(x % METADATA_BLOCK) as u16` (`mod.rs:234-235`), always
0..=8191 < `METADATA_BLOCK` (8192). The condition can never be true — dead
defensive code with a misleading error string that reads as a bounds check but
validates nothing.
- Fix: remove the vacuous `if` (the `% METADATA_BLOCK` already bounds the value),
  or, if a real invariant was intended, check the pre-modulo `stream_offset`.

## Goal

- No production field/item is retained solely on a "consumed by a later phase"
  justification; each is either read by real code or deleted. No file-level
  `#![allow(dead_code)]` masks a whole module.

### Non-goals

- No behavioral change; pure removal of unread state.

## Blast Radius

- Each item is isolated (write sites + the `#[allow]`); removal is compiler-checked.
- Additional dead-code items found later in goal-07 are appended here as they
  surface.
