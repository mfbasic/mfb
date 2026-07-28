# plan-69: `term::` keyboard input — `getKeyDown` / `getKeyUp` and the `Key` enum

Last updated: 2026-07-27
Effort: large (multi-session; 3 landable phases)
Depends on: the existing `term::on()`/`off()` raw-mode lifecycle
(`src/target/shared/code/term.rs:380-498`) and the `io::` stdin read path
(`src/builtins/io.rs`, `src/target/shared/code/io_stdin.rs`).
Produces: `term::getKeyDown(timeout) AS term::Key`, `term::getKeyUp(timeout) AS
term::Key`, and the `term::Key` enum, across the console and macOS-app backends.

The single behavioral outcome: an MFBASIC program can read structured key **press**
and **release** events (arrows, letters, digits, symbols, F-keys, modifiers) with a
blocking-or-timed wait, while every existing `io::input` / `io::read*` call keeps
returning byte-for-byte what it returns today whether or not TUI mode is on.

References (read first):

- `src/target/shared/code/app.rs:84-97` — `emit_get_mode`: the exact "return an enum
  by its i64 ordinal in `RESULT_VALUE_REGISTER`" template. `getKeyDown`/`getKeyUp`
  copy this return shape.
- `src/target/shared/code/io_stdin.rs:3-100` — `lower_io_poll_input_helper`: the
  `poll(2)`-with-timeout primitive (`pollfd` on fd 0, `POLLIN`) the console reads reuse.
- `src/target/shared/code/term.rs:380-498` — `term::on`/`off`: the cbreak/termios
  flip that Kitty enable/disable hooks into (Phase 2).
- `src/target/macos_aarch64/app/term_view.rs:2239-2442` — `emit_term_key_down_helper`:
  the only `keyDown:` IMP today; Phase 3 adds its `keyUp:` sibling.
- `src/target/shared/runtime/{io_specs,term_specs,catalog}.rs` — the runtime-spec +
  catalog registration a new returning builtin needs.
- The Kitty keyboard protocol progressive-enhancement flags (external): flag `0b1`
  disambiguate, `0b10` report-event-types (press/repeat/**release**), `0b1000`
  report-all-keys-as-escape-codes.

## 1. Goal

- `term::Key` enum (declared in `src/builtins/term_package.mfb`, like `LineStyle`/
  `FillStyle`), ordinal `0` = `None` (the timeout / no-key sentinel). Confirmed layout:

  ```
  None                                             ' 0
  A B C D E F G H I J K L M N O P Q R S T U V W X Y Z   ' 1..26
  Num0 Num1 Num2 Num3 Num4 Num5 Num6 Num7 Num8 Num9     ' 27..36
  Space Enter Tab Backspace Escape Delete Insert
  UpArrow DownArrow LeftArrow RightArrow
  Home End PageUp PageDown
  Shift Ctrl Alt Meta                              ' modifiers are their own keys
  F1 F2 F3 F4 F5 F6 F7 F8 F9 F10 F11 F12
  Minus Equal LeftBracket RightBracket Backslash
  Semicolon Quote Backtick Comma Period Slash
  ```
  `Key.A` is returned for both `'a'` and `'A'` (case/shift arrives as a separate
  `Key.Shift` event); `'-'` and `'_'` both map to `Key.Minus`.

- `term::getKeyDown(timeout AS Integer) AS term::Key` — wait up to `timeout` ms for the
  next key **press**; `timeout = 0` blocks forever; on timeout return `Key.None`.
- `term::getKeyUp(timeout AS Integer) AS term::Key` — same, for key **release**.
- Console backend: real release events via the Kitty keyboard protocol, enabled while
  `term::on()` and disabled on `term::off()` **and at process shutdown**. Terminals
  that do not support Kitty: `getKeyDown` still works (legacy decode), `getKeyUp`
  returns `Key.None`.
- macOS app backend: real `keyDown:` / `keyUp:` NSEvents; identical surface to the CLI.
- **Hard invariant:** `io::input`, `io::readLine`, `io::readChar`, `io::readByte`,
  `io::pollInput` return exactly what they return today, Kitty on or off.

### Non-goals (explicit constraints)

- **No new `io::` surface and no `io::` behavior change.** `io::read*` are protected by
  a legacy re-encoder, not modified in contract.
- **No callback delivery.** Keys are polled (consistent with plan-13-G §2.4: there is
  no native→MFBASIC callback mechanism in the tree).
- **Linux GTK app backend is deferred** (the `term::` GTK backend is itself still
  deferred — see plan-62). Phase 3 is macOS-app only; GTK tracked as an open decision.
- **Windows console Kitty support is an open decision** (Phase 2) — Windows console
  input is `ReadConsoleInput`, not a byte stream; may land as legacy-only first.
- No key-repeat exposure in v1 (Kitty repeat events are folded into press or dropped;
  decided in Phase 2).

## 2. Current State

### 2.1 Measured populations

| What | Count / value | Command |
|---|---|---|
| `term::` input functions today | **0** | `rg -n 'READ|INPUT|KEY' src/builtins/term.rs` |
| Builtins that already return an enum by ordinal | **≥2** (`app::getMode`→`Mode`, `datetime::weekday`→`Weekday`) | `rg -n 'returns: "Mode"|returns: "Weekday"' src/target/shared/runtime/` |
| `keyUp` / `NSEventTypeKeyUp` handlers in `src/` | **0** | `rg -n 'keyUp|NSEventTypeKeyUp' src/` |
| Timeout-taking input precedents | **1** (`io::pollInput`) | `src/target/shared/code/io_stdin.rs:3` |

### 2.2 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| An enum returns as a bare i64 ordinal in `RESULT_VALUE_REGISTER`, scalar path, no arena copy | **CONFIRMED** | `app::getMode` end-to-end (`app.rs:84-97`); `result_payload_is_block` treats a non-record/non-collection name as scalar (`builder_value_semantics.rs:918-925`) |
| `term::` has no input today; `io::` owns all reads; `term::on` only flips raw mode | **CONFIRMED** | exploration map, this session |
| Kitty release events for text keys require report-all-keys-as-escape-codes (`0b1000`), which reroutes plain `'a'` | **CONFIRMED (protocol)** | Kitty spec; this is why the re-encoder exists |
| The macOS app delivers keys through a single per-NSApp pipe carrying keyDown UTF-8 only | **CONFIRMED** | `app/mod.rs` PIPE_ASSOC_KEY; `term_view.rs:2239-2442` |
| `io::read*` stay byte-identical under Kitty | **UNVERIFIED — the Phase-2 acceptance criterion** | proven by a golden `io::readChar`/`readLine` run with TUI mode on, not by reasoning |

## 3. Design Overview

**Return shape (all phases).** `getKeyDown`/`getKeyUp` copy `emit_get_mode`: compute the
`Key` ordinal, `move_immediate(RESULT_VALUE_REGISTER, "EnumOrdinal"/Integer, n)`, tag OK.
Front-end `call_return_type_name` → `"Key"`; `param_types` → `["Integer"]`; runtime specs
`returns: "Key"`; `is_builtin_type("Key")` true.

**Console input, one decoder two consumers (Phase 2).** Raw terminal bytes feed a single
stateful decoder producing structured events `{key_ordinal, is_release}`. Two consumers
draw from it:
- `io::read*` — a **legacy re-encoder** turns each text-producing **press** event back
  into the exact byte(s) a non-Kitty terminal would have sent (`'a'`→`0x61`, arrow-up
  →`\x1b[A`), skips releases and bare-modifier events, and feeds the existing
  `io_stdin.rs` byte path unchanged. This is what keeps `io::read*` byte-identical.
- `getKeyDown`/`getKeyUp` — consume the structured events directly, filtering by
  `is_release`, mapping to a `Key` ordinal.

**Phase-1 simplification (no Kitty yet).** Before Kitty is enabled, the "decoder" is the
legacy escape-sequence parser alone: it maps a single UTF-8 char or a legacy CSI/SS3
sequence (`\x1b[A`… arrows, `\x1b[H` home, `\x1bOP` F1…) to a `Key` ordinal for
`getKeyDown`. No release events exist without Kitty, so `getKeyUp` is **not exposed in
Phase 1** — it ships in Phase 2 so nothing is a stub. `getKeyDown` reads the same byte
stream `io::readChar` reads today, so Phase 1 touches no `io::` code at all.

**Kitty lifecycle (Phase 2).** `term::on()` emits the enable sequence
(`\x1b[>{flags}u`, flags = disambiguate | report-event-types | report-all-keys); `term::off()`
and the normal-exit path emit the pop (`\x1b[<u`). Enable is gated on `io::isInputTerminal`
so a piped stdin is untouched. A terminal that ignores the sequence simply never emits
release events → `getKeyUp` returns `Key.None` (the accepted fallback).

**App input (Phase 3).** Register a `keyUp:` IMP beside the existing `keyDown:` on the
synthesized TermView; both push `{key_ordinal, is_release, is_modifier}` records onto a
new down/up event channel (a second pipe or a tagged framing on the existing one).
`getKeyDown`/`getKeyUp` drain that channel. Bare modifier presses (`flagsChanged:`) become
`Key.Shift`/`Ctrl`/`Alt`/`Meta` events. Grid-touching stays main-thread-marshalled as
today; the key channel is read off the pinned pipe fd.

**Where correctness risk concentrates:** the `io::read*` byte-identity invariant under
Kitty. The re-encoder is the fix and its proof is a golden `io::` read run with TUI mode
**on**, diffed against the same program with TUI mode off — not reasoning about the
protocol.

**Where design uncertainty concentrates:** the single-enum-per-event model vs. modifier
state. A terminal reports characters, not chords; `Key.A` for `'A'` deliberately drops the
shift bit into a *separate* `Key.Shift` event (only observable under Kitty/app). On a
legacy console you observe only the resulting character. This is inherent to a one-enum
return and is documented, not worked around.

**Rejected alternative:** *enable Kitty flag `0b1000` and let `io::readChar` see the
escape sequences.* Rejected — it breaks the hard `io::read*` byte-identity invariant.

**Rejected alternative:** *a parallel second input fd for getKey\* on console.* Rejected —
one terminal, one byte stream; two readers draining the same fd race. One decoder, two
logical consumers.

## Compatibility / Format Impact

- **New:** `term::Key` enum; `term::getKeyDown`/`getKeyUp`; two runtime specs; a console
  input decoder + legacy re-encoder; a Kitty enable/disable in `term::on`/`off`; a macOS
  `keyUp:` IMP + key event channel.
- **Unchanged:** `io::` surface and byte-level behavior; `term::` output/draw surface;
  the `.mfp` format; the enum-ordinal ABI (reused, not extended).

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**

### Phase 1 — `Key` enum, surface, and `getKeyDown` (legacy decode, no Kitty)

- [ ] `term::Key` enum in `term_package.mfb` (full confirmed layout, `DOC` block).
- [ ] Front-end `src/builtins/term.rs`: `GET_KEY_DOWN` name, `is_term_call`,
      `call_param_names`/`param_types` (`["Integer"]`), `call_return_type_name` (`"Key"`),
      `arity` (1,1), `is_builtin_type("Key")`, unit tests.
- [ ] Runtime spec `TERM_GET_KEY_DOWN_SPEC { returns: "Key" }` + `catalog.rs` registration.
- [ ] Console codegen: poll(timeout) then read one key event; legacy escape-sequence
      parser → `Key` ordinal; return via the `emit_get_mode` shape; `Key.None` on timeout.
- [ ] macOS app codegen: `getKeyDown` drains the existing keyDown pipe → `Key` ordinal.
- [ ] Man pages `getKeyDown.md`, `Key` in `types.md`, `package.md`; spec update.
- [ ] Fixtures: `func_term_getKeyDown_valid` / `_invalid`, goldens.

Acceptance: `getKeyDown` compiles, types as `term::Key`, arrows/letters/digits decode on a
real terminal; `cargo test` + artifact-gate green; full acceptance green. Commit: —

### Phase 2 — Kitty protocol, `io::` re-encoder, and console `getKeyUp`

- [ ] Kitty enable in `term::on` (gated on `isInputTerminal`), pop in `term::off` **and**
      the normal-exit path.
- [ ] Unified console decoder (legacy + Kitty CSI-u), structured `{ordinal, is_release}`.
- [ ] Legacy re-encoder feeding `io::read*` → **byte-identical** goldens with TUI on.
- [ ] `term::getKeyUp` surface + spec + catalog + console body (release events; `Key.None`
      fallback on non-Kitty).
- [ ] Modifier keys (`Key.Shift`/`Ctrl`/`Alt`/`Meta`) from Kitty modifier reports.
- [ ] Man page `getKeyUp.md`; spec update; fixtures + goldens.

Acceptance: a golden `io::readChar`/`readLine`/`input` program run with `term::on()` is
byte-identical to the TUI-off run; `getKeyUp` returns real releases under a Kitty terminal
and `Key.None` otherwise. Commit: —

### Phase 3 — macOS app `keyUp:` + modifiers

- [ ] `keyUp:` IMP on the synthesized TermView; register the selector (`bootstrap.rs`).
- [ ] A down/up key event channel; `getKeyDown`/`getKeyUp` drain it (not the line pipe).
- [ ] `flagsChanged:` → bare `Key.Shift`/`Ctrl`/`Alt`/`Meta` events.
- [ ] Fixtures/goldens where golden-testable; on-device proof for the live behavior.

Acceptance: on-device macOS app, a poll loop distinguishes press from release and sees bare
modifier events; the CLI surface is unchanged. Commit: —

## Validation Plan

- Tests: syntax + rt-behavior fixtures for each function's arity/type/return; `cargo test`
  (full, never one module); artifact-gate (`scripts/artifact-gate.sh`) for codegen goldens.
- The **critical** gate: Phase-2 `io::` byte-identity goldens (TUI-on vs TUI-off).
- Runtime proof: real-terminal decode (Phase 1/2) and on-device macOS app (Phase 3) — the
  live key behavior is not golden-testable and that is stated, not papered over.
- Doc sync: `src/docs/man/builtins/term/*`, `src/docs/spec/app/04_term-backend.md`, and the
  `man_citations_resolve` / `spec_citations_resolve` tests.
- `.mfp`: unaffected (no package-format change); `scripts/sync-package-mfp.sh` not needed.

## Open Decisions

1. **Windows console Kitty support.** Windows Terminal speaks the Kitty protocol but the
   Windows console read path is `ReadConsoleInput`, not a byte stream. Recommended: Phase 1
   legacy `getKeyDown` on Windows; defer Windows `getKeyUp`/Kitty to a follow-up, documented.
2. **Linux GTK app backend.** Deferred with the rest of the GTK `term::` backend; when it
   lands it reuses the Phase-3 event-channel shape.
3. **Key-repeat.** Recommended: fold Kitty repeat events into a press for v1; no `Key`
   repeat variant.
4. **`flagsChanged:` on console.** Bare modifier presses are unobservable on a legacy
   console and only partially observable under Kitty; documented as app-first.

## Corrections

<!-- Filled in during execution. -->

## Summary

The engineering risk is the `io::read*` byte-identity invariant: Kitty's release events for
text keys require a flag that reroutes plain `'a'` into an escape sequence, so a legacy
re-encoder must reconstruct the original bytes for the `io::` path while `getKey*` read the
structured events. Phase 1 sidesteps all of it — `getKeyDown` on the legacy stream is a
complete, low-risk feature that touches no `io::` code — and Phase 2 introduces Kitty behind
the re-encoder, proven by a byte-identical `io::` golden run rather than by argument. Phase 3
adds real press/release to the macOS app via a `keyUp:` sibling of the existing handler.
