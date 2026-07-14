# plan-34-A: Neutral tokens for the invariant registers — `%zero`, `%lr`, `%arena`

Last updated: 2026-07-09
Overall Effort: huge (>3d) — the whole `plan-34` feature (A + B + C)
Effort: medium (1h–2h)
Depends on: nothing

`src/target/shared/code/` names registers in two incompatible styles, sometimes in the same
function call. `abi::stack_pointer()` returns the **role name** `"sp"`; `abi::link_register()`
returns the **AArch64 register number** `"x30"`. `mir::ARENA_BASE` is the role name
`"arena_base"`; `CLOSURE_ENV_REGISTER` is `"x28"`. The zero register is spelled `"x31"` at
213 sites and `"xzr"` at ~6 more, and is neither the stack pointer nor a general register on
two of the three supported ISAs.

This sub-plan finishes the role-name style for the three registers whose role is a
**program-wide or frame-wide invariant** — zero, the link register, and the arena base — so
that none of them is ever spelled as an AArch64 register number in shared lowering. The
call-boundary registers (arguments, results, syscall arguments, the closure environment) are
plan-34-B; the hand-picked scratch is plan-34-C.

The single behavioral outcome: **shared lowering contains no `"x30"`, `"x31"`, or `"x19"`
literal; each is named by one neutral token; and the emitted bytes for all four targets are
unchanged.**

References:

- `src/docs/spec/memory/06_native-calling-convention.md` — the register ABI this plan
  re-words but does not change.
- `src/docs/spec/memory/07_runtime-helper-abi.md` — helper-body register rules.
- `planning/old-plans/plan-00-D-*` — introduced `mir::ARENA_BASE`, the precedent this plan
  generalises and completes.
- `.ai/compiler.md` — runtime completion gate, acceptance suite, register lifetimes.

## 1. Goal

- ✅ `grep -E '"x(19|30|31)"' src/target/shared/code/` returns nothing in production lowering
  (the only matches are inside `mir.rs`'s guard test, which *names* the three registers in a
  negative assertion in order to forbid them).
- ✅ Three constants — `abi::ZERO`, `abi::LR`, `abi::ARENA` — are the only way shared code names
  these registers.
- ✅ Byte-identity verified for macos-aarch64, linux-aarch64, linux-x86_64, and linux-riscv64
  against a pristine-HEAD worktree — via `artifact-gate.sh` (committed macos-aarch64
  `.nobj`/`.nir`) **and** a full four-target `.nobj` cross-emission diff (447 × 4 → 0 diffs).
  Note: the byte-identity artifact is `.nobj` (the encoded machine-code plan); the `.ncode` and
  `-mir` *text* dumps render register mnemonics and so legitimately change (`x31`→`xzr`,
  `x30`→`lr`) — their goldens were refreshed.

### Non-goals (explicit constraints)

- **No emitted-byte change, anywhere.** This is a rename. A single differing byte in any
  `.ncode` golden is a bug in this plan, not a golden to refresh.
- **No change to the call boundary** (`x0`–`x7`, results, syscall args, `x28`) — plan-34-B.
- **No vreg conversion** of any operand — plan-34-C.
- **No change to `abi::stack_pointer()`**, which already returns `"sp"` and is correct.
- **`arena_base` stays reserved from allocation** on all three ISAs. This plan changes where
  its name is written, never whether the allocator may color it.
- The AArch64 encoder must keep accepting literal `"x30"`/`"x31"` from any *other* producer
  (tests, `-ncode` round-trips); only shared lowering stops emitting them.

## 2. Current State

### 2.1 The two styles

`src/arch/aarch64/abi.rs`, within twelve lines of each other:

- `:88` — `pub(crate) fn link_register() -> &'static str { "x30" }` — a register number.
- `:92` — `pub(crate) fn stack_pointer() -> &'static str { "sp" }` — a role name.

`abi::stack_pointer()` has 2665 call sites in `src/target/shared/code/`, and every backend
handles `"sp"` as a role (`riscv64/select.rs:526` — `"sp" | "raw_sp" => "sp"`). It is the
working precedent. The rest of the file is not.

### 2.2 Zero

| spelling | count in `shared/code/` | example |
|---|---|---|
| `"x31"` | 213 | `builder_arena_transfer.rs:289` — `abi::store_u64("x31", "x1", FILE_OFFSET_BUF_PTR)` |
| `"xzr"` | ~6 | `builder_numeric.rs:1427` — `abi::subtract_registers(register, "xzr", register)` |

Both mean *the constant zero as a register operand*. `builder_arena_transfer.rs:289-297`
zeroes six `File` record fields by using it as a store **source**; `float_format.rs:105`
annotates the intent inline (`// k = 0 - e2 (xzr source)`). The stack pointer is `"sp"`, so
no `"x31"` site can mean `sp` — but Phase 1 proves that rather than assuming it.

Both non-AArch64 backends already special-case the pair:

- `riscv64/select.rs:527` — `"x31" | "xzr" => ZERO` (RISC-V has a hardware `zero`).
- `x86_64/encode/operand.rs:41-44` — `"xzr" | "rzero" | "zero" => 16`, a "no register"
  sentinel, because **x86-64 has no zero register**.
- `x86_64/regmodel.rs:36` — `ZERO_REGISTER: &str = "r14"`: x86 **pins a callee-saved GPR**,
  zeroed once at entry and never allocated, purely to realize `xzr`.
- `x86_64/encode/emitter.rs:1194-1195` records why: "AArch64 freely sources `xzr` — most
  importantly `sub d, xzr, r` to negate — but x86 has no zero register."

That pin is expensive. `x86_64/regmodel.rs:35` leaves `INT_ALLOCATABLE` at **five** registers
(`r10, r11, rbx, r12, r13`) against AArch64's nineteen, after excluding `rsp`, `rbp`, `r14`
(zero), `r15` (arena), and the SysV argument/implicit registers. See Open Decisions.

### 2.3 The arena base — the precedent, already built

plan-00-D built exactly the abstraction this plan generalises, but only at the MIR layer:

- `mir.rs:297` — `pub(crate) const ARENA_BASE: &str = "arena_base";`
- `mir.rs:496-498` — `lower_to_mir` rewrites the pinned physical register into that token.
- `aarch64/regmodel.rs:72` — `fn arena_base(&self) -> &'static str` is a `RegisterModel` trait
  method; its doc says the register is *"reserved from allocation — it is absent from
  `allocatable`."*
- Per-ISA realization, with tests: AArch64 `x19`, x86-64 `r15` (`x86_64/select.rs:1043
  arena_base_realizes_to_r15`), riscv64 `s11` (`riscv64/select.rs:694 arena_base_realizes_to_s11`).

What is *not* done: `error_constants.rs:123` still reads `ARENA_STATE_REGISTER = "x19"`, and
3 raw `"x19"` literals remain in `shared/code/`. The neutral name only exists after
`lower_to_mir` swaps it in. This plan pushes the token down to the source.

### 2.4 The link register

`abi::link_register()` returns `"x30"`. Both backends remap it positionally
(`riscv64/select.rs:526` — `"x30" | "lr" => "ra"`). x86-64 has no link register at all; the
`call` instruction pushes the return address. So `"x30"` is an AArch64 implementation detail
that shared code should never have to spell.

## 3. Design Overview

Three registers, three layers, landed in order:

1. **Define the tokens** in `abi`: `ZERO`, `LR`, `ARENA`. Their *values* are the strings each
   backend already accepts today — `"xzr"`, `"lr"`, `"arena_base"` — so the migration is a
   pure source-level rename with **no behavioral surface**. Nothing downstream changes on day
   one. (`riscv64/select.rs:526-527` already maps `"lr"` and `"xzr"`; `mir.rs:297` already
   defines `"arena_base"`.)

2. **Migrate the callers.** Replace `"x31"`, `"xzr"`, `"x30"`, and `"x19"` literals in
   `src/target/shared/code/` with the constants. Repoint `abi::link_register()` at `LR` and
   `ARENA_STATE_REGISTER` at `ARENA`; retire `mir.rs:496-498`'s late rewrite, which becomes a
   no-op once shared code emits the token directly.

3. **Retire the aliases** in the backends: drop the `"x31"` arm from
   `riscv64/select.rs:remap_register`, the `"x30"` arm, and any dead `"rzero"`/`"zero"` arms
   in `x86_64/encode/operand.rs:44`, leaving one canonical spelling per role.

**Where the correctness risk concentrates:** step 2's 213-site zero migration, and only if a
site meant `sp` rather than zero. It cannot — `sp` is spelled `"sp"` — and the byte-identical
gate catches any misclassification, because a wrong register changes emitted bytes. Retiring
`mir.rs:496-498` is the one step with real semantic content: it must be verified that no
*other* producer still emits the physical arena register into the MIR stream.

**Rejected alternative — make the token values fresh strings (`"%zero"`, `"%lr"`).** Cleaner
and consistent with `regalloc/mod.rs:26`'s `%`-sentinel convention, but it forces a
same-commit change to three encoders and turns a provably-inert rename into a behavioral
change. Land the existing accepted spellings first; renaming the *value* is a one-line change
once every producer routes through the constant. plan-34-B introduces `%`-sentinels where they
are load-bearing (there, an unmigrated site must fail to compile); here they are not.

**Rejected alternative — model zero as an immediate, not a register.** Correct in the long
run — it is what x86-64 actually wants (`emitter.rs:1194`), and it would free `r14`, growing
x86's allocatable set from 5 to 6 registers, a 20% increase under the tightest pressure of any
target. But it changes instruction selection and therefore emitted bytes, so it cannot ride
this plan's byte-identical gate. See Open Decisions.

**Rejected alternative — fold the closure-environment register in here.** `x28` looks like an
invariant register but is not: it is set immediately before an indirect call and read by the
callee (`builder_emit_helpers.rs:223-224`), i.e. an implicit *argument*. It belongs with
plan-34-B's call-boundary tokens. Filing it here would put it under the wrong concept and
miss the reservation question plan-34-C raises.

## 4. Detailed Design

### 4.1 The constants

In `src/arch/aarch64/abi.rs`, beside `stack_pointer()` (`:92`):

```rust
/// The zero register as a register operand — the constant 0 readable as a source,
/// a discard as a destination. AArch64 spells it `xzr`; RISC-V maps it to the
/// hardware `zero`; x86-64 has none and pins `r14`. Never `"x31"`.
pub(crate) const ZERO: &str = "xzr";

/// The link register (return address). AArch64 `x30`, RISC-V `ra`; x86-64 has no
/// such register — `call` pushes the return address. Never `"x30"`.
pub(crate) const LR: &str = "lr";

/// The pinned arena base pointer — a program-wide invariant, reserved from
/// allocation on every ISA (`RegisterModel::arena_base`). AArch64 `x19`,
/// RISC-V `s11`, x86-64 `r15`. Never `"x19"`.
pub(crate) const ARENA: &str = mir::ARENA_BASE;   // "arena_base"
```

`abi` is already imported unqualified by shared code (`mod.rs:4`), so call sites read
`abi::ZERO` with no new import. `link_register()` becomes `LR`; `error_constants.rs:123`
becomes `ARENA_STATE_REGISTER = abi::ARENA`.

### 4.2 The migration

Site classes, from the Phase 1 audit. Every zero site is a *source* operand or a discard
*destination*; none is an address base:

- store source — `abi::store_u64(abi::ZERO, ptr, OFFSET)` (zeroing a record field)
- negate — `abi::subtract_registers(dst, abi::ZERO, src)`
- compare-against-zero and move-zero forms

#### Phase 1 audit — actual findings (2026-07-10)

The tree evolved since this plan was drafted: **244** `"x31"` and **26** `"xzr"` in
`src/target/shared/code/` (not 213 / ~6), **0** `"x30"` (all link-register uses already route
through `abi::link_register()`), and **2** `"x19"` (`error_constants.rs:123` the constant, and
`mir.rs:1431` inside a unit test that deliberately feeds the physical name to exercise the
`lower_to_mir` rename).

`"x31"` site classes — none is an address base or an `sp`-semantics destination:

| class | count | x86 realization today |
|---|---|---|
| `store_u64` / `store_u8` source (zero a field) | 240 | select rewrites `"x31"` → `r14` (`ZERO_REGISTER`) → `mov [mem], r14` |
| `subtract_registers(dst, "x31", src)` negate | 3 | `"x31"` → `r14`, `is_zero_token(14)==false` → **general** `alu3` (`mov;sub` / `neg;add r14`) |
| inspector `i.get("src") == Some("x31")` (`tls/macos.rs:3732`) | 1 | a pattern match over emitted CodeOps, not an operand |

`"xzr"` sites (26): all `subtract_registers(dst,"xzr",src)` negates and `add_carry` carry
in/out sentinels. On x86 `"xzr"` → `16` (`is_zero_token`) → the `neg` / no-carry paths.

**The one place a naive rename is not byte-inert: x86-64 treats the two spellings
differently.** `"x31"` is rewritten to the pinned zero register `r14` in `select`, while
`"xzr"` stays the `16` "no register" sentinel that `alu3`/carry branch on. Consequently the
**3** `subtract_registers(dst,"x31",src)` negate sites emit x86's *general* two-instruction
form today, whereas the 26 `"xzr"` negates emit the canonical one-instruction `neg`. AArch64
(`"x31"`==`"xzr"`==31) and RISC-V (both → `zero`) are byte-identical across the pair; **only
x86 diverges, and only for those 3 sites.**

Resolution (decided in Phase 1, **verified byte-identical in Phase 2**): `abi::ZERO = "xzr"`,
migrate **every** zero site including the 3 negates. The one required backend change is
teaching the x86 store encoder (`mem_store`) to materialize `r14` for a zero-token source, so
all 240 store sites stay byte-identical on x86 (a bare `xzr` would otherwise reach the store
path as x86's "no register" sentinel `16` and mis-encode). LR is handled by widening x86
`select`'s LR-drop `retain` to cover the `"lr"` token as well as `"x30"`.

Empirically, the 3 `subtract_registers` negate sites are **also byte-identical** on x86 — the
predicted "general → `neg`" divergence did not materialize; the encoded `.nobj` machine code
is identical. **All four targets are byte-identical**, proved two ways: the committed
`artifact-gate.sh` gate (macOS-aarch64: `.nobj`/`.nir` unchanged, only `.ncode`/`.mir` *text*
dumps show the rename) and a full four-target cross-emission `.nobj` diff of every codegen
test (447 tests × {macos-aarch64, linux-aarch64, linux-x86_64, linux-riscv64} → **0 diffs**),
pristine-worktree baseline vs. migrated tree.

### 4.3 Retiring the `lower_to_mir` arena rewrite

`mir.rs:496-498` exists to convert the physical arena register into `ARENA_BASE` before
selection. Once `ARENA_STATE_REGISTER == abi::ARENA`, shared code emits the token directly and
the rewrite is the identity. Delete it **only after** a guard test proves no producer emits
the physical name — otherwise a stray `"x19"` would reach `x86_64/select.rs`'s
`map_scratch_register(19)` and be realized as `rbp`, **not** the arena base `r15`. That is a
live footgun today and the single strongest reason to push this token down to the source.

### 4.4 Backend alias retirement

- `riscv64/select.rs:524 remap_register` — reduce `"x31" | "xzr"` to `"xzr"`; `"x30" | "lr"`
  to `"lr"`.
- `x86_64/encode/operand.rs:44` — keep the `"xzr"` sentinel; audit `"rzero"`/`"zero"` for live
  producers and delete dead arms.
- Behavior-preserving *given* Phase 2, which is why Phase 3 lands after it.

## Compatibility / Format Impact

Emitted machine code, `.mfp` package format, and the diagnostic surface are unchanged.

The `-mir` dump **text** changes (`x31` → `xzr`, `x30` → `lr`) and its goldens must be
refreshed. Do not conflate the two gates: `.ncode` bytes are unchanged; `-mir` text is not.

## Phases

### Phase 1 — Audit (no code change)

Prove every `"x31"` means zero, and inventory the `"x30"`/`"x19"` sites. Lands nothing.

- [x] Enumerate all `"x31"` operands (244) in `src/target/shared/code/`, recording the `abi::`
      helper each is passed to and its operand position (source / dest / base).
- [x] Confirm no site passes `"x31"` as an address base or an `add`/`sub` destination that
      would imply `sp` semantics. (None do — 240 store sources, 3 negate lhs, 1 inspector.)
- [x] Enumerate the `"xzr"` sites (26, all negate/carry-sentinel), the `"x30"` sites (0 — all
      via `abi::link_register()`), and the `"x19"` sites (2: the constant + one unit test).
- [x] Confirm nothing outside `lower_to_mir` produces a physical arena register into the MIR
      stream (§4.3). (`ARENA_STATE_REGISTER = "x19"` is the sole producer; `mir.rs:1431` is a
      test.)
- [x] Write the findings into this file as §4.2's site-class table. (See §4.2 Phase 1 audit.)

Acceptance: the audit table is in this document and accounts for every site with zero
unclassified. A site that resists classification is an Open Decision, not a silent assumption.
Commit: —

### Phase 2 — Introduce the tokens and migrate every caller

The whole rename, gated on byte-identity. **DONE + verified (2026-07-10).**

- [x] Add `ZERO`, `LR`, `ARENA` to `src/arch/aarch64/abi.rs` (§4.1); repoint
      `abi::link_register()` (→ `LR`) and `error_constants.rs:123`
      (`ARENA_STATE_REGISTER = abi::ARENA`). The physical `x19` realization moved to the
      aarch64 backend as `regmodel::ARENA_BASE_REGISTER` (mirroring riscv's `s11`);
      `arena_base_realization()` and `select_aarch64` now source it there, so
      `ARENA_STATE_REGISTER` is free to be the neutral operand token.
- [x] Replace all `"x31"`/`"xzr"`/`"x30"`/`"x19"` literals in `src/target/shared/code/` with
      the constants (37 files; `"x30"` was already 0 sites — all via `link_register()`).
- [x] **Required backend change for byte-identity:** teach x86 `mem_store` to materialize
      `r14` (`ZERO_REGISTER`) for a zero-token source, and widen x86 `select`'s LR-drop
      `retain` to cover `"lr"`. Without the store change a bare `xzr` mis-encodes on x86 (the
      `16` "no register" sentinel).
- [x] Add a guard test (`mir.rs::invariant_registers_are_neutral_tokens`) asserting the tokens
      are neutral and that lowering abi-emitted zero/lr/arena streams leaks no physical
      `x19`/`x30`/`x31` into the MIR. Seed of plan-34-C's invariant.
- [x] Refresh the `-mir` (2) and `.ncode`/`.app.ncode` (6) macOS-aarch64 goldens for the text
      change (register mnemonics; `.nobj`/`.nir` machine code unchanged).
- [x] Left `abi.rs`'s `syscall_register() == "x8"` assertion untouched (plan-34-B owns it).

Acceptance: **met.** `scripts/artifact-gate.sh` → 0 diffs (macOS-aarch64 `.nobj`/`.nir`
byte-identical). Full four-target `.nobj` cross-emission diff (447 codegen tests ×
{macos-aarch64, linux-aarch64, linux-x86_64, linux-riscv64}) vs. a pristine-HEAD worktree →
**0 diffs**. `cargo test` → 2479 passed. The guard test fails if a literal is reintroduced.
Commit: —

### Phase 3 — Retire the backend aliases (partial: the RISC-V zero/lr aliases + x86 dead arms)

**Landed the retirements whose only live producers were inside plan-34-A's reach; kept the two
whose live producers are out-of-scope platform code. Commit 523f1e89.** The precise producer
audit (not the loose "~161 sites" first cited) is what decided each: of the platform literals,
`x30`=0 (all via `link_register()`), `x31`=10 (only **one** in production RISC-V), `x19`=158
(all aarch64/GTK **app-mode** helpers).

- [x] **RISC-V `remap_register` — narrowed** `"x31" | "xzr"` → `"xzr"`, `"x30" | "lr"` → `"lr"`.
      The sole production RISC-V producer of `"x31"` (`linux_riscv64/code.rs`'s zero store) was
      migrated to `abi::ZERO` first; the two riscv `select` tests now feed the tokens. A stray
      physical `x31`/`x30` now falls through to an invalid `a{n}` and the encoder rejects it
      **loudly** — not the silent `x31`→`t6` miscompile the alias would have allowed. Byte-
      identical on all four targets.
- [x] **x86 `operand.rs` — deleted the dead `"rzero"`/`"zero"` arms**, kept `"xzr"`. Confirmed
      no producer (shared or platform) emits them — only two unit tests, now updated to assert
      rejection.
- [ ] ~~Delete `lower_to_mir`'s physical→`arena_base` rewrite.~~ **Kept, with a concrete
      blocker.** 158 aarch64/GTK **app-mode** helpers (`macos_aarch64/app/*`, `linux_gtk/*`)
      still emit raw `"x19"` for the arena base and route through `lower_to_mir`, which
      neutralizes it to `arena_base` before selection. On **x86-GTK** that is load-bearing for
      correctness: without it a raw `x19` reaches `x86 select::map_scratch_register(19)` and
      realizes to `rbp`, not the arena `r15`. It also renders helpers neutrally in the `-mir`
      dump (via `build_mir_plan`'s post-select fallback). Deleting it requires migrating those
      158 app-mode sites — app-platform scope beyond this plan (plan-34-B/C).
- [ ] ~~x86 `select`'s `"x31" => r14`.~~ **Kept** — not targeted by §4.4 (which only asked to
      remove the dead `operand.rs` arms). The x86 platform backend still spells zero as `"x31"`;
      it is harmless (x86 has no `x31` register, so no ambiguity) and realizes to `r14` exactly
      as intended.
- [x] Confirmed the AArch64 encoder still accepts bare `"x30"`/`"x31"` from non-shared producers.

Acceptance: **met for what landed.** `artifact-gate.sh` 0 diffs; full four-target `.nobj`
cross-emission diff (447 tests × 4) vs. pristine-HEAD → **0 diffs**; `cargo test` 2479 passed.
The two kept items are blocked on out-of-scope app-platform code, not on any risk in this plan.
Commit: —

## Validation Plan

- **Tests:** the three-literal guard test (Phase 2); refreshed `-mir` goldens; the two
  `arena_base` realization tests, retained and re-pointed (Phase 3); existing `regalloc` unit
  tests unchanged.
- **Byte gate:** `scripts/artifact-gate.sh` — execution-free, ~5 min, and the primary proof.
  This plan is *defined* by producing identical bytes.
- **Runtime proof:** per `.ai/compiler.md`'s completion gate, byte-identity is not sufficient
  on its own. Run `tests/acceptance/` natively on macos-aarch64 and linux-aarch64, and on
  linux-riscv64 via `ssh -p 2229`. linux-x86_64 boxes were down as of the last plan-31 run —
  confirm availability or record the gap explicitly rather than claiming coverage.
- **Doc sync:** `src/docs/spec/memory/06_native-calling-convention.md` and
  `07_runtime-helper-abi.md` describe operands in AArch64 names; add `ZERO`/`LR`/`ARENA` to the
  register vocabulary. Both files have uncommitted working-tree edits — coordinate.
- **Acceptance:** `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Token values: existing spellings vs `%`-sentinels.** *Recommended: `"xzr"` / `"lr"` /
  `"arena_base"`* — every backend accepts them today, making Phase 2 provably inert. plan-34-B
  uses `%`-sentinels because there an unmigrated site must fail to compile; that pressure does
  not exist here. (§3)
- **Zero-as-immediate.** Modelling zero as an immediate rather than a register frees x86's
  pinned `r14`, taking `INT_ALLOCATABLE` from 5 to 6 — a 20% register-pressure win on the
  target that needs it most. It changes emitted bytes. *Recommended: out of scope here, file
  as follow-on* and note it is worth more than it first appears. (§2.2, §3)
- **Should `abi` live in `arch::aarch64` at all?** It is neutral by definition. *Recommended:
  leave it for now* — hoisting it is plan-34-B Phase 2, and doing it here would couple two
  independently-landable plans. (§3)
- **`abi::IO_PRINT_CLOBBERS`** (`abi.rs:4`) names `x9` and `x16` physically. Out of scope here;
  plan-34-C §4.3 owns it. (§2.1)

## Summary

The engineering risk is almost nil: all three registers already have a canonical spelling that
every backend accepts, and the byte-identical gate catches any site misclassified during the
rename. The one place it bites is §4.3 — a stray physical `"x19"` reaching `x86_64/select.rs`
is realized as `rbp`, not the arena base `r15`, and that is a silent memory-corruption path
that exists **today**. Pushing the `arena_base` token down to the source closes it.

What this leaves untouched: the call boundary (plan-34-B), every hand-picked scratch register
(plan-34-C), and the emitted bytes of all four targets.
