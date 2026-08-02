# plan-71-B: Category-2 census + AArch64/RISC-V same-register move-elision

Last updated: 2026-08-02
Effort: large (3h–1d)
Depends on: plan-71-A (the cross-check gate + the divergence census it produced —
`planning/completed/plan-71-A-fixpoint-crosscheck-census.md`, `planning/plan-71-census.md`).

This sub-plan resolves the one uncertainty plan-71-A's operand-level census could not
see, and is scheduled first for exactly that reason: **is there any genuine
"Category 2" value** — a value physically produced as a call *result* in one register
and consumed as a call *argument* in a *different* register — anywhere in the codegen?
The divergence audit is blind to it by construction (an explicit `mov %argK,%retK`
staging move has both operands agree, so it emits no `BUG387-MISMATCH`), so its
existence is *residual uncertainty*, not a measured zero (plan-71-A §Open Decisions,
`plan-71-census.md` §"Category 2").

The single behavioral outcome of plan-71-B: a **verified value-level Category-1 /
Category-2 partition** of the divergence set that plan-71-C's bulk re-tokenization can
rely on, and — only if the probe finds any same-register reuse — a **redundant
same-register-move elision pass for AArch64 and RISC-V**, so that an explicit
`mov %argK,%retK` staging move (which plan-71-E will emit on the shared path) is
byte-identical on the ISAs that reuse one physical register (`mov xN,xN` is a no-op
that does not exist today, so it must be elided). Every emitted byte stays unchanged
on all five targets.

References:

- `planning/completed/plan-71-A-fixpoint-crosscheck-census.md` — the gate, the two
  fix categories (§3), and the explicit hand-off: *"Whether any `mov xN,xN` is emitted
  today is UNVERIFIED — plan-71-B's first task"* (§2 Verified properties) and the
  Category-2 Open Decision. Read both first.
- `planning/plan-71-census.md` — the measured operand-level divergence inventory;
  §"Category 2 — move-required" states precisely why the audit cannot see it and what
  a separate probe must ask.
- `bugs/bug-387-neutral-mir-stream-carries-aarch64-register-names.md` — the bug and
  the 2026-07-28 feasibility finding; the fixpoint is load-bearing for the shared
  lowering.
- `bugs/completed-bugs/bug-85-x86-entry-runtime-arg-staging-tokens.md` — the reverted
  prior attempt. Category 2 is its surface; B is where that risk is first probed.
- `src/target/shared/abi.rs:443` `move_register(dst, src)` — emits a `mov` with **no
  `dst==src` guard**; an explicit same-index staging move therefore materializes a
  real `mov xN,xN` on AArch64/RISC-V unless a pass elides it.
- `src/target/shared/abi.rs:327` `realize_abi_token` — `%arg0` and `%ret0` **both**
  realize to `x0` on AArch64; this collision is exactly what makes a same-index
  staging move a no-op there.
- `src/target/shared/code/crypto.rs:64` — a source comment *"removed a dead x0<-x0
  self-move here"*: direct evidence that self-moves are culled **by hand** today
  because no elision pass exists.
- `src/arch/aarch64/select.rs:20` `select_aarch64`, `src/arch/riscv64/select.rs:386`
  `select_riscv64` / `:708` `remap_riscv_abi` — the two selectors that consume role
  tokens directly and would emit the no-op move.
- `scripts/bug387-gate.sh` — the byte-identity gate (`app` fast / `full` corpus).
- `.ai/compiler.md` — the completion gate; a silent wrong register is the worst class,
  which is why byte-identity is the bar, not semantics.

## Prerequisites

The whole-feature preconditions are stated once in plan-71-A's Prerequisites table
(windows-x86_64 byte-identity goldens, artifact-gate green, `bug387-gate.sh` exists,
exe-oracle baselines re-recorded from clean `main`). They remain a precondition on
every later letter; do not re-negotiate them here.

| Must be true | Command | Status |
|---|---|---|
| plan-71-A complete (gate + census landed and archived) | `ls planning/completed/plan-71-A-*.md && ls planning/plan-71-census.md` | MET (archived `db65cb13a`; census `d008d2419`) |
| the env-gated cross-check is present in the tree | `grep -c 'MFB_BUG387_AUDIT' src/arch/x86_64/select.rs` (≥1) | MET (Phase 2, `e2355e0b3`) |
| exe-oracle baselines re-recorded from clean `main` this session (ephemeral `/tmp`) | `ls /tmp/bug387/oracle-linux-aarch64.txt /tmp/bug387/oracle-linux-riscv64.txt` | RE-RECORD FIRST (plan-71-A Phase 1's design; `/tmp` is ephemeral) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** The
> `/tmp/bug387/*` baselines are ephemeral and must be re-recorded from a clean `main`
> build before any byte-identity check in this letter, exactly as plan-71-A Phase 1
> requires. A stale or missing baseline silently invalidates every check downstream.
> If you stop, report the status of *all* rows.

Everything below is written against the world where these hold.

## 1. Goal

**plan-71-B goal:** the residual Category-2 uncertainty is resolved by direct
measurement, and the machinery any real Category-2 site needs exists and is
byte-identical, such that:

- A corpus-wide probe answers, with a command, **how many distinct emitted
  `mov xN,xN` self-moves** the current codegen produces per target after realization,
  and **whether any single value** in the divergence set is consumed at two sites that
  demand two different role tokens (which would reclassify it Category 2). Both counts
  are recorded in `planning/plan-71-census.md` (a new "Category-2 probe" subsection).
- If — and only if — that probe finds a genuine same-register result→arg reuse, an
  **elision pass** for AArch64 and RISC-V drops a `mov xN,xN` whose `dst==src` after
  realization, so an explicit `mov %argK,%retK` staging move is a no-op there. With the
  pass in place (or absent, if the probe finds none), `scripts/bug387-gate.sh … full`
  is byte-identical on all five targets.
- The census's Category-1 / Category-2 partition is **proven exhaustive at the value
  level** — recorded as a verified property, not an assumption — so plan-71-C may
  re-tokenize Family 1a producers knowing no producer it touches is secretly a
  Category-2 value.

**plan-71 overall goal (context, not delivered here):** delete the fixpoint; every
byte identical on `{linux,macos}-aarch64`, `linux-x86_64`, `windows-x86_64`,
`linux-riscv64` (plan-71-E).

### Non-goals (explicit constraints)

- **Any emitted byte, on any target.** The elision pass, if built, must be
  byte-identical: it may only remove an instruction that is provably a `dst==src`
  no-op after realization, and today no such instruction is emitted (verified below),
  so on the *current* tree the pass changes nothing. It becomes load-bearing only when
  plan-71-E emits explicit staging moves.
- **No re-tokenization of any producer.** That is plan-71-C/D. B measures and (if
  needed) builds the elision primitive; it does not touch the shared builders' token
  choices.
- **No fixpoint deletion, no `select_x86` realize-loop change.** That is plan-71-E.
- **The neutral token vocabulary itself.** B introduces no new token.
- **x86 self-moves.** x86 needs the `mov rdi,rax` (distinct registers); it is never a
  no-op there. The elision is AArch64/RISC-V-only and keyed on `dst==src` after
  realization, so it can never fire on x86's cross-register staging.

## 2. Current State

Role tokens are realized to physical registers per ISA: `realize_abi_token`
(`src/target/shared/abi.rs:327`) maps **both** `%arg0` and `%ret0` to `x0` on AArch64,
`%arg1`/`%ret1` to `x1`, etc.; RISC-V has the analogous `remap_register`
(`src/arch/riscv64/select.rs:726`). x86 alone splits the roles (`%arg0`→`rdi`,
`%ret0`→`rax`) and recovers them with the `remap_x86_abi` fixpoint. Consequently a
staging move whose source and destination name the *same* index but different roles —
`mov %arg0, %ret0` — realizes to `mov rdi, rax` on x86 (a real, needed move) but to
`mov x0, x0` on AArch64/RISC-V (a no-op).

`move_register(dst, src)` (`abi.rs:443`) emits the `mov` unconditionally — there is
**no `dst==src` guard** anywhere in the emit path. Today no such same-index staging
move is emitted on the shared path (the fixpoint stages result-reuse on x86 *below*
the token layer via `stage_result_reuse_x86`, invisible to the shared builders), which
is why the tree has no `mov xN,xN` and why the one place a dead self-move crept in was
removed by hand (`src/target/shared/code/crypto.rs:64`: *"removed a dead x0<-x0
self-move here"*). plan-71-E will move that staging up to the shared path as an
explicit `mov %argK,%retK`; that is what makes the no-op appear on AArch64/RISC-V and
what this pass must be ready to elide.

### Measured populations

| What | Count | Command |
|---|---|---|
| `move_register` has a `dst==src` guard | 0 (none) | `sed -n '443,447p' src/target/shared/abi.rs` — emits `mov` unconditionally |
| `%argK` and `%retK` collide to one `xN` on AArch64 | yes | `realize_abi_token` (`abi.rs:327`): `"%arg0" \| "%ret0" … => "x0"` |
| known hand-removed self-moves in shared code | ≥1 | `grep -rn 'self-move\|x0<-x0' src/target/shared/code/` → `crypto.rs:64` |
| distinct emitted `mov xN,xN` (dst==src post-realization), per target | **UNMEASURED** | B Phase 1 (the probe below) — this is the number that decides whether Phase 3 runs |
| divergence-set values consumed at two conflicting role tokens | **UNMEASURED** | B Phase 2 (value-level partition proof) |

### Verified properties

- **AArch64/RISC-V reuse one physical register for the arg-and-result roles
  (VERIFIED).** `realize_abi_token` (`abi.rs:327`) maps `%argK` and `%retK` to the same
  `xN`; this is the collision that makes a same-index staging move a no-op — read the
  function, both spellings map to one register.
- **No redundant-move / self-move elision pass exists (VERIFIED).**
  `grep -rniE 'redundant|elide|identity move|self.move' src/arch/aarch64 src/arch/riscv64
  src/target/shared/code` finds only overflow-check elision and the hand-removal comment
  at `crypto.rs:64` — confirming self-moves are culled manually, not by a pass. (This is
  plan-71-A §2's "no elision pass exists" claim, re-verified for this letter.)
- **Whether any `mov xN,xN` is emitted today is UNVERIFIED — this letter's first task.**
  plan-71-A §2 assigns it here; the probe (Phase 1) settles it.
- **The census's Category-1/Category-2 partition is operand-level, not value-level
  (VERIFIED by reading `plan-71-census.md` §"Category 2" + §Residue).** No divergent
  operand sits on a boundary op and every inferred register has a role-token preimage,
  so no *operand* is Category 2; but a *value* consumed at two conflicting sites is
  invisible to an operand-level audit. Proving no such value exists is Phase 2.

## 3. Design Overview

Two independent pieces, ordered uncertainty-first:

- **The Category-2 probe (schedule FIRST — this is the unproven premise).** Two
  measurements the operand audit could not make: (1) enumerate every emitted
  `mov` whose `dst` and `src` realize to the same physical register per target (the
  self-move population that an explicit staging move would join); (2) at the *value*
  level, check whether any value flowing through the divergence set is read at one site
  as an argument and produced at another as a result in a conflicting register — the
  true Category-2 signature. If both are empty, Category 2 is genuinely absent, the
  elision pass is unnecessary, and B collapses to the partition proof; if either is
  non-empty, Phase 3 builds the pass. **Do not assume either way** (plan-71-A Open
  Decision).

- **The elision pass (schedule after the probe; the correctness risk).** A byte-safe,
  minimal rewrite over the finalized instruction stream for AArch64 and RISC-V: after
  role tokens are realized, drop any `mov` whose `dst` string equals its `src` string.
  It is a pure deletion of provable no-ops, so on the current tree (which emits none) it
  is byte-identical by inspection, and it makes plan-71-E's explicit staging moves
  byte-identical on the reuse ISAs. It is deliberately **not** keyed on ABI tokens (it
  runs post-realization on `xN` strings) so it can never touch x86's cross-register
  `mov rdi,rax`.

**Where design uncertainty concentrates (Phase 1):** the existence and count of
genuine same-register reuse. **Where correctness risk concentrates (Phase 3):** the
elision pass touches the finalized stream on the codegen path every AArch64/RISC-V
program uses — a wrongly-elided move is a silent miscompile, the worst class.

Rejected alternatives:

- *Add a `dst==src` guard inside `move_register` itself.* Rejected: `move_register`
  runs at the token layer, before realization, where `%arg0 != %ret0` as strings — the
  guard would never fire for the staging case (different tokens) yet could mask a
  genuine token-level self-move elsewhere. The elision must run **after** realization,
  where the collision is visible, and as a separate pass so its blast radius is a single
  reviewable rewrite.
- *Assume Category 2 is empty because the audit reported 0.* Rejected: the audit is
  blind to it by construction (`plan-71-census.md` §"Category 2"); "0 in the audit" is
  not "0 in the codegen". The probe measures it directly.
- *Build the elision pass unconditionally.* Rejected on cost/clarity only if the probe
  proves zero reuse *and* zero risk of plan-71-E needing it — but plan-71-E **will**
  emit explicit staging moves for the genuine-reuse case, so if any reuse exists the
  pass is mandatory. The probe decides; the pass is built iff reuse exists.

## 4. Detailed Design

### The probe (Phase 1–2)

1. **Self-move enumeration (Phase 1).** Extend the existing audit harness: under a new
   `MFB_BUG387_SELFMOVE=1` (or reuse the census sweep with a post-realization filter),
   after `select_aarch64` / `select_riscv64` finalize, scan the instruction stream for
   any `CodeInstruction` with op `mov` and `dst == src`, emitting a
   `BUG387-SELFMOVE tgt=… op=mov reg=xN | site: …` line. Sweep the full 1139-fixture
   corpus for `linux-aarch64`, `macos-aarch64`, and `linux-riscv64` (the reuse ISAs),
   exactly as `census-sweep.sh` walks `tests/**/project.json`. Record the distinct
   normalized count per target in `plan-71-census.md`.

2. **Value-level partition proof (Phase 2).** For the divergence set already captured
   (`plan-71-census.md`), trace each divergent value (`%vN`) to *all* its use/def sites
   in the pre-realization MIR and confirm each value's home is single and consistent —
   i.e., no value is named `%argK` at one site and `%retK` at another demanding a
   different register. Because this is a per-value property, do it by grouping the audit
   lines by their `%vN` (before the `norm()` collapse) and asserting each value's token
   set maps to one register per ISA. Record the result as a verified property.

### The elision pass (Phase 3, iff the probe is non-empty)

A single function `elide_redundant_self_moves(instructions: &mut Vec<CodeInstruction>)`
in the AArch64 and RISC-V selection paths (or one shared helper in
`src/target/shared/code/` called from both `select_aarch64` and `select_riscv64` after
their remaps), that removes every `mov` whose realized `dst == src`. It runs *after*
`realize_abi_token` / `remap_register`, so it sees `x0`/`x0`, not `%arg0`/`%ret0`. It
must:

- match only op `mov` (never `bl`/`svc`/`ret`/`str`/`ldr` — a store with equal operand
  strings is not a no-op);
- compare the *realized* register strings, so it never fires on x86 (x86 never routes
  through this pass) and never on a cross-index staging move;
- preserve instruction order and every other field for surviving instructions.

The pass is guarded by unit tests that (a) a `mov x0,x0` is dropped, (b) a
`mov x0,x1` survives, (c) a `str x0,[x0]` survives, (d) the whole current corpus is
byte-identical with the pass installed (it removes nothing today).

## Compatibility / Format Impact

None. B adds a measurement mode and (conditionally) an elision pass that removes only
provable no-ops. No externally observable contract changes; with the pass installed on
the current tree, no emitted byte changes (it elides nothing until plan-71-E emits
staging moves).

## Phases

> Keep the checkboxes current in the same commit as the work. An unticked box means
> NOT DONE.

### Phase 1 — Category-2 probe: self-move enumeration

Resolves the unproven premise before any pass is built.

- [ ] Add the post-realization self-move probe (`BUG387-SELFMOVE`) to the AArch64 and
      RISC-V selection paths (`src/arch/aarch64/select.rs`, `src/arch/riscv64/select.rs`),
      env-gated and byte-identical when off — same discipline as plan-71-A's audit
      (`remap_x86_abi_inner` returns lines; wrapper `eprintln!`s).
- [ ] Sweep the full 1139-fixture corpus for `linux-aarch64`, `macos-aarch64`,
      `linux-riscv64`; collect all `BUG387-SELFMOVE` lines; record the distinct
      normalized count per target in a new `plan-71-census.md` "Category-2 probe"
      subsection, each count carrying its command.
- [ ] Tests: a unit test asserting the probe emits exactly one line for a constructed
      `mov x0,x0` and none for `mov x0,x1`.

Acceptance: `plan-71-census.md` gains a measured self-move count per reuse target (no
`~`); `bug387-gate.sh … full` PASS with the probe env unset (byte-identical, five
targets). If the count is >0, Phase 3 is required; if 0, Phase 3 is skipped and that is
recorded with its command.
Commit: —

### Phase 2 — value-level Category-1/Category-2 partition proof

- [ ] Group the divergence audit lines by pre-`norm` `%vN` and confirm each value's
      role-token set realizes to a single register per ISA (no value demands both
      `%argK` and a conflicting `%retK`).
- [ ] Record the result as a verified property in `plan-71-census.md` (the partition C
      relies on), with the command that produced it.

Acceptance: `plan-71-census.md` states, with a command, whether any value is
Category 2; if none, the Category-1 partition is marked proven-at-the-value-level —
the precondition plan-71-C depends on.
Commit: —

### Phase 3 — AArch64/RISC-V self-move elision pass (iff Phase 1 count > 0)

Largest blast radius; behind tests; runs only if the probe found reuse.

- [ ] Add `elide_redundant_self_moves` (shared helper called from `select_aarch64` and
      `select_riscv64` after their remaps) removing every `mov` with realized
      `dst == src`; op-`mov`-only, order-preserving.
- [ ] Tests: `mov x0,x0` dropped; `mov x0,x1` survives; `str x0,[x0]` survives; a
      corpus byte-identity assertion (pass installed removes nothing on the current
      tree).
- [ ] Gate: `bug387-gate.sh … full` byte-identical on all five targets with the pass
      installed.

Acceptance: elision unit tests green; `bug387-gate.sh … full` PASS (byte-identical);
full `cargo test --bin mfb` → real `test result: ok`.
Commit: —

## Validation Plan

- Tests: probe-emission and elision unit tests in `src/arch/aarch64/select::tests` and
  `src/arch/riscv64/select::tests`.
- Coverage check: the probe passes every corpus instruction through the self-move
  filter; the elision tests exercise drop/keep/store-keep paths.
- Runtime proof: none needed for a byte-identical measurement + no-op-removal pass; the
  byte-identity gate (`bug387-gate.sh … full` PASS, audit/probe unset) IS the proof.
  Re-probe remote GTK boxes (2228/2227) only at plan-71-E.
- Doc sync: none (no vocabulary change); update `plan-71-census.md` with the probe
  results.
- Acceptance: `cargo test --bin mfb` (real `test result: ok`), `scripts/bug387-gate.sh
  <exe> full` PASS. `scripts/artifact-gate.sh` only if no concurrent run holds it
  (project forbids concurrent artifact-gate).

## Open Decisions

- **Probe channel** — a dedicated `MFB_BUG387_SELFMOVE` env vs. extending the existing
  `MFB_BUG387_AUDIT` sweep with a post-realization filter. Recommend: a dedicated env,
  so the two measurements (operand divergence on x86, self-move on AArch64/RISC-V) stay
  independently reproducible. (§4)
- **Elision pass placement** — one shared helper in `src/target/shared/code/` called
  from both selectors vs. a per-arch copy. Recommend: one shared helper (identical
  logic, `dst==src` on realized strings), so there is a single reviewable no-op-remover.
  (§4)
- **If Phase 1 finds zero reuse** — skip Phase 3 entirely and let plan-71-E prove no
  staging move is ever needed, vs. build the pass anyway as insurance. Recommend: skip;
  plan-71-E will surface any late-appearing staging need and can pull the pass forward
  then. Record the zero with its command either way.

## Corrections

<Filled in during execution.>

## Summary

The engineering risk plan-71 carries into B is the *unmeasured* one: whether any
genuine same-register result→arg reuse exists, which the operand-level census cannot
see. B measures it directly (the self-move probe + value-level partition proof) and,
only if it exists, builds the byte-safe AArch64/RISC-V no-op-move elision that
plan-71-E's explicit staging depends on. Nothing about the token vocabulary, the
builders, or any emitted byte on the current tree changes in B — it produces the
verified partition C relies on and, conditionally, the elision primitive E relies on.
