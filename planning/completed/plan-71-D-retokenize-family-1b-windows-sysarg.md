# plan-71-D: re-tokenize Family 1b tail + windows `%sysarg`

Last updated: 2026-08-02
Effort: medium (1h–2h)
Depends on: plan-71-C (Family 1a driven to zero). plan-71-C depends on plan-71-B →
plan-71-A.

This sub-plan clears the structurally-distinct **tail** the census isolated from the
Family-1a bulk: the arg-named values the fixpoint colors as *results*
(`%argK`→`RETS[K]`) and the Windows-only syscall-argument token used as a Win64 call
argument (`%sysargK`→Win64 arg). It is small (linux ~2,744 raw operands; windows
`%arg0`→rax 21,709 and `%sysarg1`→rdx 1,232 — `plan-71-census.md` §"Category 1 — 1b")
and separated from C precisely because it is *fallback-driven* and *platform-specific*,
not more of the same uniform swap.

The single behavioral outcome of plan-71-D: after D, the `MFB_BUG387_AUDIT` sweep
reports **0** mismatches of **any** family on `linux-x86_64` and `windows-x86_64`, with
every emitted byte unchanged on all five targets — the zero-divergence precondition
plan-71-E needs to delete the fixpoint under a live cross-check.

References:

- `planning/plan-71-census.md` — §"Category 1 — 1b" (the two shapes, the caution that
  the correct token here is dictated by the *inferred* register — a fixpoint fallback —
  **not** the semantic role) and the token→register transitions table (`%arg0/1/2`→RETS,
  windows `%arg0`→rax, `%sysarg1`→rdx).
- `planning/completed/plan-71-A-fixpoint-crosscheck-census.md` — §3 Category 1; the
  byte-identity Non-goal.
- `src/target/shared/abi.rs:137` `ARG`, `:144` `RET`, `:153` `SYSARG`
  (`["%sysarg0"…"%sysarg5"]`), `:327` `realize_abi_token` — the token spellings and the
  AArch64 realization (both `%argK` and `%retK`→same `xN`; `%sysargK`→`xK`).
- `src/arch/x86_64/select.rs:123` `map_abi_register` (the fixpoint's `None`-fallback
  that colors an un-inferrable `%argK` as `RETS[K]`), `:166` `map_token_direct`,
  `:208` `remap_x86_abi` — the cross-check.
- `src/target/shared/code/` — the emission sites; the Windows path that emits
  `%sysargK` for OS calls (Windows has no raw syscalls — OS calls go through the IAT).
- `scripts/bug387-gate.sh`, `scripts/artifact-gate.sh` — byte-identity gates.

## Prerequisites

The whole-feature preconditions live in plan-71-A's Prerequisites table and remain in
force. This letter additionally requires:

| Must be true | Command | Status |
|---|---|---|
| plan-71-C complete (Family 1a at zero) | `ls planning/completed/plan-71-C-*.md` | NOT MET (C not yet landed) |
| the audit reports 0 Family-1a mismatches, residual = 1b + `%sysarg` only | `MFB_BUG387_AUDIT=1` sweep → `grep 'token=%ret' … \| wc -l` = 0 | NOT MET (C Phase 3 establishes it) |
| exe-oracle baselines re-recorded from clean `main` (ephemeral `/tmp`) | `ls /tmp/bug387/oracle-windows-x86_64.txt` | RE-RECORD FIRST |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** D starts
> only once C has driven Family 1a to zero, so the *only* remaining audit mismatches are
> the 1b tail and windows `%sysarg` D targets. Re-record the ephemeral baselines first.
> If you stop, report the status of *all* rows.

Everything below is written against the world where these hold.

## 1. Goal

**plan-71-D goal:** the two tail families are re-tokenized to follow the fixpoint's
chosen register, verified byte-identical, such that:

- **1b (arg-named colored a result):** each `%argK` producer whose value the fixpoint
  colors `RETS[K]` (an un-inferrable value whose only downstream fact is a stack spill,
  so `map_abi_register`'s `None`-fallback lands it at `RETS[K]`) emits `%retK` instead —
  chosen by the *inferred* register, not the semantic role (`plan-71-census.md` §1b
  caution).
- **windows `%sysarg`:** each Windows emission of `%sysargK` used as a Win64 call
  argument emits `%argK` instead (Windows routes OS calls through the IAT, not raw
  syscalls, so a syscall-arg token there is really a call-arg token).
- The `MFB_BUG387_AUDIT` sweep reports **0** mismatches of any family on `linux-x86_64`
  and `windows-x86_64`; every emitted byte unchanged on all five targets.

**plan-71 overall goal (context, not delivered here):** delete the fixpoint (plan-71-E).

### Non-goals (explicit constraints)

- **Any emitted byte, on any target.** Every re-tokenization is byte-identical by the
  cross-check; a byte move means the site was mis-classified — reclassify, never
  re-baseline.
- **No staging move.** Category 1 is re-tokenization only (plan-71-A §3).
- **The fixpoint, `select_x86`'s realize loop, the live `map_token_direct`.** plan-71-E.
- **Semantic "correctness" of the token over byte-identity.** The 1b caution is
  load-bearing: the correct token is the one whose `map_token_direct` equals the
  fixpoint's *inferred* register (a fallback), even where that reads as semantically
  odd. Byte-identity is the bar.
- **The token vocabulary.** No new token.

## 2. Current State

After plan-71-C, the only `MFB_BUG387_AUDIT` mismatches left are:

- **Family 1b (linux + windows):** a value the builder names `%argK` whose only
  downstream fact is a stack spill with no call boundary. The fixpoint's inference finds
  no argument role, so `map_abi_register` (`src/arch/x86_64/select.rs:123`) takes its
  `None`-fallback and colors it `RETS[K]` — `%arg0`→rax, `%arg1`→rdx (rsi is
  `RETS`-shifted), `%arg2`→rcx. The direct map says `%arg1`→rsi, so it diverges; the
  byte the fixpoint fixes today is `RETS[K]`. Fix: emit `%retK` so the direct map matches
  the inferred (fallback) register. On AArch64 `%argK` and `%retK` both realize to the
  same `xN` (`realize_abi_token`, `abi.rs:327`), so the swap is byte-identical there.
- **windows `%sysarg` (windows only):** `%sysarg1`→rsi by the direct map, but the
  fixpoint colors it Win64 arg 1 = rdx, because Windows has no raw syscalls — the
  emission that reached for a syscall-arg token is really passing a Win64 call argument.
  Fix: emit `%argK` on the Windows path. `%sysarg1` and `%arg1` both realize to `x1` on
  AArch64, so byte-identical there; on x86 `map_token_direct(%arg1)=rsi`… — **verify
  against the census**: the target is the fixpoint's *inferred* register (Win64 arg 1),
  so choose the token whose direct map equals it.

### Measured populations

| What | Count | Command |
|---|---|---|
| Family 1b raw operands (linux `%arg0/1/2`) | ~2,744 | `plan-71-census.md` §1b (linux tail) |
| windows `%arg0`→rax | 21,709 | `plan-71-census.md` transitions table |
| windows `%sysarg1`→rdx | 1,232 | `plan-71-census.md` transitions table |
| distinct 1b + `%sysarg` shapes | 143−(1a linux) / 106−(1a windows) | post-C audit: `grep 'token=%arg\|token=%sysarg' distinct-*.txt \| wc -l` (MEASURE at D Phase 1) |
| exact 1b/`%sysarg` emission sites in shared code | **MEASURE FIRST** | D Phase 1 — from the post-C audit `@fixture`+`site` cross-referenced to `src/target/shared/code/` |

### Verified properties

- **1b's correct token is dictated by the inferred (fallback) register, not the role
  (VERIFIED by the census caution + the fixpoint fallback path).** `map_abi_register`'s
  `None`-branch (`select.rs:123`) colors an un-inferrable `%argK` as `RETS[K]`; the byte
  to preserve is that fallback register, so the re-tokenization follows the inference,
  not the semantics (`plan-71-census.md` §1b explicit caution). Confirm each site's
  target register against the census transition, not intuition.
- **windows has no raw syscalls (VERIFIED — platform fact, cited in the census).** A
  `%sysargK` on the Windows path is a Win64 call argument; `%sysargK`→`%argK` there is
  the fix. Linux/macOS `%sysarg` on a real syscall boundary is untouched (it does not
  diverge — no boundary op carries a divergent operand, `plan-71-census.md` §"Measured
  populations").
- **Every 1b/`%sysarg` swap is byte-identical (VERIFIED per site by the cross-check).**
  The `bug387-gate.sh` PASS after each commit is the proof; the AArch64 realization
  collision (`abi.rs:327`) guarantees no byte moves on the reuse ISAs.

## 3. Design Overview

Two small, structurally-distinct transforms, each gated byte-identical:

- **1b re-tokenization (fallback-driven).** At each site the post-C audit attributes to
  `%argK`-colored-`RETS[K]`, emit the token whose `map_token_direct` equals the fixpoint's
  inferred (fallback) register — read the census transition for that exact site; do not
  infer from the source shape.
- **windows `%sysarg` re-tokenization (platform-specific).** On the Windows emission
  path only, replace the `%sysargK` used as a Win64 call argument with `%argK`. Guard
  that the Linux/macOS syscall path is untouched.

**Where correctness risk concentrates:** the 1b "follow the inference, not the
semantics" trap — a site re-tokenized by its apparent role instead of its measured
target register would move a byte. The per-site cross-check catches it immediately.
**Where design uncertainty concentrates:** none new; the census already isolated and
characterized both shapes.

Rejected alternatives:

- *Fold 1b/`%sysarg` into C.* Rejected: they are fallback-driven and platform-specific,
  a different reasoning discipline (follow the inferred register / Windows-IAT fact)
  than C's uniform `%retK`→`%argK` swap; isolating them keeps each letter one kind of
  change (`plan-71-census.md` §"B-onward split").
- *Re-tokenize 1b by semantic role.* Rejected: the byte to preserve is the fixpoint's
  *fallback* register, which can differ from the semantic role; following the role
  would move a byte (the census caution).

## 4. Detailed Design

1. **Tail site census (Phase 1).** From the post-C `MFB_BUG387_AUDIT` sweep, list every
   remaining mismatch site (`file:line` + the `abi::ARG[K]`/`%sysargK` emission), split
   into 1b (linux + windows `%arg0`) and windows `%sysarg`, each annotated with its
   census target register.
2. **1b swap (Phase 2).** At each 1b site, emit the token whose direct map equals the
   census target register (`%argK`→`%retK` for the `RETS[K]`-fallback cases). Commit
   per site/group, gated byte-identical.
3. **windows `%sysarg` swap (Phase 3).** On the Windows path, `%sysargK`→`%argK` for the
   Win64-call-arg cases; assert the Linux/macOS syscall path is untouched. Commit, gated
   byte-identical.
4. **Convergence (Phase 4).** The audit reports 0 mismatches of any family on both x86
   targets — the plan-71-E precondition.

## Compatibility / Format Impact

None. D changes only which role token a builder emits at the tail sites; realized
encoding is identical on every target (the cross-check is the proof). No externally
observable contract changes; no emitted byte changes.

## Phases

> Keep the checkboxes current in the same commit as the work. An unticked box means
> NOT DONE.

### Phase 1 — tail site census (1b + windows `%sysarg`)

- [ ] From the post-C audit, list every remaining mismatch site (`file:line` +
      emission), split into 1b and windows `%sysarg`, each annotated with its census
      target register. Record in `plan-71-census.md` (a "D work-list" subsection).
- [ ] State the site count with its command (no `~`).

Acceptance: a complete, census-annotated tail work-list exists; the count carries its
command.
Commit: —

### Phase 2 — Family 1b re-tokenization (follow the inferred register)

- [ ] At each 1b site emit the token whose `map_token_direct` equals the census target
      register (`%argK`→`%retK` for the `RETS[K]`-fallback cases). Per-site/group
      commits.
- [ ] Gate per commit: `bug387-gate.sh … full` byte-identical; the audit's 1b mismatch
      count drops by this site's contribution.

Acceptance: audit 1b mismatch count = 0 on `linux-x86_64` and `windows-x86_64`;
`bug387-gate.sh … full` PASS; `cargo test --bin mfb` green.
Commit: —

### Phase 3 — windows `%sysarg` re-tokenization

- [ ] On the Windows emission path, `%sysargK`→`%argK` for the Win64-call-arg sites;
      assert the Linux/macOS syscall path is untouched (a test or a grep proof that no
      real-syscall `%sysarg` emission changed).
- [ ] Gate: `bug387-gate.sh … full` byte-identical (windows and the four others).

Acceptance: audit `%sysarg` mismatch count = 0 on `windows-x86_64`; Linux/macOS syscall
path unchanged; `bug387-gate.sh … full` PASS.
Commit: —

### Phase 4 — convergence: audit at zero on both x86 targets

- [ ] Confirm the `MFB_BUG387_AUDIT` sweep reports **0** mismatches of any family on
      `linux-x86_64` and `windows-x86_64` (`grep -c BUG387-MISMATCH` = 0 over the full
      corpus).
- [ ] Full `cargo test --bin mfb` real `test result: ok`; `artifact-gate.sh` 0 diffs
      (if no concurrent run).

Acceptance: zero mismatches on both x86 targets — the plan-71-E precondition; full suite
green; `bug387-gate.sh … full` PASS.
Commit: —

## Validation Plan

- Tests: the `src/arch/x86_64/select::tests` cross-check tests continue to pass; add a
  guard test that the Linux/macOS syscall `%sysarg` path is unchanged (Phase 3).
- Coverage check: the audit sweep exercises every re-tokenized tail site; zero
  mismatches over the full corpus is the coverage proof.
- Runtime proof: byte-identity across five targets; runtime confirmation at plan-71-E.
- Doc sync: update `plan-71-census.md` with the D work-list and the drop to zero
  mismatches. No spec change.
- Acceptance: per-commit `bug387-gate.sh … full` PASS; final `cargo test --bin mfb` real
  `test result: ok`; `scripts/artifact-gate.sh` 0 diffs if no concurrent run.

## Open Decisions

- **1b target-token derivation** — read each site's target register from the census
  transition table vs. re-run the audit per site to confirm. Recommend: derive from the
  census, verify by the post-commit `bug387-gate.sh` (a byte move = wrong token). The
  census caution (follow the inference) governs. (§4)
- **windows syscall-path guard** — a unit test vs. a grep proof that no real-syscall
  `%sysarg` emission changed. Recommend: a unit test if the emission path has a
  testable seam; otherwise a documented grep proof in the commit. (§Phase 3)

## Corrections

<Filled in during execution.>

## Summary

D clears the fallback-driven and platform-specific tail C deliberately left: `%argK`
producers the fixpoint colors `RETS[K]` (re-tokenized `%retK`, following the *inferred*
register, not the role) and the Windows-only `%sysargK`-as-Win64-arg (re-tokenized
`%argK`). Small in volume but distinct in reasoning, so isolated from C's uniform swap.
Every change is byte-identical by the cross-check; the one trap — following semantics
instead of the measured target register — is caught per-site by the byte-identity gate.
D's product is a **zero-divergence** audit on both x86 targets, the precondition
plan-71-E needs to delete the fixpoint under a live `assert_eq!`.
