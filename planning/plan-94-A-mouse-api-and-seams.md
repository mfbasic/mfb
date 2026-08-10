# plan-94-A: Mouse events — API, types, and registration seams

Last updated: 2026-08-09
Overall Effort: huge (>3d)
Effort: medium (1h–2h)
Depends on: nothing

Add mouse event support to the `term::` surface (`planning/term.md` item 8 —
clicks/drag/scroll) across CLI and `--app` modes, delivered as an opt-in,
poll-based API over a **unified stdin input decoder** that CLI and every app
backend share. This sub-plan (A) lands the language surface — the two new
functions and three new types — and every registration seam, wired to **no-op
stubs** so the feature typechecks, lowers, and runs (returning "no event") on all
five targets before any decoder or backend exists. It is the hub for the whole
`plan-94` feature: sub-plans **B–E** reference §2–§4 here for the shared design.

Behavioral outcome for A alone: a program can `IMPORT term`, call
`term::enableMouse(TRUE)` and `term::pollMouse()`, and build+run on every target;
`pollMouse()` always returns a `MouseEvent` with `kind = None`, and `enableMouse`
is inert. Nothing decodes mouse input yet.

References:

- `planning/term.md` item 8 (mouse) and item 7 (decoded key input — the same
  decoder, added later).
- `.ai/resources-packages.md` "New builtin-package registration seams" and the
  auto-memory note `adding-a-call-to-an-existing-native-pkg.md` — the seam list.
- The just-landed `term::didResize()` work on this branch (`worktree-term`) — the
  closest precedent for a new no-arg `term::` call and its per-arena state slot.
- `src/docs/spec/app/04_term-backend.md` (term backend spec), `src/docs/spec/language/18_builtin-functions.md` §18.

## Prerequisites

Everything below is written against the world where these hold.

| Must be true | Command | Status |
|---|---|---|
| On a branch, not `main` | `git rev-parse --abbrev-ref HEAD` → not `main` | MET (`worktree-term`) |
| Tree builds clean before starting | `cargo build` → `Finished` | MET |

> The Status column is a snapshot; the Command column is the truth. Re-run before
> starting and before stopping.

## 1. Goal

- `term::enableMouse(enabled AS Boolean)` and `term::pollMouse() AS MouseEvent`
  are registered builtins that build and run on all five targets
  (`linux-{aarch64,riscv64,x86_64}`, `macos-aarch64`, `windows-x86_64`), console
  and `--app`.
- The types `MouseEvent`, `MouseKind`, `MouseButton` resolve and construct.
- `pollMouse()` returns `MouseEvent` with `kind = MouseKind.None` unconditionally;
  `enableMouse(...)` is a no-op. No decoder, no queue, no ANSI emission yet.

### Non-goals (explicit constraints)

- **No behavior change to existing `term::`/`io::` calls.** In particular the
  stdin read path (`io::readChar`/`readByte`/`readLine`, the plan-15 broadcast
  log) is untouched in A — only new symbols are added.
- **No new ANSI bytes emitted** by `term::on`/`off` in A (mouse tracking is
  opt-in and unimplemented here).
- **API shape is frozen by this sub-plan.** B–E implement behavior behind exactly
  these signatures and record layout; they do not renegotiate the surface.
- The three new types are additive; no existing builtin type changes.

## 2. Current State

`term::` is a descriptor-driven builtin package. Every call's return type is a
function of the name alone; the descriptor owns arity/return/type membership and
hand-authored tables own `param_types`/`call_param_names` (`src/builtins/term.rs`,
module doc at `:58`). Two existing type *kinds* are the precedents:

- **Native record types** — `TermColor`, `TermSize` are declared in the Rust
  descriptor `TERM_TYPES` with `TypeKind::Record` + `builtin_type_fields`
  (`src/builtins/term.rs:69`). `MouseEvent` follows this.
- **Source-companion enums** — `LineStyle`, `FillStyle` are declared in the
  injected `.mfb` companion (`src/builtins/term_package.mfb`) as real `ENUM`s,
  with `TypeKind::Enum` (no native fields) in `TERM_TYPES`
  (`src/builtins/term.rs:80`). `MouseKind`, `MouseButton` follow this.

The full seam list for adding a call to an existing native package (verified while
landing `term::didResize` on this branch):

1. Descriptor `src/builtins/term.rs`: the `*_fn` table entry, the
   `param_types`/`call_param_names` groups, and the `#[cfg(test)]`
   `ALL`/`NO_ARG`/return-type tables.
2. Runtime spec `src/target/shared/runtime/term_specs.rs` (`*_SPEC`) + add to
   `src/target/shared/runtime/catalog.rs` SPECS list.
3. Shared code arm: the `match call` in `src/target/shared/code/term.rs`
   (`lower_term_helper`).
4. **Per-target supported-call lists** — `src/target/macos_aarch64/mod.rs`,
   `src/target/linux_common/mod.rs` (all 3 linux arches share it),
   `src/target/win_x86_64/mod.rs`. Missing one → build fails
   `native backend does not support runtime call '<pkg>.<call>'`
   (`src/target/shared/validate/capabilities.rs:21`).
5. `plan.rs runtime_imports` arm **only if** the call makes a libc call. A pure
   state read/write falls through to no imports (like `isOn`/`get*`). `enableMouse`
   in A is a no-op → no imports; `pollMouse` in A returns a constant record via
   arena_alloc (like `terminalSize` builds `TermSize`) → arena helper only, which
   is already available.
6. Man pages (`src/docs/man/builtins/term/*.md`), spec §18
   (`src/docs/spec/language/18_builtin-functions.md:66`), the term backend spec
   (`src/docs/spec/app/04_term-backend.md`).
7. Golden regeneration — see §Validation.

App-mode dispatch: each app backend has its own `emit_app_term_helper`
(`src/target/macos_aarch64/app/app_io.rs`, `src/target/linux_gtk/app_io.rs`,
`src/target/win_x86_64/app/mod.rs`) that returns `None` to delegate a call to the
shared `lower_term_helper`. In A, `enableMouse`/`pollMouse` are **not** added to
any app dispatch → they delegate to the shared no-op stub, uniform on every mode.

### Measured populations

| What | Count | Command |
|---|---|---|
| `term::` functions before this feature | 24 | `grep -c 'term_fn(' src/builtins/term.rs → 24` (23 pre-didResize + didResize) |
| `term::` builtin types before this feature | 4 | `TERM_TYPES` entries in `src/builtins/term.rs:69` (TermColor, TermSize, LineStyle, FillStyle) |
| Per-target supported-call lists to edit | 3 | `grep -rl '"term.terminalSize"' src/target/*/mod.rs → macos_aarch64, linux_common, win_x86_64` |

### Verified properties

- **`MouseEvent` fits the native-record precedent.** Verified `TermColor`/`TermSize`
  are `TypeKind::Record` with `builtin_type_fields` and construct un/qualified
  (`src/builtins/term.rs:69`, and the didResize/terminalSize man examples build).
  Record field types must be builtin scalars; `MouseKind`/`MouseButton` fields are
  enum-typed, matching how records reference companion enums — **VERIFY** a native
  record field may be a source-companion enum type; if not, encode `kind`/`button`
  as `Integer` ordinals and expose the enums only for comparison (fallback noted in
  Open Decisions).
- **No-arg + one-arg `term::` calls both exist** (`isOn` no-arg, `setBold(Boolean)`
  one-arg), so `pollMouse()`/`enableMouse(Boolean)` need no new arity machinery
  (`src/builtins/term.rs:183,186`).

## 3. Design Overview (shared across plan-94 A–E)

The feature has four independent pieces; A builds only the first.

1. **Language surface (A).** Two functions + three types + all seams, no-op stubs.
2. **The unified input decoder / "pump" (B).** One stdin filter in the io read
   path that recognizes complete escape sequences, side-channels events into a
   worker-local queue, and passes non-event bytes through as characters. Fed by
   real tty bytes in CLI; reused verbatim for item 7 (decoded keys).
3. **The event queue (B).** A per-thread (per-arena) fixed-size **timestamped
   overwrite-on-full ring**; `pollMouse` returns the oldest event ≤100ms old.
4. **Per-backend byte injectors (C/D/E).** Each app backend converts a native
   mouse event to cell coords and **injects the SGR bytes into the same worker
   input pipe** it already uses for keystrokes — so the one decoder in (2) serves
   all four modes with no per-backend event queue.

**Where design uncertainty concentrates (schedule first, in B):** the decoder's
placement in the plan-15 broadcast-log read path, and whether the app window-pipe
path is gated by `thread::openStdIn` the same way (see §4.4). B is the cheapest
experiment that falsifies the "one pump, all modes" premise.

**Where correctness risk concentrates (schedule last, C/D/E):** hand-written app
mouse handlers + px→cell conversion. These are compile/assembly-verified only
(no headless window), same limitation as `didResize` app mode.

**Byte-identity is NOT this feature's gate.** Every phase legitimately changes
emitted code and runtime behavior; the gates are rt-behavior tests (CLI) and
compile+assembly inspection (app). Byte-identity/`.ncode` goldens for term
fixtures are **expected to diff** whenever a phase changes `term::` codegen (A adds
the stub bodies; B adds the decoder/enable; C changes the macOS TermView) — a diff
there is the plan working; regenerate and confirm the diff is only the intended
change (the `didResize` work established this exact regen loop).

Rejected alternatives:

- **Per-backend native event queues + per-backend `pollMouse` arms** (instead of
  byte injection). Rejected: triples the hand-written per-backend surface and adds
  a thread-safe queue per backend. Byte injection reuses the existing keystroke
  pipe as the cross-thread channel and needs one decoder. (See §4.3.)
- **Callback/handler API.** Rejected: `term::` has no closure-as-handler idiom; a
  poll model fits the existing draw-loop shape.
- **Dynamic/growable event queue.** Rejected: overwrite-on-full fixed ring gives
  bounded memory and "newest wins" backpressure for free (§4.2).

## 4. Detailed Design

### 4.1 Surface (this sub-plan)

Functions (descriptor `Implementation::Same`, `Lowering::Helper`, like every
`term::` call):

```
term::enableMouse(enabled AS Boolean)   ' returns Nothing
term::pollMouse() AS MouseEvent
```

Types:

```
' native record (descriptor TERM_TYPES, TypeKind::Record + builtin_type_fields)
TYPE MouseEvent { kind, button, row, column, shift, ctrl, alt }

' source-companion enums (src/builtins/term_package.mfb, like LineStyle/FillStyle)
ENUM MouseKind   { None, Down, Up, Move, Drag, ScrollUp, ScrollDown }
ENUM MouseButton { None, Left, Middle, Right }
```

Field encoding of `MouseEvent`: `kind` and `button` are the enum types if a native
record may carry a companion-enum field (see Verified properties VERIFY); else
`Integer` ordinals. `row`/`column` are `Integer` (0-based cell coords, consistent
with `term::moveTo`/`terminalSize`). `shift`/`ctrl`/`alt` are `Boolean`.

A-only lowering (stubs in `lower_term_helper`, `src/target/shared/code/term.rs`):

- `term.enableMouse` → no-op returning `Ok(Nothing)` (mirror the inactive path of
  a setter; ignore the arg).
- `term.pollMouse` → build a `MouseEvent` record with all fields zero
  (`kind = None = ordinal 0`, `button = None = 0`, coords 0, flags false) via
  `arena_alloc`, exactly like `emit_terminal_size` builds `TermSize`
  (`src/target/shared/code/term.rs`), and return `Ok(record)`. The `None`-kind
  zero record is the permanent "no event" sentinel; B replaces the body.

`MouseKind.None = 0` is load-bearing: the zero record from an uninitialized/empty
poll must read as "no event", so `None` MUST be ordinal 0 in the enum declaration.

### 4.2 Event queue (design for B; frozen here so A's record layout matches)

Per-thread (per-arena) fixed-size ring in an arena block; pointer + head/tail in
per-arena term-state slots (same placement family as the `didResize` flag,
`TERM_STATE_*_OFFSET`). Slot = `(MouseEvent fields, u64 monotonic_stamp)`.
Overwrite-on-full (advance tail when head catches it). `pollMouse` returns the
oldest entry with `now − stamp ≤ 100 ms`, advancing tail past staler entries
(prefix-skip; stamps monotonic in enqueue order), else the `None` record. Ring
size 64 (~1–2 KB). Monotonic clock: reuse `emit_read_monotonic_nanos`
(`src/target/shared/code/perf.rs:197`, CLOCK_MONOTONIC Darwin=6/Linux=1) — B
verifies it is usable outside `perf.rs` and on Windows.

### 4.3 Unified decoder / byte injection (design for B/C-E)

See §3(2)(4). CLI: real tty bytes flow through the pump. App: each backend formats
the native event to cell coords and writes SGR bytes
(`\x1b[<b;x;yM` press / `m` release; enabled via `\x1b[?1000h\x1b[?1002h\x1b[?1006h`,
1006 SGR extended coords mandatory) into the worker input pipe it already uses for
keystrokes. **The ring is worker-local — fed only by the pump, never written by the
GUI thread** — so overwrite needs no atomics (the pipe is the cross-thread
boundary). Do not let anyone write the ring cross-thread (that is a lock-free
SPSC-overwrite hazard). Escape valve if the GUI ANSI round-trip grates: a private
internal injected encoding instead of raw ANSI, same pipe, same filter.

### 4.4 Threads / per-arena placement (verified against plan-15; frozen for B–E)

Stdin buffering today (`src/target/shared/code/error_constants.rs:600–697`,
`stdin_broadcast.rs`):

- Global broadcast log `_mfb_rt_stdin_log` — zero-init **non-arena data section**
  holding the mutex/cursors and a 128-entry subscriber registry inlined so no
  registry entry lives in a per-thread arena (`:677`).
- Log byte blocks (8 KiB) — **`malloc`/`free`'d, never per-arena**, explicitly so a
  block read on one thread and freed on another never races an arena free-list
  (`:691`).
- Per-thread 4 KiB local copy buffer `STDIN_LOCAL_BUF` — **lazily arena-allocated**,
  lock-free fast path, pointer+filled+pos in per-arena slots (`:620`).
- App mode has **no broadcast log**: `io::readChar` is a direct per-byte read of
  the window pipe (`src/target/shared/code/io_stdin.rs:228,251`).

Threads opt into stdin via `thread::openStdIn` (main auto-subscribes at entry,
`entry.rs:488`); an unsubscribed stdin read traps `ErrInvalidContext` naming
`thread::openStdIn` (`error_constants.rs:216`). The mouse filter rides the
per-thread reader `_mfb_rt_stdin_next_byte`, so **a thread must `openStdIn` to get
mouse events** — inherited for free; the existing trap enforces it. The mouse-mode
flag is a per-arena term-state slot; parse buffer + ring live in that thread's
arena; a thread with mouse off still sees raw escape bytes, mouse on filters them
— no cross-thread coupling.

- **MUST document (B):** broadcast (per-subscriber) semantics — each
  subscribed+mouse-enabled thread independently decodes the same bytes and gets its
  own copy of the events (not consumed once).
- **MUST verify, not assume (B, explicit task):** app-mode input ownership —
  whether `openStdIn` gates the app window-pipe path the same way (likely
  CLI-broadcast-only; app gated differently or not at all).

## Compatibility / Format Impact

- **Additive only.** Two new call names, three new types, one new per-arena
  term-state flag slot (B), one new arena block per stdin-reading thread (B). No
  existing signature, record layout, term-state offset, or ANSI output changes.
- `term::on`/`off` output is unchanged until mouse is enabled (opt-in).

## Phases

### Phase 1 — Surface + seams + no-op stubs (this sub-plan is one phase)

Delivers the full language surface on every target with inert behavior.

- [ ] Descriptor `src/builtins/term.rs`: add `ENABLE_MOUSE`/`POLL_MOUSE` consts;
      `term_fn` entries (`OV_*` for `(Boolean)→Nothing` and `()→MouseEvent`);
      `MouseEvent` to `TERM_TYPES` (`TypeKind::Record`, fields per §4.1);
      `param_types`/`call_param_names` groups; update `#[cfg(test)]`
      `ALL`/`NO_ARG`/return-type tables.
- [ ] Companion `src/builtins/term_package.mfb`: add `ENUM MouseKind` (None first)
      and `ENUM MouseButton` (None first); add both to `TERM_TYPES` as
      `TypeKind::Enum`. (Line-neutral where possible; expect importer `.ir`/`.ast`
      churn per `.ai/resources-packages.md` — regenerate.)
- [ ] Runtime `term_specs.rs`: `TERM_ENABLE_MOUSE_SPEC` (returns Nothing),
      `TERM_POLL_MOUSE_SPEC` (returns MouseEvent); add both to `catalog.rs` SPECS.
- [ ] Shared `src/target/shared/code/term.rs` `lower_term_helper`: `term.enableMouse`
      no-op arm; `term.pollMouse` arm building the zeroed `MouseEvent` (mirror
      `emit_terminal_size`).
- [ ] Per-target supported lists: add `"term.enableMouse"`, `"term.pollMouse"` to
      `macos_aarch64/mod.rs`, `linux_common/mod.rs`, `win_x86_64/mod.rs`.
- [ ] Man pages `src/docs/man/builtins/term/enableMouse.md`, `pollMouse.md`, and a
      `types.md`/`package.md` update; spec §18 line; term-backend spec note that
      the two calls exist as stubs.
- [ ] Tests: `tests/rt_native_term_runtime.rs` — a `native_term_poll_mouse_is_none_stub`
      case: build+run (piped and pty), assert `pollMouse().kind` prints `None` and
      `enableMouse(TRUE)` emits no escape bytes.
- [ ] Regenerate goldens: `byte-identity/term` ×5 (`scripts/artifact-gate.sh <exe> term`
      → regen the diffs), importer `.ir`/`.ast` for the companion enum growth, and
      any macos-app `.app.ncode`/`.ncodesum` that shift; confirm each diff is only
      the additive surface.

Acceptance: on every target, a program calling `term::enableMouse(TRUE)` +
`term::pollMouse()` builds and runs; the rt test shows `kind = None` and zero
escape bytes; `cargo test --bin mfb` (descriptor/spec pins) and
`scripts/artifact-gate.sh <exe> term` pass.
Commit: —

## Validation Plan

- Tests: `tests/rt_native_term_runtime.rs` stub case (above); descriptor unit
  tests in `src/builtins/term.rs` extended for the new names/types.
- Coverage check: the rt case actually calls both new builtins (in the suite
  denominator), not just references them.
- Runtime proof: `mfb build` + run a 5-line program printing `pollMouse().kind`
  → `None`, under a pty, with no `\x1b` in output.
- Doc sync: man pages + spec §18 + term-backend spec; `python3
  scripts/check-man-examples.py` compiles the new man examples.
- Acceptance: `cargo test --bin mfb`; `scripts/test-accept.sh <exe> /tmp/out '*term*'`;
  `scripts/artifact-gate.sh <exe> term`.

## Open Decisions

- **`MouseEvent.kind`/`button` field type** — companion-enum-typed (cleanest API)
  vs. `Integer` ordinals (guaranteed to work as native record fields). Recommend
  enum-typed pending the VERIFY in §2; fall back to `Integer` if a native record
  cannot hold a companion-enum field. (§4.1)
- **`enableMouse` return** — `Nothing` (recommended, matches setters) vs. a
  `Result` reporting "terminal does not support mouse". Recommend `Nothing`;
  best-effort like every `term::` setter (§4.2.1 gate philosophy).

## Corrections

<Filled in during execution.>

## Summary

A is pure surface + seams with inert bodies — low risk, fully verifiable on every
target, and it freezes the API/record layout so B–E implement behind a fixed
contract. The engineering risk is entirely downstream: the decoder/queue (B) and
the per-backend injectors (C–E).
