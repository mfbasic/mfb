# plan-71 census — x86 ABI-token divergence inventory (produced by plan-71-A Phase 3)

Last updated: 2026-08-02

This is the measured product of plan-71-A: the exact, bucketed set of operands where
the context-free `map_token_direct` (the map the fixpoint deletion will install)
disagrees with `remap_x86_abi`'s CFG role-inference. It sets the scope and split of
plan-71-B onward. Every count below carries the command that produced it.

## How it was measured

The env-gated cross-check landed in plan-71-A Phase 2 (`src/arch/x86_64/select.rs`,
commit e2355e0b3). With `MFB_BUG387_AUDIT=1`, `remap_x86_abi` emits a
`BUG387-MISMATCH abi=… token=… direct=… inferred=… | site: <op> [<fields>]` line for
every operand whose deferred ABI role token maps (context-free) to a different x86
register than the fixpoint chose. The sweep built **every** fixture for `linux-x86_64`
and `windows-x86_64` with the audit on:

```
bash /tmp/bug387/census-sweep.sh target/release/mfb linux-x86_64   > mismatch-linux-x86_64.txt
bash /tmp/bug387/census-sweep.sh target/release/mfb windows-x86_64 > mismatch-windows-x86_64.txt
```

(`census-sweep.sh` walks `tests/**/project.json` — 1139 fixtures — exactly as
`scripts/exe-oracle.sh` does, cleaning `build/` and root `*.mfp` after each.)

The raw line count is inflated by the shared runtime helpers (arena/string/collection/
record builders) that are linked into **every** executable, so the same helper site
recurs across the corpus. The meaningful population is the **distinct normalized
shape** (drop the `@fixture=` tag; collapse `%vN`, immediates, offsets, symbols):

```
norm() { sed -E 's/ @fixture=.*//; s/%v[0-9]+/%vN/g; s/value=[0-9-]+/value=I/g;
                  s/offset=[0-9-]+/offset=O/g; s/symbol=[^],]+/symbol=S/g'; }
norm < mismatch-<t>.txt | sort -u | wc -l
```

## Measured populations

| Metric | linux-x86_64 | windows-x86_64 | Command |
|---|---|---|---|
| raw mismatch operands | 1,034,322 | 484,408 | `wc -l < mismatch-<t>.txt` |
| **distinct shapes** | **143** | **106** | `norm \| sort -u \| wc -l` |
| distinct DEF-field shapes (`dst`/`carry_out`/`borrow_out`) | 108 | 68 | `grep -cP 'token=(%\w+) .*\[(?:[^]]*, )?(?:dst\|carry_out\|borrow_out)=\1[,\]]'` |
| distinct USE-field shapes | 35 | 38 | (total − DEF) |
| fixtures with ≥1 mismatch | 641 / 1139 | 615 / 1139 | `grep -oE '@fixture=\S+' \| sort -u \| wc -l` |
| boundary ops (`bl`/`svc`/`ret`) that diverge | **0** | **0** | `grep -hoE 'site: (bl\|svc\|ret) '` → empty |

### Token → inferred-register transitions

linux-x86_64 (`sed -E 's/ \| site:.*//; s/@fixture=.*//' … \| sort \| uniq -c`):

| token | direct (map_token_direct) | inferred (fixpoint) | raw count | role reading |
|---|---|---|---|---|
| `%ret0` | rax | **rdi** | 910,622 | result-named value used as call arg 0 |
| `%ret1` | rdx | **rsi** | 44,348 | result-named value used as call arg 1 |
| `%ret2` | rcx | **rdx** | 38,398 | result-named value used as call arg 2 |
| `%ret3` | rsi | **rcx** | 38,210 | result-named value used as call arg 3 |
| `%arg1` | rsi | **rdx** | 1,372 | arg-named value colored a result (`RETS[1]`) |
| `%arg2` | rdx | **rcx** | 1,324 | arg-named value colored a result (`RETS[2]`) |
| `%arg0` | rdi | **rax** | 48 | arg-named value colored a result (`RETS[0]`) |

windows-x86_64:

| token | direct | inferred | raw count | role reading |
|---|---|---|---|---|
| `%ret0` | rax | **rcx** | 461,467 | result-named value used as Win64 arg 0 |
| `%arg0` | rcx | **rax** | 21,709 | arg-named value colored a result (`RETS_WIN64[0]`) |
| `%sysarg1` | rsi | **rdx** | 1,232 | syscall-arg token used as Win64 arg 1 (Windows has no syscalls) |

The distinct shapes span only value-producing/consuming ops — linux: `add_imm` 74,
`mov` 21, `ldr_u64` 19, `str_u64` 6, `cmp` 4, `add` 4, `mov_imm` 3, `cmp_imm` 3,
`ldr_u32` 2, `adrp` 2, `sxtw`/`sub`/`mvn`/`mul`/`eor` 1 each; windows similar. **No
`bl`/`svc`/`ret` boundary op ever carries a divergent operand.**

## Buckets

### Category 1 — colorable (re-tokenize the producer; no move, no elision) — **100% of the divergence audit**

Every measured divergence is a role token that names the *wrong* role: the operand's
value has a single consistent home, and the fixpoint's CFG inference already colors it
correctly. Because `map_token_direct` is onto the register file — every inferred
register has at least one role-token preimage (`rdi`=`%arg0`, `rax`=`%ret0`,
`rdx`=`%ret1`/`%arg2`, `rcx`=`%ret2`/`%arg3`, `rsi`=`%ret3`/`%arg1`, `r10`=`%sysarg3`,
the Win64 tables analogously) — the fix at each site is to **emit the role token whose
`map_token_direct` equals the inferred register**, which is byte-identical on x86 (same
register) *and* on AArch64/RISC-V (the correctly-named token realizes to the same
`xN`/`aN` the value already used). No instruction is added.

Two structurally distinct sub-families:

- **1a — result-named used as an argument (`%retK` → `CALL_ARGS[K]`).** The bulk:
  ~99.7% of raw operands (linux `%ret0..3`→`rdi/rsi/rdx/rcx` = 1,031,578; windows
  `%ret0`→`rcx` = 461,467). The builder emitted `%retK` for a value produced by a plain
  op and then consumed as call argument K. Fix: re-tokenize the producer `%retK`→`%argK`.
  Representative distinct sites (`grep 'token=%ret' distinct-linux.txt`):
  - `add [dst=%ret0, lhs=%ret0, rhs=%vN]` — accumulate into a value used as arg 0
  - `add_imm [dst=%ret0, src=%ret0, imm=40]` — pointer bump used as arg 0
  - `mov_imm [dst=%ret0, type=Integer, value=I]` — constant loaded as arg 0
  - `ldr_u64 [dst=%ret1, base=sp, offset=O]` — spilled value reloaded as arg 1
  - `mov [dst=%ret0, src=%ret1]` (182 raw) — stage `%ret1` (rdx) into the arg-0 slot

- **1b — arg/sysarg-named colored a result (`%argK`→`RETS[K]`, `%sysargK`→Win64 arg).**
  The tail: linux ~2,744 raw (`%arg0/1/2`); windows `%arg0`→rax 21,709 and
  `%sysarg1`→rdx 1,232. Two shapes:
  - a value the builder named `%argK` whose only downstream fact is a **stack spill**
    with no call boundary, so the inference's `None`-fallback colors it `RETS[K]` (e.g.
    `add_imm [dst=%arg1, src=sp, imm=…]` → rdx, `mov [dst=%arg0, src=x27]` → rax). Fix:
    re-tokenize `%argK`→`%retK` so the byte output (which the fixpoint fixes at `RETS[K]`
    today) is preserved. **Note:** the correct token here is dictated by the *inferred*
    register (a fixpoint fallback), NOT by the semantic role — a caution for C: follow
    the inference, since byte-identity is the bar, not semantics.
  - windows-only: `%sysarg1` (a syscall-arg token) used as a Win64 call argument, since
    Windows has no raw syscalls (OS calls go through the IAT). Fix: re-tokenize the
    windows emission `%sysargK`→`%argK`.

### Category 2 — move-required (explicit staging + AArch64/RISC-V elision) — **not visible in this audit; 0 sites measured here**

The genuine cross-call reuse §3 warns about — a value physically produced as a call
**result** (in `rax`) and consumed as an **argument** in a different register (`rdi`),
which AArch64 satisfies with **no move** (reuse `x0`) but x86 needs `mov rdi,rax` —
**does not surface as a divergence**, by construction:

- An explicit staging move `mov %argK, %retK` has *both* operands agree
  (`%argK`→`CALL_ARGS[K]`=inferred arg; `%retK`→`RETS[K]`=inferred post-call result),
  so it emits **no** `BUG387-MISMATCH`. Confirmed: the divergence set contains **no
  same-index `mov %argK,%retK`** — only *cross-index* stagings (`mov [dst=%ret0,
  src=%ret1]`, `mov [dst=%arg3, src=%ret0]`), which are themselves Category-1
  re-tokenizations (re-tokenize the divergent `dst`, byte-identical on both ISAs).
- The same-index physical reuse (the AArch64 `mov x0,x0` no-op that would need elision)
  lives **below** the token layer, staged by the fixpoint itself
  (`stage_result_reuse_x86`, cited in §3), so it is invisible to a token-vs-inference
  cross-check.

**Consequence for the split:** measuring Category 2 requires a *separate* probe — does
the codegen ever emit a same-register result→arg reuse (a `mov xN,xN` after elision, or
a value whose token would need to be *both* `%retK` and `%argK`)? — which plan-71-A §2
already assigns to **plan-71-B's first task** ("Whether any `mov xN,xN` is emitted today
is UNVERIFIED — plan-71-B's first task"). The divergence census cannot answer it; it is
the residual uncertainty, scheduled first.

### Residue (fits neither) — **0**

No divergent operand sits on a boundary op, and every inferred register has a role-token
preimage, so there is no third mechanism and no new token to introduce. The only
open-ended risk is the *safety* of the Category-1 re-tokenization — that no single
**value** is consumed at two sites demanding two different tokens (which would reclassify
that value as Category 2). That is a per-value property the operand-level audit cannot
see; it is resolved by plan-71-B before the bulk re-tokenization of C.

## Category-2 probe (plan-71-B Phase 1) — measured

The operand audit is blind to same-register `mov xN,xN` (an explicit staging move's
operands agree, so it emits no `BUG387-MISMATCH`), so plan-71-B added a dedicated
post-realization probe (`MFB_BUG387_SELFMOVE`, `src/target/shared/code/selfmove_probe.rs`,
invoked at the tail of `select_aarch64` and after `remap_riscv_abi` in `select_riscv64`)
and swept the corpus for the three reuse ISAs (`/tmp/bug387/selfmove-sweep.sh <exe> <tgt>`,
the same 1162-fixture walk as `census-sweep.sh`).

| target | raw self-moves | distinct shape | command |
|---|---|---|---|
| linux-aarch64 | **486** | `mov x1,x1` (1) | `selfmove-sweep.sh … linux-aarch64 \| wc -l` |
| macos-aarch64 | **234** | `mov x1,x1` (1) | `selfmove-sweep.sh … macos-aarch64 \| wc -l` |
| linux-riscv64 | **486** | `mov a1,a1` (1) | `selfmove-sweep.sh … linux-riscv64 \| wc -l` |

**The count is NOT zero — and this corrects a false premise in plan-71-B §2.** All of
these are one shape: a redundant same-token `mov %ret1,%ret1`, emitted by the
Result-return staging idiom `abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1])`
where `RESULT_VALUE_REGISTER == abi::RET[1]` (`error_constants.rs:26`). **19 shared-builder
sites** emit it (`grep -rnE 'move_register\(\s*RESULT_VALUE_REGISTER\s*,\s*abi::RET\[1\]'
src/target/shared/`: term, link_thunk, crypto, tls/{openssl,schannel_server,macos/*},
net/io, fs/{io,paths,atomic}). Verified properties of this population:

- **They reach final emitted code** (not a pre-regalloc artifact): the `-ncode` dump of
  `tests/byte-identity/fs` for `linux-aarch64` carries 7 × `{ "op":"mov","dst":"x1","src":"x1" }`.
- **They are SYMMETRIC across ISAs — not the asymmetric Category-2 staging.** The same
  fs sites emit `mov rdx,rdx` on `linux-x86_64` (7 ×, `RESULT_VALUE_REGISTER=%ret1→rdx`).
  A genuine Category-2 reuse would be a *real* cross-register move on x86 (`mov rdi,rax`)
  that is a no-op only on the reuse ISAs; this is a plain no-op on **every** target.
- **They are NON-DIVERGENT** — no `mov [dst=%?1, src=%?1]` shape appears in the divergence
  audit (`grep 'site: mov \[dst=%.*1, src=%.*1\]' distinct-*.txt` → empty). So plan-71-C/D
  (which re-tokenize only divergent Family-1a/1b sites) never touch them, and plan-71-E
  (x86-local; non-divergent ⇒ `map_token_direct == inferred` here) leaves them
  byte-identical. **They are byte-invariant across all of plan-71.**

**Consequence for the elision decision (corrects plan-71-B Phase 3's trigger).** Because
these 486/234/486 self-moves are already in the byte-identity baseline, a blanket
"remove every `mov xN,xN`" elision pass (plan-71-B Phase 3 as written) would *delete* them
and **break byte-identity** — the opposite of its stated "elides nothing today". The raw
self-move count therefore does NOT decide whether an elision pass is needed. The real
trigger is whether plan-71-E introduces a *new asymmetric* staging move (a genuine
Category-2 value — a value the x86 fixpoint stages via a real `mov rdi,rax` that has no
explicit shared-stream counterpart, so deletion would strand it). That is the value-level
question Phase 2 answers; if Phase 2 finds no Category-2 value, plan-71-E adds no new
staging and **no elision pass is built** (Phase 3 skipped — see plan-71-B Corrections).

## Value-level partition + param-bridge (plan-71-B Phase 2) — proven

Re-measured on **current `main`** (the plan-71-A census predates plan-78, which moved
bytes) with `MFB_BUG387_AUDIT=1` via `/tmp/bug387/audit2-sweep.sh` (captures both
`BUG387-MISMATCH` and the new `BUG387-PARAMBRIDGE` probe):

| target | raw mismatches | distinct shapes | param-bridge insertions | boundary-op divergences |
|---|---|---|---|---|
| linux-x86_64 | 1,096,094 | 79 | **0** | **0** |
| windows-x86_64 | 513,699 | (win64 3 families) | **0** | **0** |

Fresh transitions are exactly the plan-71-A families (linux `%ret0..3`→arg regs +
`%arg0/1/2`→result regs; win64 `%ret0`→rcx, `%arg0`→rax, `%sysarg1`→rdx). Every divergent
operand sits on a **value-producing/consuming op** (`mov`/`add_imm`/`ldr_*`/`str_*`/`cmp*`/
`add`/`sub*`/`sxtw`/`mul`/`eor`/`adrp`), **never** a boundary op (`bl`/`call`/`svc`/`ret`) —
re-confirmed on fresh data for both targets.

**Category-1 partition proven at the value level — no Category-2 value exists.** The proof
is structural and measured, not assumed:

- **The x86 fixpoint inserts NO instructions.** Its only `instructions.insert(…)`
  (`select.rs`, the param-bridge prologue) fires **0** times across both x86 corpora
  (`BUG387-PARAMBRIDGE` = 0). `grep 'instructions\.(insert|splice)' select.rs` shows that
  is the *only* insertion site. So `remap_x86_abi` is a **pure per-operand register
  rename** — every operand gets exactly one register, no move is ever added.
- **Therefore every value already occupies a single consistent register** across its def
  and all uses. If any value needed two conflicting homes (a Category-2 result→arg reuse),
  the fixpoint — which inserts no bridging move — would have to color the def and a use
  with different registers and NOT bridge them, i.e. the *current* code would miscompile.
  It does not (the whole suite + byte-identity gate pass), so **no such value exists**.
- Consequently C may re-tokenize every Family-1a producer knowing none is secretly a
  value that must also stay `%retK` at a conflicting use. **This is the proven-at-the-value-level
  partition plan-71-C depends on.**

**Bonus finding for plan-71-E (de-risks the deletion).** The param-bridge prologue being
dead (0 insertions) means the fixpoint's *only* non-rename behavior is absent on the real
corpus, so deleting `remap_x86_abi` and replacing it with the context-free
`map_token_direct` loses **no** inserted instruction — the "audit-at-zero ⟹ byte-identical
deletion" premise (plan-71-E) is sound on this axis. (This corrects the §Residue "no third
mechanism" wording: there *is* a third mechanism — instruction insertion via the
param-bridge — but it is empty, so it collapses to renaming in practice. plan-71-E should
still keep a live assert / re-add a bridge only if a future param layout ever makes
`param_home != arg`.)

**Elision decision (plan-71-B Phase 3): SKIPPED.** No Category-2 value ⇒ plan-71-E emits no
new asymmetric staging move ⇒ no new `mov xN,xN` appears on the reuse ISAs ⇒ no elision
pass is needed (and a blanket one would wrongly delete the 486/234/486 baseline
Result-return self-moves). Recorded with commands above.

## C work-list (plan-71-C Phase 1) — the Family-1a source sites, deterministically derived

Built with the plan-71-C Phase-0 source-instrumentation tool (commit `bac02f1c6`; byte-identity gate PASS all 5 targets, `/tmp/bug387/gate-C-phase0.log`): the
`MFB_BUG387_AUDIT` line now carries `@src=file:line` (the shared-builder emission site,
threaded through `CodeInstruction`→MIR→`select_x86` via `#[track_caller]`). The work-list
is a deterministic derivation, not an ambiguous grep:

```
bash /tmp/bug387/audit2-sweep.sh target/release/mfb linux-x86_64 \
  | grep '^BUG387-MISMATCH' | grep -oE '@src=[^ ]+' | sort | uniq -c | sort -rn
```

linux-x86_64: **1,082,777** mismatches → **115 distinct `@src` sites** across **20 files**;
**0** mismatches lack a source. Per-file (site count):

| file | sites | note |
|---|---|---|
| `builder_error_emission.rs` | 28 | the error-Result construction (~half of all divergences; line 278 alone = 240,472) |
| `abi.rs` | 18 | **imprecise** — instructions added via `.push`/`.extend` (not `self.emit`) capture the abi-helper line, not the builder. Refine by making the `abi::` emit helpers `#[track_caller]` (deferred; a minority, ~3% of raw) |
| `builder_arena_transfer.rs` | 11 | |
| `builder_collection_layout.rs` | 10 | line 332 = 69,089 |
| `builder_owned_cleanup.rs` | 8 | the arena_free arg pattern (`load_u64(return_register(), …)` then `emit_arena_free_call`; 187/193/201 = 150,760 each) |
| `builder_collection_query.rs` | 8 | |
| `builder_values.rs` | 7 | |
| others (12 files) | 15 | inplace_assign 4, strings 3, search 3, map/list/value_semantics/resource_cleanup/fs_paths 2 each, +6 single-site files |

**Each site is a candidate `return_register()`/`RET[K]` → `ARG[K]` re-tokenization** where the
produced value is consumed as a call argument (verified per-site by reading the builder + the
byte-identity gate, never by the `@src` count alone — a genuine *result* producer at the same
line must stay `%retK`). The 18 `abi.rs` sites need the helper `#[track_caller]` refinement to
pin their builder before re-tokenization. C Phase 2 works this list per-file, each a
byte-identity-gated commit.

## B-onward split (implementation order = letter order; each depends only on its predecessor)

Derived from the counts above. The uncertainty (Category-2 existence + re-tokenization
safety) is scheduled FIRST; the high-volume-but-mechanical re-tokenization next; the
fixpoint deletion last.

- **plan-71-B — Category-2 census + AArch64/RISC-V same-register move-elision (size: large).**
  Uncertainty-first, the bug-85 surface. (1) Determine whether any value is a genuine
  same-register result→arg reuse — enumerate emitted `mov xN,xN` (post-realization) and
  any value whose token would need to be both `%retK` and `%argK`. (2) If any exist,
  build the redundant same-register-move elision pass for AArch64/RISC-V so an explicit
  `mov %argK,%retK` staging move is byte-identical on those ISAs (the master §3 no-op
  the plan calls out). (3) Prove the Category-1/Category-2 partition of the census is
  exhaustive at the value level. Gate: `bug387-gate.sh full` byte-identical; new elision
  unit tests. **Produces:** the verified Cat1/Cat2 value partition C relies on, and the
  elision pass (if needed).

- **plan-71-C — re-tokenize Family 1a (result-named → argument) (size: large).** The
  bulk (~99.7% of operands, but concentrated in the shared arena/string/collection/record
  builders — up to the 73 `abi::ARG[]`/`RET[]`-referencing files §2 measured). Emit
  `%argK` where the builder emits `%retK` for a value the census (and B's value
  partition) prove is a pure call-argument producer. Mechanical + per-file commits, each
  gated byte-identical. Split C into C/D-by-subsystem only if the file volume proves
  unwieldy in execution; the census shows one uniform transform, so keep it one letter.
  **Depends on:** B's value partition. **Produces:** Family 1a driven to zero mismatches.

- **plan-71-D — re-tokenize Family 1b tail + windows `%sysarg` (size: medium).** The
  arg-named-colored-result sites (`%argK`→`%retK`, following the inferred fallback
  register) and the windows-only `%sysargK`→`%argK`. Smaller and structurally distinct
  (fallback-driven and platform-specific), so isolated from C. **Depends on:** C.
  **Produces:** Families 1b/windows driven to zero; the full audit reports **0**
  mismatches on all x86 targets.

- **plan-71-E — delete the fixpoint; flip the cross-check live (size: large).** With the
  audit at zero, replace `select_x86`'s deferred-token realization + the 587-line
  `remap_x86_abi` fixpoint with the direct `map_token_direct` lookup, and flip the
  cross-check `assert_eq!` live as the deletion's safety net (Open Decision 2). Prove
  byte-identity on all five targets (`bug387-gate.sh full`, `artifact-gate.sh` 0 diffs)
  and re-probe the remote GTK boxes (2228/2227) + Windows box 2230 for runtime
  confirmation. **Depends on:** D (zero mismatches). **Produces:** the single behavioral
  outcome of plan-71 — the fixpoint deleted, every emitted byte unchanged.

## Open Decisions surfaced

- **Category-2 existence is unresolved by the audit** (it is invisible to the divergence
  cross-check by construction). plan-71-B's first task must probe it directly; if it
  finds none, the AArch64/RISC-V elision pass is unnecessary and B collapses to the
  value-partition proof only. Do not assume either way from this census.
- **Re-tokenization safety** (no value needs two tokens) is a per-value property this
  operand-level census cannot verify; B establishes it before C's bulk edit.
