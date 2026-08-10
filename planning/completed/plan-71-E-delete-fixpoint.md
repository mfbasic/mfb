# plan-71-E: delete the `remap_x86_abi` fixpoint; flip the cross-check live

Last updated: 2026-08-02
Effort: large (3h–1d)
Depends on: plan-71-D (the `MFB_BUG387_AUDIT` sweep reports 0 mismatches of any family
on both x86 targets). plan-71-D depends on plan-71-C → plan-71-B → plan-71-A.

This is the final sub-plan and the one that delivers plan-71's single behavioral
outcome: with the divergence audit at **zero** (every producer now emits the role token
whose context-free `map_token_direct` equals the register the fixpoint would infer),
`remap_x86_abi`'s three stacked CFG dataflow analyses are **deleted** and replaced by
the direct `map_token_direct` lookup, `select_x86`'s deferred-token realization is
retired, and the cross-check `assert_eq!` is flipped **live** as the deletion's safety
net (plan-71-A Open Decision 2). Every emitted byte is unchanged on all five targets;
that byte-identity, plus a green live-assert build, is the proof the 587-line fixpoint
was doing nothing the direct map does not.

References:

- `src/arch/x86_64/select.rs:208` `remap_x86_abi` / `:229` `remap_x86_abi_inner`
  (the fixpoint + its audit-returning inner, ~the `:208`–end block to delete),
  `:166` `map_token_direct` (the replacement map), `:199` `is_abi_role_token`,
  `:123` `map_abi_register`, `:36` `map_scratch_register`, `:878` `select_x86`
  (the deferred-token realize loop to retire).
- `planning/plan-71-census.md` — the divergence inventory D drove to zero; §Residue = 0
  (every inferred register has a role-token preimage — the guarantee `map_token_direct`
  is onto the register file, so the direct map is total over the role tokens).
- `planning/completed/plan-71-A-fixpoint-crosscheck-census.md` — §4 (the cross-check
  design and the plan for the final `assert_eq!`), Open Decision 2 (flip to live
  `assert_eq!` and keep it), the Non-goals (byte-identity is the law), and the Prereq
  table (windows byte-identity goldens; remote GTK/Windows box re-probe).
- `bugs/bug-387-neutral-mir-stream-carries-aarch64-register-names.md` — the bug this
  closes; reconcile its status on landing.
- `bugs/completed-bugs/bug-85-x86-entry-runtime-arg-staging-tokens.md` and
  `planning/old-plans/plan-34-B-role-named-registers.md` — the reverted direct-lookup
  attempt E finally lands *under the gate*, and the role-token vocabulary it builds on;
  reconcile both on landing.
- `src/docs/spec/architecture/` — the register-role vocabulary the deletion keeps in
  sync (`.ai/specifications.md` governs spec updates).
- `scripts/bug387-gate.sh`, `scripts/artifact-gate.sh`, `scripts/exe-oracle.sh`,
  `scripts/test-appimage.sh` — the byte-identity + runtime gates.
- `.ai/compiler.md` (completion gate; silent-wrong-register is the worst class),
  `.ai/remote_systems.md` (the GTK boxes 2228/2227, Windows box 2230).

## Prerequisites

The whole-feature preconditions live in plan-71-A's Prerequisites table and remain in
force — and, being the deletion, E is the letter that re-checks the windows
byte-identity goldens and the remote runtime boxes those rows name. This letter
additionally requires:

| Must be true | Command | Status |
|---|---|---|
| plan-71-D complete (zero divergence on both x86 targets) | `ls planning/completed/plan-71-D-*.md` | NOT MET (D not yet landed) |
| the `MFB_BUG387_AUDIT` sweep reports 0 mismatches of any family (linux + windows) | full-corpus sweep → `grep -c BUG387-MISMATCH` = 0 | NOT MET (D Phase 4 establishes it) |
| plan-71-B's value-level partition proven (no value is secretly Category 2) | `grep -n 'proven-at-the-value-level' planning/plan-71-census.md` | NOT MET (B Phase 2) |
| exe-oracle baselines re-recorded from clean `main` (all five targets, ephemeral `/tmp`) | `ls /tmp/bug387/oracle-*.txt` | RE-RECORD FIRST |
| remote GTK boxes 2228/2227 + Windows box 2230 reachable for runtime re-probe | per `.ai/remote_systems.md` | RE-PROBE (were down during plan-71-A) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** E must
> not begin until the audit is genuinely zero across the full corpus on both x86
> targets — a single surviving mismatch means a producer the direct map colors
> differently than the fixpoint, and deleting the fixpoint would then move that byte.
> Re-record the ephemeral baselines and re-run the sweep to confirm zero **immediately
> before** the deletion. If you stop, report the status of *all* rows.

Everything below is written against the world where these hold.

## 1. Goal

**plan-71-E goal (and plan-71's overall goal):** the fixpoint is gone and the direct
map is live, byte-identically, such that:

- `remap_x86_abi`'s three stacked CFG dataflow analyses (`src/arch/x86_64/select.rs`,
  the `:208`+ block, 587 lines per plan-71-A §Current State) are **deleted**;
  `select_x86` no longer defers-then-re-realizes role tokens — it realizes every operand
  once, mapping the ABI role tokens through `map_token_direct` and all others as today.
- The cross-check is flipped **live**: where the code previously chose the fixpoint's
  `mapped`, it now takes `map_token_direct(token, abi)` and (in debug/assert builds)
  `assert_eq!`s the two only during a transition build — after the fixpoint is gone the
  direct map is the sole source of truth, and the assert (retained per Open Decision 2)
  guards against any future token regression.
- **Every emitted byte is unchanged** on `{linux,macos}-aarch64`, `linux-x86_64`,
  `windows-x86_64`, `linux-riscv64`: `scripts/bug387-gate.sh <exe> full` PASS,
  `scripts/artifact-gate.sh` 0 diffs, `scripts/exe-oracle.sh` base-vs-mine byte-identical
  on all five targets.
- Runtime confirmation on the remote GTK boxes (2228/2227) and the Windows box (2230),
  and the spec/vocabulary + bug-387/bug-85 reconciliation, are complete.

### Non-goals (explicit constraints)

- **Any emitted byte, on any target.** The whole point is a zero-byte deletion; a moved
  golden is a failed change (plan-71-A Non-goals). If the deletion moves a byte, the
  audit was not truly zero — stop, re-open D, do not re-baseline.
- **Instruction selection decisions, the `EncodedImage` field set, relocation
  kind/binding, the linker's view.** Only the register-naming path changes.
- **The neutral token vocabulary.** E deletes a *consumer* of the tokens; it adds no
  token.
- **AArch64/RISC-V selection.** They never call `remap_x86_abi`; the deletion is
  entirely within `arch/x86_64/`. (Their byte-identity is nonetheless re-proven by the
  full gate — the change must be provably x86-local.)

## 2. Current State

`select_x86` (`src/arch/x86_64/select.rs:878`) currently **defers** the ABI role tokens
(skips `is_abi_role_token` values in its realize loop, per plan-71-A Phase 2) and
realizes everything else; then `remap_x86_abi` (`:208`) realizes the deferred role
tokens to `xN` so its three stacked CFG dataflow analyses can re-derive each register's
role and color it to the SysV/Win64 home, writing `mapped`. Under `MFB_BUG387_AUDIT` the
inner (`:229` `remap_x86_abi_inner`) reports every operand where `map_token_direct`
disagrees with `mapped`. After plan-71-C/D that disagreement is **zero** across the
whole corpus (D Phase 4) — meaning for every operand the fixpoint emits,
`map_token_direct(token, abi)` already equals `mapped`. The fixpoint is therefore pure
overhead: a 587-line CFG analysis computing a function a context-free table lookup
computes.

### Measured populations

| What | Count | Command |
|---|---|---|
| fixpoint block to delete | 587 lines (`:208`+) | plan-71-A §Current State (`awk` span); re-measure at E: `grep -n 'fn remap_x86_abi' src/arch/x86_64/select.rs` → start; matching close brace → end |
| audit mismatches after plan-71-D (precondition) | 0 (linux + windows) | full-corpus `MFB_BUG387_AUDIT=1` sweep → `grep -c BUG387-MISMATCH` |
| targets requiring byte-identity re-proof | 5 | `scripts/bug387-gate.sh <exe> full` (linux-x86_64, windows-x86_64, linux-aarch64, linux-riscv64) + app-ncode macos-aarch64 |
| `map_token_direct` is onto the x86 register file (total over role tokens) | yes | `plan-71-census.md` §Category 1 / §Residue — every inferred register has a role-token preimage |

### Verified properties

- **After D, `map_token_direct` reproduces the fixpoint exactly (VERIFIED by the zero
  audit — E's precondition).** Zero `BUG387-MISMATCH` across the corpus means, for every
  operand, direct == inferred. Deleting the inference and keeping only the direct map is
  therefore byte-identical by construction — the audit is the exhaustive equivalence
  proof over the actual corpus. (Re-run the sweep at E to confirm zero on the exact
  build being cut over.)
- **No value is secretly Category 2 (VERIFIED in plan-71-B Phase 2).** The value-level
  partition proof guarantees no value needs two conflicting tokens, so the operand-level
  zero is also a value-level zero — the deletion cannot strand a value that the fixpoint
  was staging below the token layer. (If plan-71-B found genuine reuse, its elision pass
  is installed and E relies on it; if none, no staging move is emitted.)
- **The deletion is x86-local (VERIFIED by inspection at E).** `remap_x86_abi` is called
  only from the x86 path; `git grep 'remap_x86_abi'` confirms no AArch64/RISC-V caller.
  The full gate re-proves the other four targets are byte-identical regardless.

## 3. Design Overview

A single, high-blast-radius deletion, landed last, behind the exhaustive equivalence
proof the earlier letters built:

- **Retire the deferral + delete the fixpoint (the correctness climax).** `select_x86`
  stops skipping role tokens; it realizes ABI role tokens through `map_token_direct` and
  everything else exactly as today, in one pass. `remap_x86_abi` and
  `remap_x86_abi_inner` (the 587-line block + its audit machinery) are deleted, along
  with the now-dead deferral branch. This is the bug-85 surface — the exact change that
  reverted before — so it lands **only** after the audit proved (C/D) and the value
  partition proved (B) that direct == inferred everywhere.

- **Flip the cross-check live, then keep the assert (Open Decision 2).** As a transition
  safety net, land the cutover in two moves if desired: (1) make the live path take
  `map_token_direct` while an `assert_eq!(direct, mapped)` still runs (fixpoint retained
  one commit) to catch any last discrepancy on the actual build; (2) delete the fixpoint,
  leaving `map_token_direct` as the sole map and retaining a cheap
  `debug_assert`/unit-test guard against future token regressions.

- **Reconcile the record.** Update `src/docs/spec/architecture/` register-role
  vocabulary to describe the direct map (no CFG inference); close
  `bug-387` and note in `bug-85`/`plan-34-B` that the direct lookup landed under the
  byte-identity gate that the prior attempt lacked.

**Where correctness risk concentrates:** here, entirely — the codegen path every x86
program uses. It is mitigated by (a) the exhaustive per-operand audit at zero, (b) the
value-level partition proof, (c) the two-step live-assert cutover, (d) the five-target
byte-identity gate, and (e) remote runtime re-probe. **Where design uncertainty
concentrates:** none remains — B/C/D removed it.

Rejected alternatives:

- *Delete the fixpoint before the audit is zero.* Rejected: any surviving mismatch is a
  byte move; the deletion is gated on D's zero.
- *Delete the cross-check entirely at the end.* Rejected (Open Decision 2): keep a cheap
  assert/unit-test guard so a future token regression is caught, not silently
  miscompiled. It may be removed later if it proves hot, once confidence is established.
- *Re-baseline goldens if a byte moves.* Rejected: byte-identity is the law; a moved byte
  means the equivalence proof was incomplete — re-open the prior letter.

## 4. Detailed Design

1. **Live-assert transition (Phase 1).** In `remap_x86_abi_inner`, change the write path
   from `mapped` to `map_token_direct(token, abi)` for role-token operands, and add a
   live `assert_eq!(direct, mapped)` (not env-gated) so a debug build of the whole corpus
   proves direct == mapped on the exact tree. Run `cargo test`, `bug387-gate.sh … full`,
   and a debug corpus build; a panic anywhere means a surviving discrepancy (re-open D).
2. **Delete the fixpoint (Phase 2).** Remove `remap_x86_abi` / `remap_x86_abi_inner`
   (the 587-line CFG block + audit machinery) and `select_x86`'s deferral branch; make
   `select_x86` realize role tokens through `map_token_direct` in its single pass. Retain
   a `debug_assert`/unit-test guard on `map_token_direct` (Open Decision 2). Prove
   byte-identity on all five targets.
3. **Reconcile spec + bugs (Phase 3).** Update `src/docs/spec/architecture/`; close
   `bug-387`; annotate `bug-85`/`plan-34-B`.
4. **Runtime re-probe (Phase 4).** On the remote GTK boxes (2228/2227) and Windows box
   (2230), run `scripts/test-appimage.sh --libc both` and the Windows runtime suite;
   confirm real execution, not just byte-identity.

## Compatibility / Format Impact

None observable. E deletes an internal register-coloring pass and replaces it with a
table lookup that produces the identical bytes. No API, file/wire format, layout/ABI, or
config changes. The only internal change: `remap_x86_abi` ceases to exist; any internal
caller/test referencing it is updated in the same commit.

## Phases

> Keep the checkboxes current in the same commit as the work. An unticked box means
> NOT DONE.

### Phase 1 — flip the cross-check live (fixpoint retained one commit)

Proves direct == mapped on the exact build before anything is deleted.

- [ ] Re-record clean-`main` exe-oracle baselines (all five targets) and re-run the full
      `MFB_BUG387_AUDIT` sweep; confirm **0** mismatches on `linux-x86_64` and
      `windows-x86_64` on the build about to be cut over.
- [ ] In `remap_x86_abi_inner`, write `map_token_direct(token, abi)` on the role-token
      path and add a live `assert_eq!(direct, mapped)`; keep the fixpoint computing
      `mapped`.
- [ ] Tests: extend `src/arch/x86_64/select::tests` — the live assert holds on the clean
      cases; construct no reuse case (D guarantees none).
- [ ] Gate: `cargo test --bin mfb` real `test result: ok`; a **debug** corpus build
      completes with no assert panic; `bug387-gate.sh … full` byte-identical (five
      targets).

Acceptance: audit = 0 on both x86 targets on the cutover build; live assert holds across
a full debug corpus build; `bug387-gate.sh … full` PASS; full suite green.
Commit: —

### Phase 2 — delete the fixpoint

Largest blast radius; lands only after Phase 1's live assert held.

- [ ] Delete `remap_x86_abi` / `remap_x86_abi_inner` (the 587-line CFG block + audit
      machinery) and `select_x86`'s role-token deferral; realize role tokens through
      `map_token_direct` in `select_x86`'s single pass. Retain a `debug_assert`/unit-test
      guard on `map_token_direct` (Open Decision 2).
- [ ] Update/remove any test or caller referencing the deleted symbols in the same
      commit; no `#![allow(dead_code)]` — delete what is dead.
- [ ] Gate: `bug387-gate.sh … full` byte-identical on all five targets;
      `scripts/exe-oracle.sh` base-vs-mine byte-identical (five targets);
      `scripts/artifact-gate.sh` 0 diffs (if no concurrent run holds it — project forbids
      concurrent artifact-gate).

Acceptance: fixpoint gone; `bug387-gate.sh … full` PASS (byte-identical, five targets);
exe-oracle base-vs-mine byte-identical on all five; `cargo test --bin mfb` real
`test result: ok`; artifact-gate 0 diffs.
Commit: —

### Phase 3 — reconcile spec + bug record

- [ ] Update `src/docs/spec/architecture/` register-role vocabulary to describe the
      direct map (no CFG inference), per `.ai/specifications.md`.
- [ ] Close `bug-387`; annotate `bug-85`/`plan-34-B` that the direct lookup landed under
      the byte-identity gate the prior attempt lacked.

Acceptance: spec reflects the direct map; bug-387 moved to `completed-bugs/`; bug-85 /
plan-34-B annotated. Spec-citation sweeps green (per the project's citation rules).
Commit: —

### Phase 4 — remote runtime re-probe

- [ ] On GTK boxes 2228/2227: `scripts/test-appimage.sh --libc both` — real execution
      green.
- [ ] On Windows box 2230: the Windows runtime suite — real execution green.

Acceptance: remote runtime green on both GTK boxes and the Windows box (or, if a box is
down, byte-identity stands as the definitive proof for a zero-byte-change deletion and
the down box is recorded — plan-71-A's stance).
Commit: —

## Validation Plan

- Tests: `src/arch/x86_64/select::tests` gains the live-assert equivalence assertions
  (Phase 1) and retains the `map_token_direct` table tests as the post-deletion guard.
- Coverage check: the deletion is on the path every x86 program uses; the full
  `bug387-gate.sh … full` (whole executables) is a strict superset of artifact-gate's
  package-object check, so a green gate means nothing *covered* moved.
- Runtime proof: `scripts/test-appimage.sh --libc both` on the GTK boxes and the Windows
  runtime suite on box 2230 — real end-to-end execution beyond byte-identity.
- Doc sync: `src/docs/spec/architecture/` register-role vocabulary; bug-387 closed;
  bug-85 / plan-34-B annotated.
- Acceptance: `cargo test --bin mfb` real `test result: ok`; `scripts/bug387-gate.sh
  <exe> full` PASS (five targets); `scripts/exe-oracle.sh` base-vs-mine byte-identical
  (five targets); `scripts/artifact-gate.sh` 0 diffs (no concurrent run); remote runtime
  green (Phase 4).

## Open Decisions

- **Keep or remove the cross-check after deletion** (plan-71-A Open Decision 2) — keep a
  cheap `debug_assert`/unit-test guard on `map_token_direct` as a regression net vs.
  remove it once the fixpoint is gone. Recommend: keep the guard (cheap; catches future
  token regressions), remove only if it proves hot. (§4)
- **One-commit vs. two-commit cutover** — flip-live-with-assert then delete (two
  commits, a safety checkpoint) vs. delete-and-flip in one. Recommend: two commits, so
  the live assert is proven on the real corpus build before the fixpoint is removed.
  (§Phase 1/2)
- **Down remote box** — if a GTK/Windows box is unreachable at Phase 4, treat
  byte-identity as the definitive proof for a zero-byte-change deletion and record the
  down box, vs. block on it. Recommend: record and proceed (plan-71-A's stated stance);
  re-probe when the box returns. (§Phase 4)

## Corrections

<Filled in during execution.>

## Summary

E is where plan-71's real engineering risk lives — deleting the 587-line `remap_x86_abi`
fixpoint on the codegen path every x86 program uses, the exact surface bug-85 reverted.
It is landed last and safely because the earlier letters removed the uncertainty: B
proved no value is secretly Category 2 (and built the elision pass if it was), and C/D
re-tokenized every producer so the context-free `map_token_direct` reproduces the
fixpoint's output on **zero** divergences across the whole corpus. E flips the direct map
live under an assert, deletes the fixpoint, proves byte-identity on all five targets,
re-probes remote runtime, and reconciles the spec and the bug record — delivering
plan-71's single behavioral outcome: the fixpoint gone, every emitted byte unchanged.
