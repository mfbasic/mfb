# plan-88-C: migrate the symbol-path emitters + declare helper errors

Last updated: 2026-08-03
Effort: large (3h–1d)
Depends on: plan-88-B (all per-call-site emitters must already be on
`raise_error`/`raise_error_bare` — if B is not complete, C cannot start, full
stop.)

Sub-plan **C** of plan-88. See plan-88-A §3 for the overall design. C migrates
the **symbol path** — the 49 `push_error_message_address(.., ERR_*_SYMBOL, ..)`
sites across 11 fixed-native-helper files — onto the same `raise_error` /
`raise_error_bare` primitive, and declares the `errors` contract for the builtins
those helpers implement. After C, the used-set is populated by *every* error
emission in the codebase (per-call-site from B, symbol-path from C), which is the
precondition D needs to switch emission over and delete the manual gating.

Behavioral outcome for C: **every symbol-path error emission goes through
`raise_error`/`raise_error_bare` and records into the used-set; the compiled
program raises the same error code+message as before (byte-identity is expected
to break here — the emission unifies — and goldens re-baseline).**

References: plan-88-A; `src/target/shared/code/data_objects.rs`
(`push_error_message_address`); the 11 symbol-path files (below);
`src/target/shared/code/app.rs` (`prepend_wrong_mode_gate`, already takes a
`function_id`); `.ai/compiler.md`.

## Prerequisites

See plan-88-A §Prerequisites (shared). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-88-B complete | `ls planning/completed/plan-88-B-*` | MET (`402aa0596`) |
| No `emit_*_return` **call sites** remain | `grep -rc 'self\.emit_[a-z_]+_return()' src/target/shared/code/*.rs` → 0 | MET |
| **The symbol-path helpers are `&mut CodeBuilder` methods (can call `raise_error`)** | `grep -nE 'fn (lower_io_\|lower_term_helper\|emit_thread\|thread_queue\|lower_thread\|prepend_wrong_mode_gate)' … \| grep -c self` → **0** | **NOT MET** |

> **PREREQUISITE DEFECT (premise falsified) — the entry gate should have tested
> this.** All 48 symbol-path sites live in **free functions** (e.g.
> `pub(super) fn lower_io_poll_input_helper(symbol, platform_imports, platform,
> app_mode) -> HelperResult`, `prepend_wrong_mode_gate(instructions, relocations,
> …)`) that build a `Vec<CodeInstruction>` directly via the free function
> `push_error_message_address(instructions, relocations, …)`. `raise_error` /
> `raise_error_bare` are `&mut self` **`CodeBuilder` methods** — a free function
> cannot call them. `grep -c self` over every symbol-path helper fn = 0.
>
> So C's design ("the sites converge on the two methods") **cannot work as
> written**. Resolving it needs a design decision with real tradeoffs, and one
> option contradicts the feature's explicit "exactly two entry points" invariant:
> - **(a) Refactor** the ~11 fixed-helper free functions into `CodeBuilder`
>   methods so they can call `raise_error`. Large and structurally invasive — these
>   emit *fixed runtime routines* (`_mfb_rt_io_*`, `_mfb_rt_thread_*`) built once
>   per program from helper-registration code, not per-statement lowering; they
>   have no `self`/`current_loc` and returning a `HelperBody` is their contract.
> - **(b) A free-function / fragment emitter** (`raise_error_into(instructions,
>   relocations, used_errors, name)`) that shares the table + used-set. Smaller,
>   but it is a **third** emission entry point — it nuances (or breaks) the
>   Definition-of-done invariant #1 ("exactly two"). It could also **preserve the
>   helpers' lightweight register-set emission byte-identically** (table-sourced
>   code + message symbol), avoiding C's golden churn entirely.
>
> This is a user decision (it bears on the explicit invariant), so plan-88 is
> paused here after A + B. See Corrections.

## 1. Goal

- Zero remaining `push_error_message_address(.., ERR_*_SYMBOL, ..)` calls: every
  symbol-path error emission replaced by `raise_error(func_id, name)` (helper
  implements a named builtin) or `raise_error_bare(name)` (shared runtime helper
  with no owner), each recording into the used-set. Every builtin whose helper
  raises an error declares that error in its `BuiltinFunction.errors`.

### Non-goals

- Do **not** delete `ERR_*` constants, wrappers, or the manual gating — that is D.
- Do **not** activate used-set-driven emission — that is D. In C the manual
  `push_string_value` gating in `data_objects.rs` still emits the strings; C
  changes how the error is *raised* (through the two methods).
- **Runtime behavior is preserved** (same code+message). Codegen output **does**
  change (the helper shape unifies) — that is intended, and goldens re-baseline.
  What must not change is the error a program observes at runtime.

## 2. Current State (delta from B)

After B, all per-call-site emitters use `raise_error`. The remaining error
emitters are the symbol path:

| What | Count | Command |
|---|---|---|
| `push_error_message_address` calls | 49 | `grep -rcE 'push_error_message_address' src/target/shared/code/*.rs \| awk -F: '$2>0{s+=$2} END{print s}'` → 49 |
| files (excl. `data_objects.rs`) | 11 | `app.rs, datetime.rs, io_stdin.rs, float_format.rs, entry.rs, io_stdout.rs, native_helpers.rs, io_terminal.rs, runtime_helpers_thread.rs, runtime_helpers.rs, term.rs` |
| distinct `ERR_*_SYMBOL` referenced | 41 | `grep -rhoE 'ERR_[A-Z_]+_SYMBOL' src/target/shared/code/*.rs \| grep -v error_constants \| sort -u \| wc -l` → 41 |

### Verified / to-verify properties

- **Some symbol-path files implement named builtins** (have a
  `BuiltinFunction`): `datetime.rs` (`datetime.*`), `io_stdin.rs` (`io.input`/
  `pollInput`/…), `io_stdout.rs`/`io_terminal.rs` (`io.*`), `term.rs` (`term.*`),
  `app.rs` (`app.*` + the shared `prepend_wrong_mode_gate`). These → `raise_error(
  func_id, name)`.
- **Some are shared runtime helpers with no single owner**: `native_helpers.rs`,
  `runtime_helpers.rs`, `runtime_helpers_thread.rs`, `entry.rs`, `float_format.rs`.
  These → `raise_error_bare(name)`.
- **The per-site owner classification is UNVERIFIED per site** and is the first
  task of C: for each of the 49 sites, `grep` its enclosing `pub(super) fn` and
  decide func-vs-bare (as in B). `prepend_wrong_mode_gate` already carries a
  `function_id` (worktree change) → `raise_error(function_id, "ErrWrongMode")`.

### The emission-shape change (central to C) — and why byte-identity is NOT the gate

Today the helper sites emit a **different, lighter shape** than the per-call-site
path: the fixed helper sets the result return registers directly
(`RESULT_VALUE_REGISTER` = code, `RESULT_TAG_REGISTER` = err-tag) and calls
`push_error_message_address` only to load the message address into
`RESULT_ERROR_MESSAGE_REGISTER`, then returns — **no `ErrorLoc`, no
`_mfb_make_error_result` call.** The per-call-site path (and therefore
`raise_error`) builds the full `Error` via `_mfb_make_error_result`, *with* a
source location.

Converting the helper sites to `raise_error`/`raise_error_bare` therefore **will
change the emitted bytes** (and give those errors a source loc they lacked, and
route them through `_mfb_make_error_result`). **That is intended** — it is the
unification that makes invariant #1 (two entry points) true. Preserving the old
fragment shape would require a third emission method, which is exactly what the
feature forbids. So C does **not** chase byte-identity; it re-baselines the
affected goldens and proves **runtime behavior is preserved** — the same error
code + message is raised — plus the intended, documented deltas (source loc now
present; `_mfb_make_error_result` now used). The correctness risk is that a
converted helper raises the *wrong* error or corrupts its result registers, not
that its bytes moved.

## 3. Design

- **No new primitive.** Every helper site is rewritten to call the *same* two
  methods everything else uses — `raise_error(func_id, name)` or
  `raise_error_bare(name)`. There is deliberately no fragment primitive; adding
  one would be a third emission path and break invariant #1. The helper's own
  inline register-setting + `push_error_message_address` sequence is deleted and
  replaced by the single `raise_error` call, which owns the whole error return.
- Migrate **one file at a time**; re-baseline that file's goldens and verify
  runtime behavior (same code+message; the intended source-loc / make-error
  deltas).
- For each helper implementing a named builtin, add its raised errors to that
  builtin’s `BuiltinFunction.errors` (`src/builtins/*.rs`).
- `prepend_wrong_mode_gate` already has `function_id`: replace its hardcoded
  `ERR_WRONG_MODE_CODE` + `push_error_message_address(ERR_WRONG_MODE_SYMBOL)` with
  `raise_error(function_id, "ErrWrongMode")`, and add `"ErrWrongMode"` to the
  `errors` of the gated builtins (`io.input`/`readLine`/`readChar` and the gated
  `term.*` set — enumerate from the two call sites in `mod.rs`).

## Phases

### Phase 1 — free-function emitter + table symbol column (infrastructure)

Superseded design (see Corrections C-1): sites are free functions, so they use the
byte-identical free-function emitter `raise_error_into`, NOT the `raise_error`
methods; goldens stay **constant**, not re-baselined.

- [x] Add a 4th `symbol` column to `ERRORCODE_CONSTANTS` (the exact historical
      `ERR_*_SYMBOL` per error, irregular ones included) + `runtime_error_emission(name)
      -> (code, symbol)`. `errorcode.rs`.
- [x] Add `data_objects::raise_error_into(from, name, instructions, relocations)` —
      emits `move code; move ERR tag; push_error_message_address(symbol)` from the
      table, byte-identical to the historical fixed-helper sequence.
- [x] Parity pins: `emission_symbols_match_codegen_constants` (every emitted symbol =
      its `ERR_*_SYMBOL`), and the 4-tuple update of `error_constants_match_table`.

Acceptance: `cargo test --bin mfb` green for the errorcode + parity tests; the
emitter compiles. No golden re-baseline (byte-identical by construction).
Commit: 83cd6955c

### Phase 2 — convert every emission site to the table (~307, not 49; see C-2)

- [x] `emit_fail` (both defs: `native_helpers` + the `net/mod.rs` duplicate) takes an
      error *name* and delegates to `raise_error_into`; all **216** `emit_fail`
      callers (crypto, audio/, tls/, crypto_ec/, net/) pass a name instead of a
      `(ERR_*_CODE, ERR_*_SYMBOL)` pair.
- [x] All **91** direct `push_error_message_address` callers + the top-level fixed
      helpers (io_stdin/io_stdout/io_terminal/term/app/datetime/entry/float_format/
      runtime_helpers/runtime_helpers_thread + fs/*, os/*) converted to
      `raise_error_into` — the two `move` lines dropped, the push replaced.
- [x] `prepend_wrong_mode_gate` uses `raise_error_into("ErrWrongMode")`; the
      exploratory `function_id`/`_builtin` thread is reverted (the free-function
      emitters do not validate against `builtin.errors`, uniformly).
- [x] Inline-message family (missed by the push census): 3 inline `adrp`/`add`
      (io_stdin/io_stdout) + 11 `emit_data_address` (link_thunk incl. boundary loop)
      → `raise_error_into`; 3 per-target app unsupported emitters (win/macOS/linux_gtk)
      → table-driven via `runtime_error_emission` + local loader; macOS duplicate
      `ERR_UNSUPPORTED_*` const deleted; parity test extended to all emitted names.

Acceptance: zero `push_error_message_address` callers remain outside its own def +
`raise_error_into` (`grep -rn` → those two only); zero `emit_fail` calls pass an
`ERR_*` constant; `cargo build` clean.
Commit: 83cd6955c

### Phase 3 — verify byte-identity + runtime error parity

- [x] `cargo test --bin mfb` green (incl. the errorcode + parity tests). **3754 passed.**
- [ ] `scripts/artifact-gate.sh` (byte-identity goldens): **0 diffs** — the emitter
      reproduces every fixed-helper sequence exactly, so no golden churn (the inverse
      of the superseded "re-baseline" plan). Any diff is a wrong name→symbol mapping
      to fix, not a re-baseline.
- [ ] `tests/rt-error/**` acceptance (`scripts/test-accept.sh`) green — every
      converted error path raises the same `Error.code`+message as before (no code or
      message change anywhere; the `ErrWrongMode` table-message consolidation is D).

Acceptance: `cargo test --bin mfb` green; artifact-gate 0 diffs; rt-error accept
green; `grep -rn push_error_message_address src/` shows only the def + emitter.
Commit: —

## Validation Plan

- Tests: `cargo test --bin mfb` after each phase; the `io`/`term`/`datetime`/
  `thread`/crypto runtime tests exercise the converted helpers.
- Coverage check: confirm those runtime tests are in the bin suite denominator
  (some resource/network tests are pre-existing reds — filter by name-glob and
  compare against the baseline, per the acceptance-preexisting-reds note).
- Runtime proof: an app-mode program that hits the wrong-mode gate, and a stdin
  read from an unsubscribed thread (`ErrInvalidContext`), still raise the same
  codes/messages after C.
- Codegen goldens: `scripts/artifact-gate.sh` must show **0 diffs** (byte-identical
  approach — Corrections C-1). Goldens are held CONSTANT, never re-baselined. A diff
  means a wrong name→symbol/code mapping (root-cause and fix it), not a re-baseline.
- Doc sync: none.
- Acceptance: `cargo test --bin mfb` green + runtime proof that every converted
  helper raises the same code+message. After C, verify `used_errors` is populated
  by BOTH paths (a test that the module’s used-set is non-empty on a program
  exercising a symbol-path helper) — the precondition D relies on.

## Open Decisions

- None. (The former "fragment primitive" decision is resolved: **no** fragment
  primitive — the helper sites converge on the two canonical methods even though
  their bytes change. Preserving the fragment shape is explicitly rejected because
  it would be a third emission path, violating invariant #1.)

- **To confirm in the pilot (not a design fork):** that giving helper errors a
  source location and routing them through `_mfb_make_error_result` is acceptable
  behavior (it is the intended unification). If any helper error must stay
  source-loc-free for a runtime reason, record it in Corrections — but the default
  is: all errors are built one way.

## Corrections

**C-1 (design, premise defect resolved by user decision).** The symbol-path sites
are **free functions** (`grep -c self` = 0), so they cannot call the `&mut self`
`raise_error`/`raise_error_bare` methods (see Prerequisites defect). Per the user's
decision (2026-08-09), C adopts a **byte-identical free-function emitter**:
`data_objects::raise_error_into(from, error_name, instructions, relocations)` emits
the historical lightweight fixed-helper sequence (`move code; move ERR tag;
push_error_message_address(symbol)`) sourcing `(code, message-symbol)` from
`ERRORCODE_CONSTANTS` — a new **4th `symbol` column** on the table. This is a third
emission entry point (nuancing DoD invariant #1 to "two methods + their shared
free-fn form"), and it keeps `.ncode` **byte-identical** (no golden churn). The
table-message consolidation for the one divergent code (`ErrWrongMode`) stays a
**D** concern — C changes only the *emission*, not the message data-object content,
so no runtime code/message changes anywhere.

**C-2 (census was wrong by ~6×; measured by recursive grep).** The plan's "49
symbol-path sites" came from a **non-recursive** glob (`src/target/shared/code/*.rs`)
that silently excluded every subdirectory (`fs/ os/ net/ audio/ tls/ crypto_ec/`) —
the "census-by-grep path-exclusion" trap. The true population, by
`grep -rn ... src/`:
- **91** direct `push_error_message_address` callers (fs/*, os/*, net/*, term.rs, …).
- **216** `emit_fail(...)` callers (the shared error-tail helper in `native_helpers`)
  across net/, audio/, tls/, crypto_ec/, crypto.rs — each passing a matched
  `(ERR_*_CODE, ERR_*_SYMBOL)` pair (only alias: OOM code ↔ allocation symbol).
- **2** variable-`message_symbol` helpers (`native_helpers::emit_fail` — converted;
  `net/mod.rs` classifier — handled in place).

Then a THIRD family surfaced — sites that inline the message-load instead of
calling `push_error_message_address` (so the first census missed them entirely):
- **3** inline `adrp`/`add_pageoff` sequences (io_stdin ×2, io_stdout ×1).
- **6** `emit_data_address(from, RESULT_ERROR_MESSAGE_REGISTER, ERR_*_SYMBOL, …)` in
  `link_thunk.rs` + **5** more in its boundary-validation loop (overflow/encoding/
  float-nan/float-inf/invalid-argument).
- **3** per-target app `terminalSize`-unsupported emitters (win/macOS/linux_gtk)
  that use a custom x0/x1/x2 ABI (macOS even had a *duplicate local*
  `ERR_UNSUPPORTED_*` const, now deleted). These cannot call the shared
  `raise_error_into` (private `data_objects` module / target-boundary), so each is
  made table-driven via `runtime_error_emission(name)` + its local loader —
  byte-identical, still one metadata source.
Verified byte-identical: `abi::load_page_address`/`add_page_offset` and
`emit_data_address`/`load_addr` emit the exact `adrp`/`add_pageoff` + DataAddrHi/Lo
relocs that `push_error_message_address` does (reloc push-order interleaving is
irrelevant — the two vectors end up identical).

Total ≈ **330** emission sites, not 49. Re-scoped in place; the approach is
unchanged. One further irregular symbol surfaced: `ERR_DIRECTORY_NOT_EMPTY_SYMBOL`
(code 77030005) is the sole fixed emission for `ErrResourceBusy`, so the table pins
that historical symbol. Non-emission `ERR_*` uses left for **D**: the data-object
message tables (`shared/code/mod.rs`, `link_thunk.rs`), the arena helper's
code-in-x0 returns (`arena.rs`), and a resource-closed *comparison*
(`builder_resource_cleanup.rs`).

**Per-site classification / message-change audit:** all ~330 sites converted with
zero runtime code/message change (byte-only). Proven comprehensively: a full
`artifact-gate all` with the **pre-C** binary (`ea805b49f`, built in a detached
worktree) produced a diff set **identical** to the post-C binary's — C changes zero
bytes. Two overlap fixtures were also checked byte-for-byte: `crypto-ec-valid`
(pre-C == post-C == `bee5f982…`) and `macos-app-mode-io` (`9b181857…`).

**C-3 (pre-existing golden drift found and fixed — was plan-88-B's "goldens
pending").** The `artifact-gate all` run surfaced **17** stale goldens across 9
`rt-behavior/`+`syntax/` fixtures (crypto-ec-valid, the 3 macos-app-mode-*,
control-flow-if, parser-hello-world, list-ops, func_map_getor, control-flow-match).
These are **not** C's doing (pre-C == post-C above). They are plan-88-**B**'s
allocation unification: the x0-optimised OOM emit (`mov x3, x0`, code returned in
x0 by `_mfb_arena_alloc`) became an explicit `mov_imm x8, "77010001"; mov x3, x8`.
Byte-only — the runtime code (77010001 = `ErrOutOfMemory`) is unchanged. B's commit
`8a336ea95` said "goldens pending" and B's re-baseline (`402aa0596`) covered only
`tests/byte-identity/*.ncodesum`, missing these `.ncode`/`.mir`/`.ncodesum` goldens.
Re-baselined here with the current compiler via the new scoped
`scripts/regen-rt-goldens.sh` (bounded to the 9 fixture dirs); exactly 17 goldens
changed. **Phase-3 acceptance correction:** the plan's "gate → 0 diffs" was
un-meetable on entry because the base already carried this pre-existing drift; the
checkable criteria are (a) C is byte-neutral (pre-C gate == post-C gate) and (b) the
gate is green after completing B's pending re-baseline.

## Summary

C moves the 49 symbol-path emitters across 11 fixed-helper files onto
`raise_error`/`raise_error_bare` — deleting each helper's bespoke register-set +
`push_error_message_address` fragment rather than preserving it. This deliberately
**changes the bytes** (helper errors gain a source loc and go through
`_mfb_make_error_result`); goldens re-baseline and the gate is a runtime proof
that the same error is raised, not zero delta. That unification is the point: it
collapses the third emission path so only the two methods remain. After C the
used-set is fed by every error emission in the tree — the precondition D needs.
Untouched: the `ERR_*` constants, wrappers, and manual gating (all deleted in D),
and `ErrWrapped`.
