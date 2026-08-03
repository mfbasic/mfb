# plan-71-C: re-tokenize Family 1a (result-named value used as an argument)

Last updated: 2026-08-03
Effort: large (3h–1d)
Depends on: plan-71-B (the verified value-level Category-1/Category-2 partition; and,
if the probe found reuse, the AArch64/RISC-V self-move elision pass). plan-71-B
depends on plan-71-A.

---

## FINAL STATUS — plan-71 CLOSED 2026-08-03: byte-identical cleanup landed; fixpoint deletion DEFERRED

plan-71 is being **closed without deleting the fixpoint.** The byte-identical
re-tokenization cleanup landed and merged to main; the fixpoint deletion is
**deferred to a successor plan** built on the register-bank-alignment finding
below. Reason: the plan's core premise — *"re-tokenize every divergence to zero
byte-identically, then delete the fixpoint"* — was **falsified for the majority of
the remaining work** (a Prerequisites-class defect: the entry gate never tested it).

### What landed (merged to main)
- **plan-71-B**: the value-level Category-1/2 partition proof + the `selfmove_probe`
  (Category-2 probe). Archived to `planning/completed/`.
- **The Phase-0 source-instrumentation tool**: `#[track_caller]` on the `abi` helpers
  + a `source` field on `MirInstruction`/`CodeInstruction` + the `@src` audit line,
  and the `selfmove_probe` module. All env-gated (`MFB_BUG387_AUDIT` /
  `MFB_BUG387_SELFMOVE`), **byte-identical when off**.
- **The arena / index-0 re-tokenizations** (`%ret0 → %arg0` at ~14 arena-alloc/free
  argument-producer sites): byte-identical, gate-verified per batch. Audit divergences
  fell from the plan-71-A census (~1.08M raw operands) to **272,012** last measured.
- Merges of main: plan-79/82 (typed operands), plan-80 (unified resource header),
  plan-76-D (async http). All conflicts resolved; `cargo test` = 3776 passed.

### Why the deletion was deferred — the falsified premise
The x86 SysV divergences split cleanly by ABI register **index**:
- **Index 0** (`%ret0`↔`%arg0` = rax vs rdi): the bulk by count, but a **single-role
  mislabel** — a value used only as an argument that the builder labeled `%ret`. This
  is byte-identically re-tokenizable, and is the work that **landed**.
- **Indices 1–3** (the ~192K "error-Result residual"): **dual-role** values — a
  `Result`'s {value, message, source} that are genuinely BOTH a return value and a
  call argument, threaded through spill/restore. These **cannot** be driven to zero by
  re-tokenization — **proven twice** (`emit_park_error_block_from_registers` and
  `store_pending_current_result` both *spiked* divergences when their spill sites were
  re-tokenized to `ARG[k]`). Here the fixpoint does genuine context-sensitive work.

### The fix that WILL work (successor plan) — register-bank alignment
Root cause: on x86 SysV, `RET[k] ≠ ARG[k]` for k=1..3, whereas on ARM/RISC-V they are
the **same** register (`x_k`). Redefine the SysV RET bank to align with ARG — exactly
as **Win64 already does**:

    SysV today:  ARG=[rdi,rsi,rdx,rcx]  RET=[rax,rdx,rcx,rsi]   (misaligned 1..3)
    Win64 today: ARG=[rcx,rdx,r8,r9]    RET=[rax,rdx,r8,r9]     (ALIGNED 1..3 already)
    SysV fix:    ARG=[rdi,rsi,rdx,rcx]  RET=[rax,rsi,rdx,rcx]   (aligned 1..3)

**Evidence it works:** the census shows **Windows has ZERO index-1..3 divergences**,
precisely because Win64's RET is already ARG-aligned. Aligning SysV's RET the same way
dissolves the entire error-Result residual — **no custom helper ABI, no staging moves.**
- **Index 0 is irreducible**: `rax` (C return) and `rdi` (C arg-0) are both C-ABI-forced
  and distinct — no permutation coincides them (Win64 keeps the same index-0 mismatch,
  rax vs rcx). So index 0 stays as the byte-identical re-tokenization already landed.
- **Scope/cost:** byte-**CHANGING** on **linux-x86 (SysV) only** — ARM/RISC-V and Win64
  are already aligned and unchanged. So it is OUTSIDE plan-71's byte-identity rule and
  needs its own plan. Work: redefine `RETS` in `src/arch/x86_64/select.rs`; update the
  x86 `_mfb_*` helpers that return their 2nd value in `rdx`; **audit the SysV `rax:rdx`
  two-value-return dependency** (FFI / entry / runtime helpers) FIRST; regenerate
  linux-x86 goldens; then delete the fixpoint (the direct map suffices).

### Ledger note
The unticked `- [ ]` boxes in C (indices 1–3), D, and E below are **DEFERRED, not
done** — superseded by the bank-alignment approach and carried into the successor
plan. C's index-0 re-tokenization is partial-but-byte-identical and landed. This
closure is the honest state; the boxes are left as-written (not falsely ticked) and
the docs are archived to `planning/completed/`.

---

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

- [x] **Phase 0 tool (was a hidden prerequisite — see Corrections):** built audit
      source-location instrumentation (`@src=file:line`) so the mapping is mechanical.
      Commit `bac02f1c6`; byte-identical (metadata only).
- [x] Mapped every Family-1a divergence to its source emission site via the tool +
      `audit2-sweep.sh`: **115 distinct `@src` sites / 20 files** on linux-x86_64
      (1,082,777 mismatches, 0 without a source). Recorded per-file in `plan-71-census.md`
      §"C work-list" with the derivation command.
- [x] Site count stated with its command (see census; 115 sites, `grep -oE '@src=…' | sort
      -u | wc -l`). NOTE: 18 sites resolve to `abi.rs` (the `.push`/`.extend` paths capture
      the helper line, not the builder) — refine by making the `abi::` emit helpers
      `#[track_caller]` before re-tokenizing those (a minority).

Acceptance: a complete, per-file, deterministically-derived list of Family-1a emission
sites exists (115 sites), each to be cleared per-site against B's partition + the
byte-identity gate in Phase 2; the count carries its command. **MET** (modulo the 18
`abi.rs` sites' refinement, tracked above).
Commit: `bac02f1c6` (Phase-0 tool) + this doc.

### Phase 2 — per-file re-tokenization (repeat until the work-list is empty)

Each file (or small related group) is one landable, byte-identical commit.

- [~] Swap Family-1a emissions `return_register()`/`RET[K]` → `abi::ARG[K]` per work-list
      site; leave genuine-result emissions untouched. **IN PROGRESS.** The dominant transform
      is the **arena-call arg producer**: `load_u64(return_register(), sp, size_slot)` (or a
      ptr) immediately before `emit_arena_alloc_call`/`emit_arena_free_call` — the value is
      arg 0, so emit `ARG[0]`. Only re-tokenize sites the audit `@src` flagged (NOT every
      `return_register()`-before-arena-call — many are genuine). Progress (linux-x86_64
      total mismatches):
      - `builder_owned_cleanup.rs` (187/193/201, arena-free ptr) — DONE, gated PASS, commit
        `3b29873c5`. 1,082,777 → 635,000 (−448K).
      - `builder_error_emission.rs:278` (error-block alloc size) + `builder_collection_layout.rs:332`
        (flat-copy alloc size) — DONE, gated PASS, commit `749593d66`. 635,000 → 323,452.
      - **Running total: 1,082,777 → 323,452 (−70%) with 3 files (all byte-identity gated).**
      - Remaining tail (re-sweep `audit5`): `builder_error_emission.rs` many sites (80, 463–479,
        718–728, 740–755, 981–996, 94/96 — the error-Result construction, varied — NOT all the
        simple arena idiom; each needs per-site reasoning), `builder_collection_layout.rs:944`
        + a few more alloc sites, and the `abi.rs` sites (450/796/444, ~42K) which need the
        `abi::` emit-helper `#[track_caller]` refinement to pin the builder before re-tokenizing.
      **NOTE:** `@src` line numbers drift as edits change line counts — re-sweep after each batch;
      the total-mismatch delta is the progress metric, not fixed line numbers.
- [~] Gate: `bug387-gate.sh … full` byte-identical on all five targets per file/group; the
      audit's Family-1a count drops by that group's contribution. (owned_cleanup gated PASS.)
- [ ] Tick each work-list entry as its file lands.

Acceptance (per commit): `bug387-gate.sh … full` PASS (byte-identical); audit Family-1a
count strictly decreased; `cargo test --bin mfb` green.
Commit: `3b29873c5` (owned_cleanup) — more per file/group as they land.

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

- **Prereqs MET (2026-08-02):** plan-71-B archived (`planning/completed/plan-71-B-*.md`,
  commit `3c6e4fc3a`); value-level partition proven (`grep 'proven-at-the-value-level'
  plan-71-census.md` = 1); clean serial baselines present (linux-x86_64 1320, windows 630,
  riscv 1318, aarch64 1320 in `/tmp/bug387/oracle-*.txt`). Fresh audit re-captured on
  current `main` (post-plan-78): linux-x86_64 1,096,094 mismatches / 79 distinct shapes,
  windows-x86_64 513,699 (`/tmp/bug387/audit2-*.txt`) — same 7 Category-1 families as the
  plan-71-A census, so C's scope is unchanged.

- **Phase 1 METHOD DEFECT — the work-list is not mechanically derivable as written.**
  Phase 1 says "cross-reference the `@fixture` + `site:` fields against
  `src/target/shared/code/`" to get each Family-1a `file:line`. That is **not achievable**:
  the `BUG387-MISMATCH` line carries only the op + *post-realization* operands (e.g.
  `add_imm [dst=%ret0, src=r10, imm=33]` — `src` is a realized scratch reg, not a source
  variable), NOT a source location. Grep-mapping a shape back to source is **ambiguous** —
  there are **659** `return_register()`/`RET[K]` producer emissions across 72 files but only
  **~79** divergent shapes, and multiple sources share an op+imm (e.g. `imm=9` matches 5+
  distinct builder lines in `builder_strings_builtins.rs`/`builder_search.rs`/`float_format.rs`).
  Blindly re-tokenizing all 659 would corrupt genuine *result* producers (a value truly
  returned to the caller must stay `%retK`). **Required prerequisite tool (Phase 0, added):**
  instrument the audit to emit the SOURCE `file:line` of each divergent CodeInstruction —
  add a metadata `source: Option<&'static Location>` to `CodeInstruction`, capture
  `Location::caller()` at construction with `#[track_caller]` propagating through the
  `abi::` emit helpers, and print it in the `BUG387-MISMATCH` line. That turns the work-list
  into a deterministic `grep 'file:line' | sort | uniq -c` over one audit sweep. This tool
  is byte-identical (metadata only, never emitted). Until it lands, Phase 1's work-list
  cannot be produced without unsafe guessing. **This is the next task.**

- **Phase 2 progress + the two-tier structure of the work-list (measured).** Tool + abi
  refinement landed (`bac02f1c6`, plus 95 `abi::` helpers `#[track_caller]`); work-list is
  fully precise. Re-tokenized the **uniform arena-call arg** tier — `load/compute a
  pointer/size into `return_register()` immediately before `emit_arena_{alloc,free}_call`
  (or a `branch_link`), where the value is arg 0 → emit `ARG[0]`. Landed across
  builder_owned_cleanup, builder_error_emission:278, builder_collection_layout (×4),
  list_mutate, builder_strings, builder_search, builder_arena_transfer, builder_values,
  builder_fs_paths (commits `3b29873c5`, `749593d66`, `8c40700af`, `f5bfbeb97`,
  `66679ded1`), each `bug387-gate full` PASS. **Total: 1,082,777 → ~291,606 (−73%).**
- **The remaining ~292K is the DELICATE tier — the error-Result construction** (almost all
  `builder_error_emission.rs`: 80, 94/96, 463–479, 718–755, 740–755, 981–996, …). This is
  NOT the uniform arena pattern and must NOT be batched blindly:
  - The 4-register error-Result convention is `RESULT_TAG_REGISTER=RET[0]`,
    `RESULT_VALUE_REGISTER=RET[1]`, `RESULT_ERROR_MESSAGE_REGISTER=RET[2]`,
    `RESULT_ERROR_SOURCE_REGISTER=RET[3]` (`error_constants.rs:25-31`). These are **genuine
    results** at a function's return (the `Result {tag,value,message,source}` is returned in
    `%ret0..3`) — so the constants themselves must NOT be re-tokenized.
  - But at the *transient* build/park sites (e.g. `emit_park_error_block_from_registers`:
    spill `RESULT_*_REGISTER` to slots around the clobbering `arena_alloc`, then reload) and
    where the code/message/source flow into the error-block *builder call*, the fixpoint
    colors `%ret1/%ret2/%ret3` as **arg registers** (`rsi/rdx/rcx` — the census `%retK`→arg
    transitions). Re-tokenizing MUST be **per-site**: change a transient/arg-flowing emission
    to `ARG[K]` while leaving the genuine-return emission as `RET[K]`. The spill/reload
    through a stack slot separates the two logical values (so this is NOT a Category-2
    conflict — consistent with plan-71-B's proof), but the classification is per-emission and
    error-prone. `emit_checked_size_add*` (line 80) is a builder-level helper whose divergent
    `dst` is a *parameter* — fix at its callers (or give it `#[track_caller]` too), not in the
    helper. **Recommendation:** do this tier one small gated group at a time (the byte-identity
    gate over all 1162 fixtures catches any mis-classification), with fresh focus; it is the
    "silent wrong register — worst class" surface.

- **CRITICAL — byte-identity is necessary but NOT sufficient for a C re-tokenization; it
  must ALSO reduce the divergence count.** Verified the hard way: re-tokenizing the
  `emit_park_error_block_from_registers` spill/reload sites `RESULT_*_REGISTER → ARG[1..3]`
  passed `bug387-gate full` **byte-identical on all 5 targets** — yet the `MFB_BUG387_AUDIT`
  count **EXPLODED 291,606 → 1,562,598** (new ~1.3M divergences at the edited sites
  `builder_error_emission.rs:725-739`). Reverted (`b080194e2`). Root cause: unlike the
  arena args (where `%argK`'s home is a *stable* register — arg0 is always rdi regardless of
  context), the error-Result **spill-preserved** values have a **context-sensitive**
  fixpoint inference: emitting `%arg1` at the spill destabilizes the inference for the
  surrounding error-construction (the reload feeds a caller still using `%retK`), so `direct
  != inferred` at *more* sites. **Consequence for the workflow:** every C batch must be
  accepted on BOTH (a) `bug387-gate full` byte-identical AND (b) an `audit2-sweep` showing
  the total mismatch count strictly *decreased* — the byte-gate alone silently admits a
  divergence-increasing edit. **Consequence for the tier:** the error-Result sites are NOT
  the simple per-site `%retK→%argK` swap. They must be re-tokenized as the **whole
  error-Result convention at once** (all transient `RESULT_*_REGISTER` uses across every
  error-construction helper, consistently, so the fixpoint's inference converges), OR they
  are a genuine residual that plan-71-E must handle specially (keep `%retK` + a targeted
  fixpoint-equivalent). This is a design-level task requiring fresh, careful analysis — do
  NOT batch-swap them. The arena tier (−73%, all gated + divergence-verified) stands.

- **Merged current `main` (16 commits, plan-79 + plan-82) into the worktree.** These
  retyped `MirInstruction.fields`/helper params to `Operand`/`impl Into<Operand>` — the
  exact code the Phase-0 source tool instruments. Resolved 4 conflicts (mir.rs struct: keep
  main's `Operand` fields + my `source`; abi.rs: take main + re-apply the 95 `#[track_caller]`;
  mod.rs: main's `VirtualRegister` export + my `selfmove_probe`; v128.rs: main's `.into()` +
  my `source: None`). `cargo test` 3776 passed; `@src` tool still works. Merge commit on
  `worktree-P-71`. **NOTE:** the byte-identity baselines are now stale — must be re-recorded
  from merged `main` (fb0d36477) before the next gate.

- **The error-Result spill/restore convention is a CONFIRMED RESIDUAL (2 failed attempts).**
  `emit_park_error_block_from_registers` AND `store_pending_current_result` both spike
  divergences by ~84K–1.3M when their `RESULT_*_REGISTER` spill sites are re-tokenized to
  `ARG[K]` — byte-identical locally but the fixpoint's *context-sensitive* inference for a
  spilled result is not a stable target (unlike the arena args). This is **~192K of the
  remaining ~292K** (66%). It resists C's `%retK→%argK` swap and needs a **plan-71-E design
  decision**: either the fixpoint deletion keeps a targeted rule for the 4-register error
  convention, or these divergent bytes genuinely differ under the direct map and plan-71-E's
  "pure byte-identical deletion" premise does not hold for them. **This is the biggest open
  risk to plan-71's feasibility and must be resolved before plan-71-E.** The remaining
  **~100K non-error divergences ARE still the safe arena-call-arg pattern** (builder_collection_layout,
  builder_inplace_assign, builder_value_semantics, io_stdout, entry.rs, resource_cleanup, …)
  and can be cleared normally (audit-drop + gate).

## Summary

C is the high-volume, low-novelty heart of the fixpoint-removal prep: re-tokenize every
Family-1a producer (`%retK`→`%argK`) so the direct map and the fixpoint agree, driving
~99.7% of the divergence audit to zero. Every swap is byte-identical by the cross-check
(same `xN` on AArch64/RISC-V, same inferred register on x86), gated per file so a
mis-classified site is caught at its own commit. The correctness risk is volume and
mis-attribution, not mechanism; it rests on B's value-level partition and is contained
by the per-file byte-identity gate. C touches no fixpoint, no vocabulary, and no emitted
byte — it only relabels producers the census and B proved are call-argument values.
