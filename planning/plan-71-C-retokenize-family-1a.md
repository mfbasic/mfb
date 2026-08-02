# plan-71-C: re-tokenize Family 1a (result-named value used as an argument)

Last updated: 2026-08-02
Effort: large (3h–1d)
Depends on: plan-71-B (the verified value-level Category-1/Category-2 partition; and,
if the probe found reuse, the AArch64/RISC-V self-move elision pass). plan-71-B
depends on plan-71-A.

This sub-plan does the bulk of the fixpoint-removal preparation: it re-tokenizes every
**Family 1a** producer — a value the shared builders emit as `%retK` (via
`abi::RET[K]` / `abi::return_register()`) but which is actually consumed as call
argument K — to emit `%argK` (`abi::ARG[K]`) instead. This is ~99.7% of the divergence
audit's raw operands (`plan-71-census.md` §"Category 1 — 1a"): linux
`%ret0..3`→`rdi/rsi/rdx/rcx` = 1,031,578 raw operands, windows `%ret0`→`rcx` = 461,467.

The single behavioral outcome of plan-71-C: after C, the `MFB_BUG387_AUDIT` sweep
reports **zero Family-1a mismatches** on `linux-x86_64` and `windows-x86_64`, and every
emitted byte is unchanged on all five targets. Re-tokenizing `%retK`→`%argK` is
byte-identical because on x86 `map_token_direct(%argK)` equals the register the fixpoint
already inferred (that is what "divergent" meant), and on AArch64/RISC-V `%argK` and
`%retK` realize to the **same** `xN` (`realize_abi_token`, `abi.rs:327`) — so no
instruction is added and no encoding moves.

References:

- `planning/plan-71-census.md` — §"Category 1 — 1a" (the population, the transitions
  table, representative distinct sites) and §"B-onward split" (this letter's scope).
- `planning/completed/plan-71-A-fixpoint-crosscheck-census.md` — §3 Category 1
  (colorable; re-tokenize, no move) and the Non-goal that Category 1 MUST be
  re-tokenization, never staging (adding a `mov` on x86 would break byte-identity).
- `src/target/shared/abi.rs:137` `ARG` (`["%arg0"…"%arg7"]`), `:144` `RET`
  (`["%ret0"…"%ret3"]`), `:12` `argument_register`, `:93` `return_register`,
  `:443` `move_register`, `:327` `realize_abi_token` — the token spellings and the
  AArch64 realization that makes the swap byte-identical.
- `src/target/shared/code/` — the shared builders that emit the tokens; **72** files
  reference `abi::ARG[` / `abi::RET[` (`grep -rlE 'abi::(ARG|RET)\[' src/target/shared/code/ | wc -l → 72`).
- `src/arch/x86_64/select.rs:166` `map_token_direct`, `:199` `is_abi_role_token`,
  `:208` `remap_x86_abi` — the cross-check that verifies each re-tokenization is
  byte-identical, and reports the remaining mismatch count.
- `scripts/bug387-gate.sh`, `scripts/artifact-gate.sh` — the byte-identity gates.
- `.ai/compiler.md` — the completion gate; register/codegen work rules.

## Prerequisites

The whole-feature preconditions live in plan-71-A's Prerequisites table and remain in
force. This letter additionally requires:

| Must be true | Command | Status |
|---|---|---|
| plan-71-B complete (value-level partition proven; elision pass landed iff reuse exists) | `ls planning/completed/plan-71-B-*.md` | NOT MET (B not yet landed) |
| the value-level Category-1 partition is recorded proven | `grep -n 'proven-at-the-value-level' planning/plan-71-census.md` | NOT MET (B Phase 2 writes it) |
| exe-oracle baselines re-recorded from clean `main` (ephemeral `/tmp`) | `ls /tmp/bug387/oracle-linux-x86_64.txt /tmp/bug387/oracle-windows-x86_64.txt` | RE-RECORD FIRST |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** C cannot
> start until plan-71-B is complete and the value-level partition is recorded proven —
> that is a precondition, not scope C absorbs. C must **not** re-tokenize a producer B
> has not cleared as Category 1; doing so on a secretly-Category-2 value would need a
> staging move C is forbidden to add. Re-record the ephemeral baselines first. If you
> stop, report the status of *all* rows.

Everything below is written against the world where these hold.

## 1. Goal

**plan-71-C goal:** every Family-1a producer emits the argument-role token, verified
byte-identical, such that:

- At each shared-builder site where the census attributes a `%retK`→arg-K divergence,
  the emission uses `abi::ARG[K]` (`%argK`) instead of `abi::RET[K]` /
  `abi::return_register()` — for values B's partition proved are pure call-argument
  producers.
- The `MFB_BUG387_AUDIT` sweep reports **0** Family-1a (`%retK`-used-as-arg) mismatches
  on `linux-x86_64` and `windows-x86_64` (the remaining mismatches are Family 1b /
  windows `%sysarg`, which plan-71-D clears).
- Every emitted byte is unchanged on all five targets (`bug387-gate.sh … full` PASS,
  `artifact-gate.sh` 0 diffs).

**plan-71 overall goal (context, not delivered here):** delete the fixpoint (plan-71-E).

### Non-goals (explicit constraints)

- **Any emitted byte, on any target.** Each re-tokenization is byte-identical by the
  cross-check; a site that moves a byte is either not Family 1a or was mis-attributed —
  stop and reclassify, never re-baseline a golden.
- **No staging move, ever.** Category 1 is re-tokenization only. Emitting `mov %argK,%retK`
  here would add a `mov` on x86 that today does not exist → breaks x86 byte-identity
  (plan-71-A §3 rejected alternative). If a "producer" cannot be re-tokenized without a
  move, it is Category 2 — B should have caught it; escalate, don't force it.
- **Family 1b and windows `%sysarg`.** Those are plan-71-D (arg-named-colored-result
  and the Windows-only syscall-token-as-arg tail).
- **The fixpoint, `select_x86`'s realize loop, `map_token_direct` as the live map.**
  All plan-71-E. C leaves the fixpoint in place and merely drives its Family-1a
  divergences to zero.
- **The token vocabulary.** No new token; only `RET[K]`↔`ARG[K]` swaps at emission
  sites.

## 2. Current State

The shared builders under `src/target/shared/code/` emit ABI tokens through the `abi`
helpers: `abi::RET[K]` / `abi::return_register()` (= `RET[0]`, `abi.rs:144`,
`["%ret0"…"%ret3"]`) for result-role values, `abi::ARG[K]` (`abi.rs:137`,
`["%arg0"…"%arg7"]`) for argument-role values. Many builder sites emit a value into
`return_register()` and then flow it straight into a call as argument K — e.g.
`src/target/shared/code/builder_search.rs:781`
`abi::add_immediate(abi::return_register(), byte_len, 9)` produces into `%ret0` a value
subsequently consumed as arg 0. The x86 `remap_x86_abi` fixpoint recovers the true
argument role and colors `rdi`; the census (`plan-71-census.md`) measures every such
operand as a `%ret0`→`rdi` divergence.

On AArch64 both spellings realize to `x0` (`realize_abi_token`, `abi.rs:327`:
`"%arg0" | "%ret0" … => "x0"`), so the value already lives in the right register
regardless of which token names it — which is why swapping `%ret0`→`%arg0` is
byte-identical there. On x86, `map_token_direct(%arg0)=rdi` equals the fixpoint's
inferred register at exactly the divergent sites — so the swap is byte-identical there
too, and it removes the divergence.

### Measured populations

| What | Count | Command |
|---|---|---|
| files referencing `abi::ARG[`/`abi::RET[` in shared code | 72 | `grep -rlE 'abi::(ARG\|RET)\[' src/target/shared/code/ \| wc -l` |
| `abi::RET[`/`return_register(` emission sites in shared code | **MEASURE FIRST** | C Phase 1 census — `grep -rncE 'abi::RET\[\|return_register\(' src/target/shared/code/` per file; only the subset flowing to a call-arg is Family 1a |
| Family 1a raw operands (linux) | 1,031,578 | `plan-71-census.md` §1a (`%ret0..3`→arg) |
| Family 1a raw operands (windows) | 461,467 | `plan-71-census.md` §1a (`%ret0`→rcx) |
| distinct Family-1a shapes to re-tokenize | 143 linux / 106 windows (superset; 1b subtracted in D) | `plan-71-census.md` §"Measured populations" distinct shapes |

### Verified properties

- **`%retK`→`%argK` is byte-identical on all five targets for a true call-argument
  producer (VERIFIED by construction + cross-check).** AArch64/RISC-V: both realize to
  the same `xN` (`realize_abi_token` read above). x86: at a divergent site
  `map_token_direct(%argK)` equals the fixpoint's inferred register (definition of
  "divergent"), so the swap makes the direct map and the fixpoint agree — the cross-check
  reports the mismatch gone with no byte change. The `bug387-gate.sh` PASS after each
  commit is the per-site proof.
- **Not every `abi::RET[K]` site is Family 1a (VERIFIED conceptually; MEASURE per-site).**
  A value genuinely returned to the caller as the function result must stay `%retK`.
  Only sites whose value is consumed as a *call argument* are Family 1a. The
  discriminator is the census attribution (the site appears as a `%retK`→arg divergence)
  **and** B's value-level partition (the value is a pure arg producer) — never a guess
  from the emission shape alone.
- **The safety of the bulk swap rests on B's value-level partition (VERIFIED there, not
  here).** C re-tokenizes only producers B cleared; a producer B flagged as
  possibly-Category-2 is out of scope until resolved.

## 3. Design Overview

One uniform, mechanical transform applied per-file with byte-identity gating between
commits:

- **Per-file re-tokenization (the bulk; mechanical but high-volume).** For each shared
  builder file the census implicates, swap the Family-1a emissions `abi::RET[K]` /
  `abi::return_register()` → `abi::ARG[K]`, guided by the census attribution and B's
  partition. Commit per-file (or per small group of related builders), each gated
  `bug387-gate.sh` byte-identical, so a mis-attributed site is caught at the first
  commit that moves a byte — not batched into an un-bisectable churn.

**Where design uncertainty concentrates:** already resolved — B proved the partition;
the census proved the transform is uniform. **Where correctness risk concentrates:**
volume, not novelty. The risk is a single mis-classified site (a genuine function-result
value re-tokenized as an argument, which *would* move a byte). The per-file byte-identity
gate localizes it to one commit.

Rejected alternatives:

- *One tree-wide sed of `RET[`→`ARG[`.* Rejected: not every `RET[K]` is Family 1a (a
  genuine result must stay `%retK`); a blind swap breaks byte-identity and is
  un-bisectable. The transform is per-site, census-guided, gated per file. (This is also
  the memory-recorded "never run tree-wide scripts unchecked" rule.)
- *Split C by subsystem into C/D/E…* Rejected unless execution proves the file volume
  unwieldy: the census shows one uniform transform (`plan-71-census.md` §"B-onward
  split": *"the census shows one uniform transform, so keep it one letter"*). Family 1b
  and windows `%sysarg` are already carved into plan-71-D because they are structurally
  distinct, not merely more of the same.

## 4. Detailed Design

1. **Site census (Phase 1).** Produce, from the `MFB_BUG387_AUDIT` sweep's `@fixture`
   + `site:` fields cross-referenced against `src/target/shared/code/`, the exact list
   of source emission sites (`file:line`, the `abi::RET[K]`/`return_register()` call)
   whose value is a Family-1a call-argument producer. Group by file. This is the C
   work-list; every entry carries the census line that justifies it and B's partition
   clearance.
2. **Per-file swap (Phase 2..N).** For each file in the work-list, change the implicated
   emissions to `abi::ARG[K]`. Leave every genuine-result emission untouched. Commit,
   run `bug387-gate.sh … full`, confirm byte-identical, and confirm the audit's
   Family-1a mismatch count dropped by exactly this file's contribution.
3. **Convergence check (final phase).** After the work-list is exhausted, the audit
   reports 0 Family-1a mismatches on both x86 targets; the residual mismatches are
   exactly Family 1b + windows `%sysarg` (plan-71-D's scope), confirmed by
   `grep 'token=%arg' / 'token=%sysarg'` on the post-C audit output.

## Compatibility / Format Impact

None. C changes only which role token a builder emits at Family-1a sites; the realized
encoding is identical on every target (the cross-check is the proof). No externally
observable contract changes; no emitted byte changes.

## Phases

> Keep the checkboxes current in the same commit as the work. An unticked box means
> NOT DONE.

### Phase 1 — Family-1a site census (the work-list)

- [ ] From the `MFB_BUG387_AUDIT` sweep, map every Family-1a divergence to its source
      emission site in `src/target/shared/code/` (`file:line` + the `abi::RET[K]` /
      `return_register()` call), cross-checked against B's value-level partition.
      Record the grouped-by-file work-list (each entry with its census justification) in
      `plan-71-census.md` (a new "C work-list" subsection) or a sibling doc.
- [ ] State the measured site count with its command (no `~`).

Acceptance: a complete, per-file, census-justified list of Family-1a emission sites
exists, each cleared by B's partition; the count carries its command.
Commit: —

### Phase 2 — per-file re-tokenization (repeat until the work-list is empty)

Each file (or small related group) is one landable, byte-identical commit.

- [ ] Swap Family-1a emissions `abi::RET[K]`/`return_register()` → `abi::ARG[K]` in the
      file; leave genuine-result emissions untouched.
- [ ] Gate: `bug387-gate.sh … full` byte-identical on all five targets; the audit's
      Family-1a mismatch count drops by this file's contribution.
- [ ] Tick the work-list entry in the same commit.

Acceptance (per commit): `bug387-gate.sh … full` PASS (byte-identical); audit Family-1a
count strictly decreased; `cargo test --bin mfb` green.
Commit: — (one per file/group; list them here as they land)

### Phase 3 — convergence: Family 1a at zero

- [ ] Confirm the `MFB_BUG387_AUDIT` sweep reports **0** Family-1a (`%retK`-as-arg)
      mismatches on `linux-x86_64` and `windows-x86_64`.
- [ ] Confirm the residual mismatches are exactly Family 1b + windows `%sysarg`
      (plan-71-D scope), with the command that shows it.
- [ ] Full `cargo test --bin mfb` real `test result: ok`; `artifact-gate.sh` 0 diffs
      (if no concurrent run holds it).

Acceptance: audit Family-1a count = 0 on both x86 targets; residual = Family 1b +
`%sysarg` only; `bug387-gate.sh … full` PASS; full suite green.
Commit: —

## Validation Plan

- Tests: the existing `src/arch/x86_64/select::tests` cross-check tests continue to
  pass; no new unit test is needed for a re-tokenization (byte-identity is the proof),
  but any builder with a dedicated codegen golden re-runs unchanged.
- Coverage check: the audit sweep exercises every re-tokenized site (each was a measured
  divergence); a green `bug387-gate.sh` means nothing *covered* moved.
- Runtime proof: byte-identity across all five targets is the proof for a
  re-tokenization; runtime confirmation is deferred to plan-71-E's remote-box re-probe.
- Doc sync: update `plan-71-census.md` with the C work-list and the drop to zero
  Family-1a mismatches. No spec change (vocabulary unchanged).
- Acceptance: per-file `bug387-gate.sh … full` PASS; final `cargo test --bin mfb` real
  `test result: ok`; `scripts/artifact-gate.sh` 0 diffs if no concurrent run.

## Open Decisions

- **Commit granularity** — one commit per file vs. per small group of related builders
  (e.g. the four `builder_arena_transfer.rs` sites together). Recommend: per file, or
  per tightly-related group where the census shows the same shape, so a byte-move is
  bisectable to a single reviewable change. (§4)
- **Work-list location** — extend `plan-71-census.md` vs. a sibling `plan-71-C-worklist.md`.
  Recommend: a subsection of `plan-71-census.md`, keeping the measurement and its
  consumption in one place. (§Phase 1)

## Corrections

<Filled in during execution.>

## Summary

C is the high-volume, low-novelty heart of the fixpoint-removal prep: re-tokenize every
Family-1a producer (`%retK`→`%argK`) so the direct map and the fixpoint agree, driving
~99.7% of the divergence audit to zero. Every swap is byte-identical by the cross-check
(same `xN` on AArch64/RISC-V, same inferred register on x86), gated per file so a
mis-classified site is caught at its own commit. The correctness risk is volume and
mis-attribution, not mechanism; it rests on B's value-level partition and is contained
by the per-file byte-identity gate. C touches no fixpoint, no vocabulary, and no emitted
byte — it only relabels producers the census and B proved are call-argument values.
