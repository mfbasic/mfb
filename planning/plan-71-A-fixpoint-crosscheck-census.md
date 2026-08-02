# plan-71-A: x86 fixpoint deletion — cross-check gate + corpus census

Last updated: 2026-07-28
Overall Effort: huge (> 3d) — the whole plan-71 feature (delete `remap_x86_abi`'s
CFG role-inference fixpoint byte-identically). This sub-plan A is the gate + the
census that measures the rest; B onward are scoped by A's census (see the roadmap).
Effort: large (3h–1d)
Depends on: nothing (the byte-identity goldens + baselines it needs already landed
on `main` — see Prerequisites).

This sub-plan builds the **machine-checked equivalence gate** the whole feature runs
on, and uses it to **census** the exact set of operands where a context-free direct
token→x86 map disagrees with the fixpoint — the work B onward must eliminate. It
lands the gate as an env-gated diagnostic that is **byte-identical by construction**
(off by default; the fixpoint still produces every byte), and produces a categorized
site inventory that sets the scope and split of every later sub-plan.

The single behavioral outcome of plan-71 as a whole: `remap_x86_abi`'s three stacked
CFG dataflow analyses (`src/arch/x86_64/select.rs:162–684`, 587 lines) are **deleted**
and replaced by a context-free `map_token_direct`, with **every emitted byte
unchanged** on all five targets. Sub-plan A delivers none of that deletion — it
delivers the gate and the measured map of what stands in the way.

References:

- `bugs/bug-387-neutral-mir-stream-carries-aarch64-register-names.md` — the bug, the
  **2026-07-28 finding** (610 divergences in 3 app fixtures; the fixpoint is
  load-bearing for the shared lowering) and the **Feasibility verdict**. Read both
  first; this plan is the "own plan" that finding calls for.
- `bugs/completed-bugs/bug-85-x86-entry-runtime-arg-staging-tokens.md` — the prior
  attempt (plan-34-B Phase 4) that realized role tokens directly, broke every x86-64
  program, and was reverted. This plan must succeed where it failed, under the gate.
- `planning/old-plans/plan-34-B-role-named-registers.md` — the neutral role-token
  vocabulary (`%argN`/`%retN`/`%sysargN`/`%sysnr`/`%scratchN`/…) this builds on.
- `src/target/shared/abi.rs:327` `realize_abi_token` — token → AArch64 `xN` map.
- `src/arch/x86_64/select.rs` — `remap_x86_abi` (the fixpoint, `:162`),
  `map_abi_register` (`:123`), `map_scratch_register` (`:36`), `select_x86` (`:749`).
- `src/docs/spec/architecture/` — the register-role vocabulary a fix keeps in sync.
- `.ai/compiler.md` — the hard completion gate and the "silent wrong value is the
  worst class" warning that makes byte-identity the law here.

## Prerequisites

These are a precondition on the whole plan-71 feature, not a dependency to negotiate.
Stated once here in sub-plan A; every later letter points back to this table.

| Must be true | Command | Status |
|---|---|---|
| windows-x86_64 byte-identity goldens exist (the fixpoint deletion rewrites the Win64 arm) | `find tests/byte-identity -name '*windows-x86_64*' \| wc -l → 20` | MET (main `eea9aadf9`) |
| artifact-gate green (incl. windows) | `scripts/artifact-gate.sh target/release/mfb → "… 0 diff(s)"` | MET (1499 goldens, 0 diffs) |
| the byte-identity gate script exists | `ls scripts/bug387-gate.sh` | MET (`ls` → `scripts/bug387-gate.sh`) |
| pre-fix exe-oracle baselines recorded (5 targets) | `ls /tmp/bug387/oracle-linux-x86_64.txt /tmp/bug387/oracle-windows-x86_64.txt /tmp/bug387/oracle-linux-riscv64.txt /tmp/bug387/oracle-linux-aarch64.txt` | MET — **re-recorded this session** by Phase 1 from a clean `main` build (binary mtime 06:35 predates the select.rs edit at 06:40): linux-x86_64=1282, windows-x86_64=611, linux-riscv64=1280, linux-aarch64=1282, macos-aarch64=640 executables. Still **EPHEMERAL /tmp** (Open Decision 1 deferred). |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** The
> `/tmp/bug387/*` baselines are ephemeral. Do not trust the MET above — re-record the
> baselines from a clean `main` build as A's first task, because a stale or missing
> baseline silently invalidates every byte-identity check downstream.
>
> If you stop, report the status of *all* rows, not just the one that blocked you.

Everything below is written against the world where these hold.

## 1. Goal

**plan-71-A goal (this sub-plan):** a reproducible, machine-checked gate exists and a
categorized census is recorded, such that:

- `select_x86` defers the ABI *role* tokens (`%argN`/`%retN`/`%sysargN`/`%sysnr`/
  `%sysret`) into `remap_x86_abi` instead of realizing them to `xN` in `select_x86`;
  `remap_x86_abi` realizes them internally for the (unchanged) inference and, under
  `MFB_BUG387_AUDIT`, reports every operand where a new context-free
  `map_token_direct(token, abi)` disagrees with the inference's chosen register.
- With the audit env **unset**, every emitted byte is unchanged on all five targets
  (`scripts/bug387-gate.sh … full` → PASS; `scripts/artifact-gate.sh` → 0 diffs).
- An audit sweep over the whole exe-oracle corpus produces `plan-71-census.md`: the
  **exact** count of divergent operands, bucketed into the two fix categories
  (§3) with per-category counts and representative sites.

**plan-71 overall goal (context, not delivered here):** delete the fixpoint; every
byte identical on `{linux,macos}-aarch64`, `linux-x86_64`, `windows-x86_64`,
`linux-riscv64`.

### Non-goals (explicit constraints)

- **Any emitted byte, on any target.** This is the same bar bug-85 failed. A single
  changed instruction encoding anywhere is a failed change.
- Instruction *selection decisions* (which `CodeOp` for a given MIR op). Only how
  registers are *named/colored* changes.
- The `EncodedImage` field set, relocation `kind`/`binding`, the linker's view.
- The neutral token *vocabulary itself* (adding a token is a later-letter decision,
  recorded in the census, not done in A).
- A does **not** modify the builder, add the elision pass, or delete the fixpoint —
  those are B onward. A is gate + measurement only.

## 2. Current State

The shared lowering emits neutral tokens; `select_x86`
(`src/arch/x86_64/select.rs:749`) realizes **every** token to its AArch64 `xN`
spelling (via `realize_abi_token`, `abi.rs:327`) *before* `remap_x86_abi` runs, then
`remap_x86_abi` (`:162–684`) runs three stacked CFG dataflow analyses to re-derive,
for each `x0`–`x8`, whether it is a call arg / syscall arg / return / call result /
incoming parameter / staged error-Result, and colors it to the SysV or Win64 home.
AArch64 (`select_aarch64`, `src/arch/aarch64/select.rs:20`) consumes the tokens
directly (no remap); riscv64 has its own `remap_riscv_abi`/`remap_register`
(`src/arch/riscv64/select.rs:708`/`:726`).

The fixpoint exists because AArch64 reuses one physical register for two roles: a
value produced as a result (`x0`) and consumed as the next call's argument is the
*same* `x0`, emitted once. On x86 the result is `rax` and the argument is `rdi`; the
fixpoint recovers which role each operand plays and colors it accordingly. bug-85
tried to replace this with a direct token lookup and broke every x86 program.

### Measured populations

| What | Count | Command |
|---|---|---|
| `remap_x86_abi` fixpoint size | 587 lines (`:162–684`) | `awk 'NR>=162&&/^}/{print NR;exit}' src/arch/x86_64/select.rs` |
| shared-code files referencing `abi::ARG[]`/`RET[]` | 73 | `grep -rlE "abi::(ARG\|RET)\[" src/target/shared/code/ \| wc -l` |
| `abi::ARG[]` refs in shared code | 485 | `grep -rhoE "abi::ARG\[" src/target/shared/code/*.rs \| wc -l` |
| `abi::RET[]` refs in shared code | 306 | `grep -rhoE "abi::RET\[" src/target/shared/code/*.rs \| wc -l` |
| divergent operands, 3 app fixtures only | 610 | audit-mode `-app -ncode` of `tests/syntax/app/macos-app-mode-*` for linux-x86_64 (this session) |
| divergent operands, **full corpus** | **UNMEASURED** | A's census task (audit sweep over all exe-oracle fixtures × {linux-x86_64, windows-x86_64}) |

### Verified properties

- **The direct map ≠ the fixpoint, pervasively (VERIFIED this session).** A cross-check
  binary in assert mode panics on the first shared-lowering site of every x86/Windows
  build; in audit mode it listed 610 divergences in 3 app fixtures. So the fixpoint is
  load-bearing for the shared lowering universally — not a linux_gtk-only concern.
- **The dominant idiom is "value produced into `%retK`/xK, then flows into a call as
  arg K" (VERIFIED).** e.g. `AddImm dst=%ret0 src=%v429`, `MovImm dst=%ret0 value=1`
  → the inference colors `rdi`, the direct map says `rax`.
- **No AArch64/RISC-V redundant-`mov xN,xN` elision pass exists (VERIFIED).**
  `grep -rniE "redundant\|elide\|identity move" src/arch/aarch64 src/target/shared/code`
  finds only overflow-check elision. Whether any `mov xN,xN` is emitted today is
  **UNVERIFIED** — plan-71-B's first task.
- **The two fix categories are distinct and the split is UNMEASURED (this is exactly
  what A's census resolves).** See §3.

## 3. Design Overview

The fixpoint is deletable byte-identically, but only by handling two structurally
different classes of divergent operand — and the split between them is the number A
must measure, because it sets whether B..N is three sub-plans or eight.

- **Category 1 — colorable (re-tokenize; no move, no elision).** A single operand
  whose `xK` the fixpoint colors as arg-or-result purely by role. The builder emitted
  the "wrong" alias (`%ret0` for a value that is actually a call argument). Fix: emit
  the *correct* token (`%arg0`) at that site. `map_token_direct(%arg0)=rdi` on x86 and
  `→x0` on AArch64 — **byte-identical on both, no instruction added.** This is expected
  to be the bulk. Risk: low per-site, but high volume across 73 files.

- **Category 2 — move-required (explicit staging + elision).** A value physically in
  one register (a call *result* in `rax`) consumed as an argument in a *different*
  register (`rdi`) — the genuine cross-call reuse. Today AArch64 needs no move (reuse
  `x0`), and x86 gets its `mov rdi,rax` from an inserted `mov x0,x0` +
  fixpoint coloring (`stage_result_reuse_x86` does exactly this for linux_gtk). Fix:
  the producer emits an explicit `mov %argK, %retK`. On x86 → `mov rdi,rax` (matches
  today). On AArch64/RISC-V → `mov xN,xN`, a **no-op that does not exist today**, so
  it must be **elided** to stay byte-identical (plan-71-B). Risk: high — this is the
  bug-85 surface.

**Where design uncertainty concentrates (schedule FIRST — this sub-plan):** the
*ratio* of Category 1 to Category 2, and whether every divergent site cleanly falls
into one of the two (a site that fits neither is a third category that would need its
own mechanism or a new token — the census must surface it explicitly, not bucket it
away). This is unmeasured and it decides the whole split. That is why A exists and
why it runs before any builder edit.

**Where correctness risk concentrates (schedule LAST — later letters):** Category 2
and the fixpoint deletion, on the codegen path every program uses.

Rejected alternatives:

- *Re-land bug-85's direct lookup unqualified.* Rejected: it is the reverted
  regression; every step here is gated by the cross-check + byte-identity instead.
- *Delete the fixpoint and accept byte changes, re-baselining goldens.* Rejected:
  byte-identity is the law (Non-goals); a moved golden is a failed change, and the
  spec bar (`.ai/compiler.md`) treats a silent wrong register as the worst class.
- *Force explicit staging everywhere (Category-2 mechanism for all sites).* Rejected:
  it adds a `mov` on x86 for Category-1 sites that today have none → breaks x86
  byte-identity. Category 1 MUST be re-tokenization, not staging.

## 4. Detailed Design (sub-plan A)

The cross-check, re-derived from this session's reverted spike (~60 lines), env-gated
so production builds are byte-identical:

1. **`map_token_direct(value, abi) -> Option<String>`** in `select.rs`: `%argN` →
   `CALL_ARGS[N]`; `%sysargN` → `SYS_ARGS[N]`; `%retN` → `RETS[N]`;
   `%sysnr`/`%sysret` → `rax`; else `None`. Uses the existing SysV/Win64 tables.
2. **`is_abi_role_token(value)`** = `map_token_direct(value, SysV).is_some()`.
3. **`select_x86`**: in the realize loop, skip `is_abi_role_token` values (leave them
   as tokens); realize all other tokens (`%scratchN`/`%localN`/`%fscratchN`/…) as today.
4. **`remap_x86_abi`**: read `let audit = env::var_os("MFB_BUG387_AUDIT").is_some();`
   once. After the `x30`/`lr` retain, capture each still-present role token per
   (instruction, field) and realize it to `xN` (so the inference runs byte-identically);
   under `audit`, also snapshot each instruction's original `op`+`fields` for the
   report. In the rewrite loop, after the inference computes `mapped`, if the operand
   was a role token and `map_token_direct(tok) != mapped`: under `audit`, `eprintln!`
   a `BUG387-MISMATCH … | site: <op> <fields>` line and continue; otherwise (assert
   path) — for A, **do not panic** (assert mode is not usable until B..N drive
   divergences to zero); A ships audit-only, so the non-audit path just writes `mapped`
   exactly as today. (The `assert_eq!` becomes live in the final letter, as the
   proof the deletion is safe.)

This is byte-identical with the audit env unset: `mapped` is still what is written,
and deferring-then-re-realizing the role tokens yields the same `xN` the old
`select_x86` produced. Verified this session for macos/linux-aarch64/windows app-ncode
(identical) — linux-x86_64 moved only when the *builder* was also edited, which A does
not do.

The census sweep: for the full exe-oracle fixture set, build `-ncode` (and
`--app -ncode` for the app fixtures) for `linux-x86_64` and `windows-x86_64` with
`MFB_BUG387_AUDIT=1`, collect all `BUG387-MISMATCH` lines, and bucket by the `op`/
`fields` shape into Category 1 (single operand, no cross-call reuse) vs Category 2
(result consumed as a later call's arg) vs any residue. Record counts + representative
sites in `planning/plan-71-census.md`.

## Compatibility / Format Impact

None. Sub-plan A adds an env-gated diagnostic and a markdown census; no externally
observable contract changes and (audit off) no emitted byte changes.

## Phases

> Keep the checkboxes current in the same commit as the work. An unticked box means
> NOT DONE.

### Phase 1 — re-record clean baselines

Defends every later byte-identity check against the ephemeral `/tmp` baselines.

- [x] Build a clean `main` release (`cargo build --release --bin mfb` → `Finished release
      profile … in 1m 55s`; binary mtime 06:35:30 predates the Phase-2 select.rs edit at
      06:40:21, so it is the unmodified-`main` binary).
- [x] `scripts/exe-oracle.sh target/release/mfb <t> record /tmp/bug387/oracle-<t>.txt`
      for `t ∈ {linux-x86_64, windows-x86_64, linux-riscv64, linux-aarch64}` **and**
      the macos-aarch64 host set; and the app-ncode baseline (seeded `app-ncode-base.txt`
      from `scripts/bug387-gate.sh … app` on the clean binary). Recorded executable
      counts: linux-x86_64=1282, windows-x86_64=611, linux-riscv64=1280,
      linux-aarch64=1282, macos-aarch64=640 (each over 1139 `project.json` fixtures).
      Open Decision 1: **deferred** — see Open Decisions / Corrections (the sha256s are
      build-specific and would false-fail on any later codegen change, so they are not
      committed as a golden).
- [x] Confirm a no-op re-run is 0-diff (`scripts/bug387-gate.sh target/release/mfb full`
      → `BUG387-GATE: PASS (byte-identical)`; app-ncode byte-identical + all four
      exe-oracle targets `OK … byte-identical`) — proves the gate is sound before it
      guards anything.

Acceptance: `bug387-gate.sh … full` reports PASS on clean `main` (**met** — PASS,
byte-identical on app-ncode ×4 targets and exe-oracle ×4 targets); baseline manifests
for all five targets exist in `/tmp/bug387` with Open Decision 1 (commit them) explicitly
deferred (**met**).
Commit: c107bff93

### Phase 2 — cross-check gate (env-gated, byte-identical)

- [x] Add `map_token_direct` + `is_abi_role_token` to `src/arch/x86_64/select.rs` (§4).
- [x] Defer role tokens in `select_x86`; capture + realize + audit-report in
      `remap_x86_abi` (§4). Non-audit path writes `mapped` unchanged. (Implemented as
      `remap_x86_abi` → `remap_x86_abi_inner(instructions, abi, audit) -> Vec<String>`
      so the unit tests can force the cross-check on regardless of the environment; the
      env-reading wrapper `eprintln!`s the returned lines. See Corrections.)
- [x] Tests: extend `src/arch/x86_64/select::tests` — `map_token_direct_matches_the_abi_tables`
      (SysV+Win64 tables), `is_abi_role_token_covers_only_role_tokens`,
      `direct_map_agrees_with_the_fixpoint_on_clean_cases` (call args, returns, syscall
      args + nr → no mismatch), `audit_reports_result_reused_as_argument` (constructed
      reuse → exactly one `BUG387-MISMATCH token=%ret0 direct=rax inferred=rdi`), and
      `audit_off_reports_nothing_and_role_tokens_select_byte_identically`.
- [x] Gate: `bug387-gate.sh target/release/mfb full` → `BUG387-GATE: PASS
      (byte-identical)` (audit **unset**) — byte-identical on all five targets
      (linux-x86_64 1282, windows-x86_64 611, linux-riscv64 1280, linux-aarch64 1282
      executables `OK … byte-identical`; app-ncode byte-identical ×4).

Acceptance: `cargo test --bin mfb arch::x86_64::select` green (**27 passed**);
`bug387-gate.sh … full` PASS with audit unset (byte-identical everywhere, **met**);
a real full `cargo test --bin mfb` shows `test result: ok. 3751 passed; 0 failed`.
Commit: —

### Phase 3 — corpus census

- [x] Audit-sweep the full exe-oracle fixture set for `linux-x86_64` and
      `windows-x86_64` (`MFB_BUG387_AUDIT=1`), collecting all `BUG387-MISMATCH` lines
      (`/tmp/bug387/census-sweep.sh` over 1139 fixtures each; raw operands: linux
      1,034,322, windows 484,408).
- [x] Bucket by `op`/`fields` shape into Category 1 / Category 2 / residue; record
      exact counts, per-file distribution, and representative sites per bucket in
      `planning/plan-71-census.md`. Result: **143 distinct linux / 106 windows** divergent
      shapes, **100% Category 1** (re-tokenize the producer; no move) in two sub-families
      (1a result-named→arg = the ~99.7% bulk; 1b arg-named→result + windows `%sysarg`);
      **Category 2 = 0 in the audit** (invisible by construction — staging moves agree),
      **residue = 0** (no boundary op diverges; every inferred register has a role-token
      preimage).
- [x] From the census, write the **B-onward split**: `plan-71-B` = Category-2 census +
      AArch64/RISC-V same-register move-elision (uncertainty-first); `plan-71-C` =
      re-tokenize Family 1a (bulk); `plan-71-D` = re-tokenize Family 1b + windows
      `%sysarg`; `plan-71-E` = delete the fixpoint + flip the cross-check live. Letter
      order = implementation order; each depends only on its predecessor. The residue/
      Category-2 uncertainty is recorded as an Open Decision (measured separately in B).

Acceptance: `planning/plan-71-census.md` exists with a measured, bucketed divergence
inventory and a concrete B-onward split whose sizes derive from the census counts
(every count carries its command, no `~`) — **met**.
Commit: —

## Validation Plan

- Tests: `src/arch/x86_64/select::tests` gains the `map_token_direct`-vs-inference
  equivalence assertions and an audit-emission test.
- Coverage check: the new functions are exercised by the select unit tests and by the
  audit sweep (every corpus operand passes through the cross-check).
- Runtime proof: none needed for A — it is a byte-identical diagnostic; the byte-
  identity gate (`bug387-gate.sh … full` PASS with audit unset) IS the proof.
- Doc sync: none in A (the vocabulary is unchanged). Later letters update
  `src/docs/spec/architecture/` and reconcile plan-34-B / bug-85.
- Acceptance: `cargo test` (full, real `test result: ok`), `scripts/artifact-gate.sh`
  0 diffs, `scripts/bug387-gate.sh … full` PASS. (Remote GTK boxes 2228/2227 were down
  this session; for a zero-byte-change refactor byte-identity is the definitive proof,
  so remote runtime is confirmation, not a gate — but re-probe them before the final
  letter and run `scripts/test-appimage.sh --libc both` + the Windows box 2230 there
  if up.)

## Open Decisions

- **Baseline persistence** — commit the exe-oracle manifests to the repo (a plan-owned
  dir) vs keep them in `/tmp` and re-record per session. Recommend: commit them, so
  the gate is reproducible across sessions and machines. (§Phase 1)
  **RESOLVED — deferred (do NOT commit).** The manifest is a list of sha256 hashes of
  *produced executables*, valid only against the exact `mfb` build that recorded it.
  Any later `main` commit that moves a single byte on any target (a routine occurrence)
  invalidates the whole manifest, turning a committed golden into a false-failure
  generator that every unrelated codegen change would have to re-baseline. The baseline
  is a per-session artifact, not a durable golden; re-recording it from clean `main` is
  Phase 1's first task by design. Kept in `/tmp/bug387` for this session.
- **assert vs audit in production** — keep the cross-check audit-only until the final
  letter, then flip to a live `assert_eq!` as the deletion's safety net, vs delete the
  cross-check at the end. Recommend: flip to `assert_eq!` in the final letter and keep
  it (cheap, catches any future token regression), then it can be removed once the
  fixpoint is gone if it proves hot. (§4)
- **Residue category** — if the census finds sites that are neither Category 1 nor 2,
  decide new-token vs new-mechanism there, in the census, before scoping C onward.
  **RESOLVED — no residue.** The census (`planning/plan-71-census.md`) found every
  divergent operand is Category 1 (re-tokenizable; no boundary op diverges, every
  inferred register has a role-token preimage). No new token or mechanism is needed.
- **Category 2 is not measurable by the divergence audit (surfaced by the census).**
  A genuine same-register result→arg reuse emits no `BUG387-MISMATCH` (its staging move's
  operands agree), so the audit reports 0 Category-2 sites by construction — this is NOT
  evidence Category 2 is empty. Measuring it (and building the AArch64/RISC-V mov-elision
  pass if needed) is plan-71-B's first task, exactly as §2 anticipated. The
  Category-1/Category-2 partition is safe only once B proves at the value level that no
  single value needs two conflicting tokens.

## Corrections

- **Open Decision 1 resolved as DEFER (do not commit baselines).** Evidence: the
  exe-oracle manifest is sha256s of produced executables, valid only against the exact
  `mfb` build that recorded it; any later byte-moving codegen change on any target
  invalidates the whole file, so committing it manufactures false failures rather than a
  durable golden. Baselines re-recorded from clean `main` in `/tmp/bug387` each session
  (Phase 1's design). See Open Decisions.
- **`remap_x86_abi` split into a thin env-reading wrapper + `remap_x86_abi_inner(…,
  audit) -> Vec<String>`.** The plan's §4 step 4 described the audit as inline
  `eprintln!`s; testing emission that way needs stderr capture, which std does not offer
  in-process. The `inner` fn instead returns the `BUG387-MISMATCH` lines (empty unless
  `audit`) and the wrapper `eprintln!`s them, so `audit_reports_result_reused_as_argument`
  can assert on the returned vec while the census sweep still reads them on stderr. The
  register rewriting is unchanged and byte-identical whether or not `audit` is set — the
  bug387-gate (audit unset) PASS on all five targets is the proof.
- **No `assert` path added (as A intended).** §4 notes A ships audit-only; the non-audit
  path writes `mapped` exactly as today with no panic. The live `assert_eq!` is the final
  letter's job.

## Summary

The real engineering risk of plan-71 lives in Category 2 (explicit staging + the new
AArch64/RISC-V mov-elision pass) and the fixpoint deletion itself — the bug-85 surface,
on the path every program uses. Sub-plan A touches none of it: it builds the
machine-checked equivalence gate (byte-identical, env-gated) and measures the exact,
bucketed divergence inventory that decides how B..N are split. The corpus divergence
count is UNMEASURED today; measuring it is A's product and the precondition for scoping
any code change. Nothing about the neutral vocabulary, selection decisions, or emitted
bytes changes in A.
