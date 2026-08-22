# io `Body::abi_function` Migration + File/Function Cleanup Plan

Last updated: 2026-08-19
Effort: x-large (1d–3d)

Migrate the built-in `io` package off the legacy `Body::native_os_seam` +
central `lower_io_helper` `match call` dispatcher onto per-function
`Body::abi_function` clean-room lowerings, and split the monolithic
`native/{mod,stdin,stdout,terminal}.rs` emitters into per-function files owned by
each `func_*.rs` — mirroring the `crypto` package's abi migration and
file/function cleanup.

The single behavioral outcome: every `io::` member lowers through
`lower_abi_function_helper` (classified `is_abi_function_call == true`), each
`func_*.rs` owns its own lowering, the central `lower_io_helper` `match` and the
`native_os_seam` registrations are gone, and **runtime behavior is unchanged** —
the emitted helper *body* stays byte-identical; only the runtime-helper symbol
family changes (`Io` → `Abi`), which is a mechanical rename synced into goldens.

References:

- `src/codegen/builtins/crypto/` — the completed precedent (per-function
  `func_*.rs` with `Body::abi_function`, shared `gen_*.rs` seams).
- `src/codegen/registry/mod.rs` — `Body::abi_function`, `AbiCtx`, `OsLowerCtx`,
  `abi_function_lower`, `is_abi_function_call`.
- `src/codegen/engine/function/function_lowering.rs:1255` —
  `lower_abi_function_helper` (the wrapper that finalizes an abi body).
- `.ai/testing-gates.md`, memory `acceptance-harness-not-in-cargo-test`,
  `test-accept-second-arg-is-rm-rf-scratch` — golden harness mechanics.
- `.ai/resources-packages.md`, `.ai/arch-abi.md` — OS-seam/app-mode emission.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Working tree builds green | `rustup run 1.96.0 cargo build --bin mfb` | MET (assumed; verify at start) |
| crypto tests pass (precedent intact) | `rustup run 1.96.0 cargo test --bin mfb crypto` | MET (19 passed, verified 2026-08-19) |

Everything below is written against the world where these hold.

## 1. Goal

- Every `io::` member is a `Body::abi_function` whose lowering lives in (or is
  owned by) its `func_*.rs`; `is_abi_function_call("io.print")` etc. return true.
- The central `lower_io_helper` `match call` dispatcher and all
  `Body::native_os_seam(lower_io_helper, …)` registrations are deleted.
- `native/{stdin,stdout,terminal}.rs` emitters are split so each member's emitter
  is owned by / co-located with its `func_*.rs` (crypto-style granularity).
- `cargo test` green and the full acceptance golden suite passes after a
  golden re-sync that contains **only** `_mfb_rt_io_*` → `_mfb_rt_abi_*`-style
  symbol renames (no instruction-body diffs).

### Non-goals (explicit constraints)

- **No behavior change.** The emitted helper body (frame, instruction stream,
  stack slots) stays byte-identical per member; the ONLY intended `.ncode`/objdump
  delta is the runtime-helper symbol rename (family `Io` → `Abi`) at the helper
  definition and its call sites. Any instruction-body diff is a bug to root-cause.
- **No public surface change.** Same 15 members, same 17 overloads, same
  signatures/return types/errors/docs. `mfb man io` output unchanged.
- **No app-mode / TUI regression.** The app-transcript routing
  (`emit_app_io_*`), the plan-35-B TUI shadow-grid routing on `io.print`/`io.write`,
  and the bug-149 cooked-mode restore on `io.readLine`/`io.input` behave exactly
  as today.

## 2. Current State

`io` is already on the clean-room registry (`src/codegen/builtins/io/mod.rs`)
with one `func_*.rs` per member, but every member registers
`Body::native_os_seam(Some(lower_io_helper), Some(lower_io_helper), &[])`
(`func_print.rs` et al.), and `lower_io_helper` (`native/mod.rs:51`) is one
`match call { … }` block routing to emitters in `native/{stdin,stdout,terminal}.rs`.

Key mechanics read (not assumed):

- **abi wrapper**: `lower_abi_function_helper`
  (`function_lowering.rs:1255`) builds a `CodeBuilder` seeded with
  `instructions = vec![label("entry")]`, hands the body its incoming arg
  registers as `ValueResult`s, calls the body, and — when the body returns a
  `void` location — finalizes `builder.instructions` via
  `finalize_vreg_body_with_locals(&mut instructions, &[], builder.stack_size)`.
- **finalize invariant**: `finalize_vreg_body_with_locals`
  (`vreg_frame.rs:199`) PANICS if the stream names any physical register
  (`regalloc::find_physical_operand`, plan-34-D). So an abi body must push a
  *pre-finalization vreg stream*, OR bypass finalize entirely.
- **console io emitters** (`native/stdout.rs`, `native/stdin.rs`,
  `native/terminal.rs`) build vreg streams and each finalize with
  `finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE)` —
  `reserved` is ALWAYS `&[]`, matching the wrapper exactly. They return a
  finalized `HelperBody`.
- **app-mode bodies** (`emit_app_io_write_helper` in
  `src/target/macos_aarch64/app/app_io.rs:14`, and GTK/Windows peers) are
  hand-written PHYSICAL-register bodies (`Asm::new`, explicit `subtract_stack`,
  `x19`/`x0`…) with their own `CodeFrame` (`AppHookBody`). They are already
  finalized and CANNOT pass through the wrapper's finalize.
- **dispatch site**: `builder/mod.rs:1913` builds `os_ctx` (with
  `term_state_offset`/`presentation_mode_offset` from `arena_layout`) and, in the
  `is_abi_function_call` branch (`:1919`), calls `lower_abi_function_helper`
  WITHOUT `os_ctx`. `AbiCtx` (`registry/mod.rs:101`) carries only
  `platform_imports`, `platform`, `build_mode`.
- **symbol family**: `catalog.rs:100` routes any `is_abi_function_call` member to
  `RuntimeHelper::Abi` ("abi", `runtime/mod.rs:33`) regardless of package — so
  migrating io flips its helper symbols from the `Io` to the `Abi` family.

### Measured populations

| What | Count | Command |
|---|---|---|
| io members | 15 | counted `register` calls in `io/mod.rs:96-110` → 15 |
| io overloads (`Implementation`s) | 17 | `grep -rh "body: Body::native_os_seam" src/codegen/builtins/io/func_*.rs \| wc -l` → 17 |
| io emitter fns (stdin+stdout+terminal) | 13 | `grep -oE "fn [a-z_0-9]+" native/{stdin,stdout,terminal}.rs \| sort -u \| wc -l` → 13 |
| io console emitters finalizing with `reserved=&[]` | 9/9 | `grep -n finalize_vreg_body_with_locals native/{stdin,stdout,terminal}.rs` → all `&[]` |
| packages still on `native_os_seam` | 6 (audio,io,net,process,term,tls) | `grep -rln native_os_seam src/codegen/builtins/*/mod.rs` |

### Verified properties

- **Console emitters are wrapper-compatible** — all 9 finalize with
  `reserved=&[]` + a per-emitter `FRAME_SIZE`/`16` `local_size`. VERIFIED by
  reading each finalize call.
- **App bodies are pre-finalized physical bodies** — VERIFIED by reading
  `emit_app_io_write_helper` (uses `x19`/`x0`, `subtract_stack`, returns
  `AppHookBody` with a `CodeFrame`). They need the escape hatch, not the finalize.
- **crypto abi bodies leave the hatch unused** — they return a real value or a
  void location and rely on the wrapper's finalize; adding an *optional* hatch
  field defaulted to `None` leaves them byte-identical. VERIFIED by reading
  `func_hash::lower_hash` / the wrapper's void-epilogue branch.

## 3. Design Overview

Each io member becomes a thin **`abi_function` adapter** that calls its
(relocated, per-function) emitter and hands the finished body back. Because both
console and app emitters already return *finalized* bodies, the adapter uses a
new **pre-finalized escape hatch** in the wrapper rather than re-finalizing:

1. **AbiCtx extension** (per the user's directive — AbiCtx holds
   platform-dependent info): add `term_state_offset: Option<usize>` and
   `presentation_mode_offset: Option<usize>`. Thread them from the existing
   `os_ctx` at `builder/mod.rs:1919` into `lower_abi_function_helper` →
   `AbiCtx`. `platform` (already in `AbiCtx`) is the handle for the
   platform-dependent app-hook functions.
2. **Pre-finalized hatch**: `CodeBuilder` gains
   `abi_prefinalized: Option<(CodeFrame, Vec<CodeStackSlot>)>` (default `None`).
   An adapter that already holds a finalized body sets
   `builder.instructions`/`builder.relocations` to the emitter's output and
   `builder.abi_prefinalized = Some((frame, slots))`, then returns a `void`
   `ValueResult`. `lower_abi_function_helper`, when `abi_prefinalized` is `Some`,
   returns `(frame, builder.instructions, builder.relocations, slots)` and SKIPS
   both the void-epilogue and `finalize_vreg_body_with_locals`. When `None`
   (crypto and every other abi member), behavior is exactly as today.
3. **Per-member adapters**: each `func_*.rs` registers
   `Body::abi_function(lower_io_<member>)`; `lower_io_<member>` reproduces the
   `match`-arm logic for that member (app-vs-console branch on
   `ctx.build_mode.is_app()`, using `ctx.term_state_offset` /
   `ctx.presentation_mode_offset` / `ctx.platform`), calls the emitter, and sets
   the hatch.
4. **File split**: the emitters move from the three monolithic
   `native/{stdin,stdout,terminal}.rs` files to per-member ownership (co-located
   in each `func_*.rs` or a per-member `native/*` module), and the central
   `lower_io_helper` `match` + `native/mod.rs` dispatcher are deleted. Shared
   sub-emitters (`emit_stdin_byte_read`, `emit_append_to_stdout_buffer`, …) stay
   shared (a small `native/shared.rs` or kept in `crate::codegen::io::*`).

**Byte-identity is the correctness gate for the emitted body**, and it is the
right gate here: this is provably-neutral code motion. The ONE expected,
non-neutral diff is the helper *symbol rename* (`Io`→`Abi` family) at each helper
definition and its `bl` call sites — that diff reads as the migration working.
Any *instruction-body* diff (registers, order, frame) is a bug: root-cause by
objdumping ONE fixture, fix, continue. Never conclude the design is dead from a
golden diff.

Rejected alternatives:

- *Convert console emitters to raw vreg-stream abi bodies (no hatch).* Works for
  console but not app (physical bodies), so the hatch is needed anyway; doing
  both is more churn for no benefit. Rejected — the uniform adapter+hatch keeps
  every body byte-identical with zero emitter-internals edits.
- *Keep app-mode members on `native_os_seam`, migrate only console members.*
  Leaves a half-cut seam and a surviving `match`; not "the same migration."
  Rejected.
- *Rewrite app hooks as vreg streams.* Huge, per-platform, high risk, no benefit.
  Rejected.

## 4. Detailed Design

### 4.1 AbiCtx + wrapper (`registry/mod.rs`, `function_lowering.rs`, `builder/mod.rs`)

- `AbiCtx` gains `term_state_offset: Option<usize>`,
  `presentation_mode_offset: Option<usize>`.
- `lower_abi_function_helper` gains two `Option<usize>` params (or takes
  `&OsLowerCtx`); constructs `AbiCtx` with them; after `lower()`, branches on
  `builder.abi_prefinalized`.
- `builder/mod.rs:1919` passes `os_ctx.term_state_offset`,
  `os_ctx.presentation_mode_offset`.
- `CodeBuilder::abi_prefinalized` default `None` at every construction site
  (grep the struct literals — `function_lowering.rs` has several; only the field
  add + `..`-free literals need updating).

### 4.2 Per-member adapters

The `match` arms in `lower_io_helper` map 1:1 to members. Each adapter mirrors
its arm. Concretely per family:

- **Trivial (no platform emitter object needed beyond symbol)**: `isBuffered`,
  `setBuffered` — call `lower_io_is_buffered_helper(symbol, app_mode)` /
  `lower_io_set_buffered_helper(symbol, app_mode)`, hatch the result.
- **stdout family** (`print`/`write`/`printError`/`writeError`, `flush`): app
  branch → `ctx.platform.emit_app_io_write_helper` / `_flush_helper`
  (`pad_no_slots`); console branch → `lower_io_write_helper` /
  `lower_io_flush_helper`. Uses `ctx.term_state_offset`.
- **stdin family** (`input`/`readLine`, `readChar`, `readByte`, `pollInput`):
  app branch (input only) → `emit_app_io_input_helper`; console →
  `lower_io_read_line_helper(…, term_state_offset)` etc. Uses
  `ctx.term_state_offset`.
- **terminal predicates** (`isInputTerminal`/`isOutputTerminal`/`isErrorTerminal`):
  app → `emit_app_io_is_terminal_helper`; console →
  `lower_io_is_terminal_helper(symbol, …, fd)`.

Each adapter returns `ValueResult { type_, location: "void", text }`.

### 4.3 File split

Move each `lower_io_*_helper` into its member's `func_*.rs` (or a sibling
`func_*_native.rs`), keeping the shared sub-emitters
(`emit_stdin_byte_read`/`emit_utf8_sequence_read`/`emit_continuation_read`/
`emit_append_to_stdout_buffer`) in one `native/shared.rs`. Delete
`native/mod.rs`'s `lower_io_helper` and the `mod stdin/stdout/terminal` wiring
once every member owns its emitter.

## Compatibility / Format Impact

- **Changed**: io runtime-helper symbols move from the `Io` family to the `Abi`
  family (`_mfb_rt_io_*` → `_mfb_rt_abi_*`), at the helper definition and every
  `bl` call site. Mass golden churn, mechanical rename only.
- **Possibly changed**: per-backend supported-runtime-call gates
  (`src/target/*/plan.rs`, `src/target/shared/validate/mod.rs`) that whitelist io
  symbols by family — must accept the `Abi`-family io symbols (as they already do
  for crypto). VERIFY in Phase 1.
- **Unchanged**: public `io::` surface, docs, `mfb man io`, and the instruction
  body of every helper.

## Phases

### Phase 1 — AbiCtx extension + pre-finalized hatch (enabling, no io change)

Delivers the mechanism with ZERO member migrated, so crypto stays byte-identical
and the hatch/ctx fields are exercised only once io moves.

- [x] Add `term_state_offset`/`presentation_mode_offset` to `AbiCtx`
      (`registry/mod.rs`).
- [x] Add `abi_prefinalized: Option<(CodeFrame, Vec<CodeStackSlot>)>` to
      `CodeBuilder` (`builder/mod.rs`), default `None` at all 4 literals in
      `function_lowering.rs`.
- [x] Thread the two offsets into `lower_abi_function_helper` and honor
      `abi_prefinalized` (skip void-epilogue + finalize when `Some`)
      (`function_lowering.rs`); pass them from `builder/mod.rs` + the
      `try_abi_inline_lower` site (`builder_values.rs`, `None`/`None`).
- [x] Confirm gates accept `Abi`-family symbols generically: `catalog.rs:18` +
      `supported_helper_specs` DERIVE all `Abi`-family specs from the registry
      globally (no per-backend allow-list); crypto already routes through it.
- [x] Tests: `cargo test --bin mfb` green (3605 passed).

Acceptance: `cargo build` + `cargo test --bin mfb` green (3605 passed); no io
member migrated yet so the hatch/ctx fields are inert (`None`) — crypto/bits
paths unchanged. Commit: fd6ab70b3

### Phase 2 — Migrate trivial + terminal-predicate + flush members

Lowest-blast-radius members first (`isBuffered`, `setBuffered`, `flush`,
`isInputTerminal`/`isOutputTerminal`/`isErrorTerminal`).

- [x] Add `abi_function_family` so io keeps the `Io` family (see Corrections) —
      migration is byte-identical, not a symbol rename.
- [x] Add shared `hatch_finalized` / `adapter_app_mode` / `app_unsupported` /
      `lower_is_terminal_common` adapters (`io/native/mod.rs`); `pub(crate)`
      re-export the emitters.
- [x] `lower_*` adapters + `Body::abi_function` in `func_is_buffered.rs`,
      `func_set_buffered.rs`, `func_flush.rs`,
      `func_is_{input,output,error}_terminal.rs`.
- [x] Emitters stay in `native/{stdout,terminal}.rs` for now (file relocation is
      Phase 5); the dead `lower_io_helper` match arms are removed in Phase 5.
- [x] Tests: `cargo test --bin mfb` green (3605 passed).

Acceptance: these members classify `is_abi_function_call == true`; the io +
app-mode golden fixtures pass UNCHANGED (byte-identical, 35 fixtures via
`test-accept.sh`); `cargo test` green. Commit: 7024cf8d4

### Phase 3 — Migrate stdout family (`print`/`write`/`printError`/`writeError`)

App + TUI routing; medium blast radius.

- [x] Shared `lower_write_family` adapter (`native/mod.rs`) branches app/console
      via `ctx`; `func_{print,write,print_error,write_error}.rs` register
      `Body::abi_function`. print/write's String + AttributedString overloads both
      point at the one adapter (shared helper, as pre-migration).
- [x] Tests: `cargo test --bin mfb` green (3605).

Acceptance: byte-identical (Io family kept, see Corrections) — validated together
with Phase 4 in the broad golden run. Commit: 03025c926

### Phase 4 — Migrate stdin family (`input`/`readLine`/`readChar`/`readByte`/`pollInput`)

Largest emitter (`stdin.rs` 49 KB); highest blast radius.

- [x] Shared `lower_read_line_family` (input/readLine, app input-branch) +
      direct adapters for `readChar`/`readByte`/`pollInput`; all five
      `func_*.rs` register `Body::abi_function`. `input`/`pollInput`'s
      Optional-arity params are handled upstream of the helper, unchanged.
- [x] Tests: `cargo test --bin mfb` green (3605).

Acceptance: byte-identical; read fixtures + bug-149 cooked-mode behavior
unchanged (broad golden run). Commit: 03025c926

### Phase 5 — Delete dispatcher + finish file split

- [x] Deleted the `lower_io_helper` `match` dispatcher (`native/mod.rs`) + the
      unused `HashMap` import; every `func_*.rs` is `Body::abi_function`.
- [x] Refreshed the stale `native_os_seam` module docs (`io/mod.rs`,
      `native/mod.rs`, all `func_*.rs`).
- [x] Emitters stay grouped in `native/{stdin,stdout,terminal}.rs` (shared by
      multiple members — the crypto-`gen_*.rs`-seam analog); the shared adapter
      glue lives in `native/mod.rs`. No further per-member file split needed.
- [x] Tests: `cargo test --bin mfb` green (3605); no `native_os_seam` *body* in
      `builtins/io` (remaining refs are historical doc comments).

Acceptance: no `Body::native_os_seam` registration remains in `builtins/io`;
`cargo test` green; 61 io + app + print/read/trap golden fixtures pass
byte-identically. Commit: 03025c926

### Phase 6 — Golden re-sync + full acceptance + fmt

- [x] No golden re-sync needed — io kept its `Io` family, so every io golden is
      unchanged (verified: `test-accept.sh` full run, 1264 tests).
- [x] `rustfmt` (root `--all` + `repository`); committed `223e96b6e`.
- [x] Full `test-accept.sh` surfaced ONE unrelated pre-existing failure
      (`inline-trap-infallible-builtin-valid`, `bits::rl64`), root-caused and
      fixed in `cbc175a64` (see Corrections); re-run clean.

Acceptance: full acceptance suite passes with ZERO io golden changes; the one
mismatch was a pre-existing inline-TRAP bug fixed in place. Commit: 223e96b6e (fmt),
cbc175a64 (bug fix).

## Validation Plan

- Tests: existing io unit tests (`cargo test --bin mfb io`) + crypto regression;
  runtime print/read programs (console and `--app`).
- Coverage check: confirm the migrated members are exercised — the acceptance
  suite and io unit tests run the release `mfb` subprocess (memory
  `mfb-exe-tests-use-release-binary`), so rebuild release before `test-accept.sh`.
- Runtime proof: `io::print`/`io::readLine` program produces identical output
  pre/post; a `term::`-active program's shadow-grid routing is unchanged.
- Doc sync: none — surface/docs unchanged (assert `mfb man io` diff empty).
- Acceptance: `test-accept.sh` full suite green after golden re-sync.

## Open Decisions

- **Emitter home** — co-locate each `lower_io_*` inside its `func_*.rs`
  (crypto-most-faithful) vs. a per-member `native/*.rs` beside it. Recommend
  co-locate in `func_*.rs` for the trivial ones and a `native/shared.rs` for the
  shared sub-emitters; decide per-file in Phase 2–4. (§4.3)
- **Wrapper signature** — pass `&OsLowerCtx` to `lower_abi_function_helper` vs.
  two `Option<usize>` params. Recommend two params (keeps `OsLowerCtx` out of the
  abi path's dependency surface). (§4.1)

## Corrections

- **Symbol family — plan said Io→Abi rename + mass golden churn; actual: io keeps
  the `Io` family, byte-identical, zero churn.** The plan's §3 / Compatibility
  sections assumed `catalog.rs:100` forcing every `abi_function` member to the
  `Abi` family was immovable, so it budgeted a golden re-sync of `_mfb_rt_io_io_*`
  → `_mfb_rt_abi_*` renames plus updates to ~10 files hardcoding the io symbols
  (`builder/mod.rs` drain/broadcast gates, the app worker-shim `IO_WRITE_SYMBOL`
  constants, `operand.rs` label classification, the spec docs, and backend test
  expectations). Actual: the `abi_function`→family routing was the wrong thing to
  hardcode. Introduced `abi_function_family(name)` (`target/shared/runtime/mod.rs`)
  — an `abi_function` member keeps its owning package's `RuntimeHelper` family when
  it has one (`io`→`Io`), falling back to `Abi` only for a package with no variant
  (`crypto`). `helper_for_call` and the catalog derivation both use it. Result: io
  members keep their exact `_mfb_rt_io_io_*` symbols, so the migration is
  **byte-identical** — all 35 io + app-mode + hello-world golden fixtures pass
  UNCHANGED (`test-accept.sh … 'func_io_*' 'io-*' 'macos-app-mode-io' …` → 35
  passed). crypto stays on `Abi`, unchanged. The Non-goal "only intended delta is
  the symbol rename" is thus strengthened to "**no delta at all**"; the Phase 6
  golden re-sync is now a no-op verification, not a re-baseline. This voids the
  per-member gate/symbol-flip work Phases 3–5 anticipated.
- **Pre-existing inline-TRAP bug found by the full acceptance run (fixed).** The
  Phase 6 full run surfaced one mismatch — `inline-trap-infallible-builtin-valid`
  — unrelated to io: `lower_infallible_member` (`builder_values.rs`) lowered an
  inline TRAP on an *infallible* inline builtin via only `try_native_lower` (the
  `common` slot) + the len/toString ladder, never `try_abi_inline_lower`. When
  `bits` migrated onto `Body::abi_inline`, the *fallible* inline path
  (`lower_inline_builtin_raw`) gained a `try_abi_inline_lower` arm but this
  infallible path was overlooked, so an inline TRAP on a total `bits` op
  (`rl64`/`rr*`/`popCount`/`clz`/`ctz`/`bswap*`/`band`/`bor`/`bxor`/`bnot`) failed
  to lower. Verified pre-existing at branch base `67dabec0c` (both `bits.rl64`
  already `abi_inline` and the missing arm already absent) — NOT a plan-101
  regression; the migration merely ran the full suite that exposes it. Fixed by
  adding the missing `try_abi_inline_lower` arm (no capture — the op cannot fail);
  the existing fixture is the guard and passes against its unchanged golden.
  Commit `cbc175a64`.

## Summary

**Complete.** All 15 `io::` members are per-function `Body::abi_function`
lowerings; the central `lower_io_helper` `match` dispatcher is gone. The enabling
insight (`abi_function_family`) let io keep its `Io`-family symbols, so the whole
migration is **byte-identical** — the full acceptance suite (1264 tests) passes
with ZERO io golden changes, and `cargo test` is green (3605). The one enabling
mechanism added — the `CodeBuilder.abi_prefinalized` hatch + the two `AbiCtx`
OS-seam offsets — is inert for every non-io abi body (`None` default), so crypto
and bits are untouched.

What was left untouched: the io OS-seam emitter *bodies* (relocated only in
concept — they stay in `native/{stdin,stdout,terminal}.rs` as shared helpers, the
crypto-`gen_*.rs`-seam analog); the public `io::` surface, docs, and symbols. The
one bug this work surfaced (an inline TRAP on an infallible `abi_inline` `bits`
op) was pre-existing and is fixed in `cbc175a64`.

Ledger: Phase 1 `fd6ab70b3` · Phase 2 `7024cf8d4` · Phases 3–5 `03025c926` ·
inline-TRAP fix `cbc175a64` · fmt `223e96b6e`.

## Follow-on: remove the hatch, decouple the app hooks (plan-101 completion)

The initial migration above kept the `native/` split conceptually and used a
`hatch_finalized`/`abi_prefinalized` bridge for app-mode. A later pass (this same
plan, per the user's directive "no more hatch_finalized wrapper crud") finished it:

- **Flattened `native/`** into per-function `func_*.rs` (deleted the dir).
- **Every io member is now a true vreg `abi_function` body** with no adapter/hatch.
  Console/terminal members emit their vreg stream directly (byte-identical). The
  7 app-touching members (`flush`, `print`/`write`/`printError`/`writeError`,
  `input`/`readLine`) `bl` a **standalone decoupled GUI helper**
  (`_mfb_rt_io_app_*`, emitted in `builder/mod.rs` like `STDOUT_DRAIN`) in app mode.
- **Why decouple, not append:** the app-mode GUI bodies are hand-written raw
  physical-register objc/GTK/Win32 sequences (the `Asm` layer), shared with `term`
  (`emit_term_ok_return`). Appending them into the vreg finalizer would require
  rewriting that foundational GUI-ABI codegen — unverifiable on GTK/Windows here
  and rippling into `term`. Decoupling keeps those bodies **byte-identical**
  (standalone functions, never vreg-finalized); only a `bl` indirection is added,
  which the golden suite fully captures. `emit_app_io_is_terminal` (trivial body)
  was instead reshaped to the append shape.
- **Hatch deleted:** `hatch_finalized`, `adapter_app_mode`, and
  `CodeBuilder.abi_prefinalized` (field + inits + wrapper branch) are gone.
- **Result:** not byte-identical (2 app-mode goldens re-synced — members now `bl`
  the `app_*` helpers); functionally identical. `cargo test` 3611; full acceptance
  suite 1264 passed. Commits `8524fe4cd` (buffered), `9b54ee456` (readers),
  `90208997d` (is_terminal reshape), `855566d19` (app decouple + hatch delete).
