# plan-73-B: audio family timeout migration

Last updated: 2026-08-01
Effort: medium (1h–2h)
Depends on: plan-73-A (the convention, shared constants, and canonical spec section must exist)

Migrate `audio::poll` and `audio::read` to the plan-73 timeout convention (defined
in plan-73-A §1). Outcome: audio timeouts read exactly like every other family —
omit = block until ready, `0` = one immediate attempt, `> 0` = wait up to that long
(clamped to `2147483647`), `< 0` = `ErrInvalidArgument`.

References:

- `.ai/compiler.md` (READ FIRST — audio codegen/runtime), `.ai/specifications.md`.
- plan-73-A — the convention and the canonical spec section every man page cites.
- Man: `src/docs/man/builtins/audio/{poll,read}.md`. Spec: `src/docs/spec/stdlib/11_audio.md`.
- Codegen: `src/target/shared/code/audio/{alsa,macos,windows,windows_io}.rs`; specs `src/target/shared/runtime/audio_specs.rs`; descriptor `src/builtins/audio.rs`.

## Prerequisites

See plan-73-A's Prerequisites table (whole-feature gate). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-73-A complete (convention + constants + spec section landed) | `git log --oneline | grep plan-73-A` and `mfb spec language builtin-functions` shows "Timeout convention" | MET — A landed a234b2e87 (in worktree-P-73); `mfb spec language builtin-functions` shows §18.4 |

If plan-73-A is not complete, this sub-plan cannot start, full stop.

> **Correction (B-C1): audio uses distinct internal names, not padded timeouts.**
> Unlike thread/net, `audio::poll`/`audio::read` do NOT pad an omitted timeout —
> the descriptor routes the omit form to a *separate* internal call
> (`audio.poll`/`audio.read`) and the timed form to `audio.pollTimeout`/
> `audio.readTimeout` (`src/builtins/audio.rs` `internal_name`, verified via `lower_audio_*`
> dispatchers). So "omit = block" (Phase 2) is implemented by changing what the
> untimed `audio.poll` codegen DOES (immediate→block), not by swapping a padded
> value. `TIMEOUT_UNBOUNDED_SENTINEL` is therefore not used by audio.

## 1. Goal

- `audio::poll(stream[, timeoutMs]) AS Boolean` (readiness query): omit = block
  until the stream is ready; `0` = immediate check returning `FALSE` when not
  ready; `> 0` = wait up to that long; `< 0` = `ErrInvalidArgument` (today
  **not range-checked** — this is the flip).
- `audio::read(input, frames[, timeoutMs]) AS List OF Byte` (readiness/partial):
  omit = block until `frames` captured; `0` = return whole frames already buffered
  without blocking; `> 0` = return whole frames captured by the deadline; `< 0` =
  `ErrInvalidArgument`. The `86400000` (24 h) upper bound that currently raises
  `ErrInvalidArgument` is replaced by the convention's clamp to `2147483647`.
- Every audio fixture/example and the two man pages + audio spec match the new
  semantics; `cargo test` and `scripts/artifact-gate.sh` green.

### Non-goals

- No arity/name changes; no new audio functions. `audio::read`'s partial-frame
  return value semantics are unchanged except for negative rejection and the cap.

## 2. Current State

From the audit (man pages read):

- `audio::poll(stream[, timeoutMs])`: `0` = non-blocking test; positive = wait up
  to; **`timeoutMs` is not range-checked** — a negative value is passed through to
  the backend rather than rejected (`src/docs/man/builtins/audio/poll.md:42`,
  `:76`). Omit = immediate (one-arg form "tests readiness immediately, never
  blocks").
- `audio::read(input, frames[, timeoutMs])`: two-arg form blocks until `frames`
  captured; three-arg returns early on `timeoutMs` with only whole frames; `0`
  polls (returns buffered whole frames, no block); **upper bound `86400000`, a
  larger value raises `ErrInvalidArgument`** (`src/docs/man/builtins/audio/read.md:39`).
  Negative not addressed.

Backends implementing the wait: `alsa.rs` (Linux), `macos.rs`, `windows.rs` +
`windows_io.rs` — each has a `TIMEOUT_MAX = 86400000` constant and a poll/read
wait path (`grep -n 86400000 src/target/shared/code/audio/*.rs`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `audio::poll` call lines (tests+examples) | 4 | `grep -rn --include='*.mfb' -F 'audio::poll' tests examples | wc -l` |
| `audio::read` call lines | 4 | `grep -rn --include='*.mfb' -F 'audio::read' tests examples | wc -l` |
| audio backends with the wait/cap | 3 (alsa/macos/windows) | `ls src/target/shared/code/audio/{alsa,macos,windows}.rs` |

### Verified properties

- The three backends share the `86400000` cap constant and independently implement
  the wait — VERIFIED by the codegen census (`grep -n 86400000 …`). Each must get
  the negative-reject + clamp change; RE-READ each before editing.
- `audio::poll` two-arg lowers to a real wait, one-arg to an immediate check —
  VERIFIED: `src/docs/man/builtins/audio/poll.md:39-42`, `:76`. Adding an unbounded
  (omit) path means the one-arg form must route to a block, not the immediate check.

## 3. Design Overview

Two small, mechanical changes across three backends plus the descriptor:

1. **Negative → `ErrInvalidArgument`** in the shared entry (prefer a single check
   in `src/builtins/audio.rs` resolve/validation or the shared audio helper prologue,
   so all three backends inherit it) — mirrors `net::poll`'s existing negative
   check rather than each backend range-checking.
2. **Cap: clamp to `2147483647` instead of raising at `86400000`.** Replace the
   `TIMEOUT_MAX`-raises path with the same INT_MAX clamp `net::poll` uses.
3. **Omit = block:** change `audio::poll`/`audio::read` omitted-timeout padding in
   `src/builtins/audio.rs` from the immediate/`0` default to
   `TIMEOUT_UNBOUNDED_SENTINEL` (from plan-73-A), and route the sentinel to the
   block path in each backend. For `audio::read`, "omit = block until frames" is
   already the two-arg behavior, so this is largely padding alignment; confirm the
   sentinel and a literal large value take the same block path.

**Correctness risk:** low–medium — three backends but the change is a negative
guard + a clamp + a padding swap, no new wait machinery. Audio has no CI runtime
device, so proof is codegen (`artifact-gate` `.ncodesum`) + fixtures that don't
require a live device where possible.

**Rejected alternative:** keep the 24 h cap as an audio-specific bound. Rejected —
it is exactly the kind of per-family special case plan-73 exists to remove.

## Compatibility / Format Impact

- **Behavioral, intentional:** negative `timeoutMs` now rejected (was passed
  through) for `audio::poll`; `> 86400000` now clamped (was `ErrInvalidArgument`);
  `audio::poll(stream)` with no arg now blocks (was immediate). `audio::read`
  no-timeout still blocks until `frames`.
- **Unchanged:** the partial-whole-frame return value of `audio::read`, arities,
  names, `audio::poll` returning `Boolean`.

## Phases

> Keep checkboxes current in-commit; fill `Commit:`; unticked = NOT DONE.

### Phase 1 — Negative-reject + clamp

- [x] Add a negative-`timeoutMs` → `ErrInvalidArgument` check for `audio::poll`
      (`pollTimeout`) and `audio::read` (`readTimeout`), mirroring `net::poll`.
      — DONE per-backend (not a single shared entry: each backend's timed path owns
      its own prologue; the descriptor has no shared runtime check site — see B-C1).
      alsa `lower_read`+`lower_query`(PollTimeout), macos `lower_read`+`lower_query`,
      windows_io `lower_read`+`lower_query`. Added an `invalid`→`ErrInvalidArgument`
      path to each `lower_query` (guarded to `PollTimeout` so other queries stay
      byte-identical).
- [x] Replace the `86400000`-raises path with a clamp to `2147483647` in
      `alsa.rs`, `macos.rs`, `windows.rs`/`windows_io.rs`. — DONE: renamed
      `TIMEOUT_MAX`(86400000) → `TIMEOUT_CLAMP_MS`("2147483647") in each backend;
      the timed read/poll now clamps (compare/branch_le/move) and stores the clamped
      value back before the wait, instead of raising.
- [x] Tests: codegen golden proving the reject+clamp. — DONE: regenerated
      `byte-identity/audio` `.ncodesum` for all 5 targets (the fixture covers
      `read(_, _, N)` and `poll(_, N)`). NOTE — no CI audio device, so the
      negative-reject/clamp are proven by codegen byte-identity, NOT runtime
      (device-free limitation, per the plan's Validation section). A unit
      "validation" test is not applicable: the check is emitted runtime code, not a
      compile-time diagnostic.

Acceptance: `artifact-gate` `.ncodesum` regenerated and diffs=0 (1501 goldens, 0
diffs); `cargo test` green. — MET.
Commit: —

### Phase 2 — Omit = block + docs + fixtures

- [ ] `src/builtins/audio.rs`: pad omitted `timeoutMs` with `TIMEOUT_UNBOUNDED_SENTINEL`;
      route the sentinel to the block path in each backend (verify one-arg
      `audio::poll` now blocks; `audio::read` two-arg unchanged).
- [ ] Migrate the 4 + 4 fixtures/examples: any that relied on `audio::poll(stream)`
      being immediate now pass `, 0`; regenerate goldens (`scripts/sync-goldens.sh`).
- [ ] Rewrite `src/docs/man/builtins/audio/{poll,read}.md` to the convention (each
      citing plan-73-A's section; run `scripts/update_man.sh`) and update
      `src/docs/spec/stdlib/11_audio.md`.

Acceptance: fixtures pass with new semantics; man/spec cite the canonical section;
man_citations + spec-citation tests green; `cargo test` + `artifact-gate` green.
Commit: —

## Validation Plan

- Tests: audio validation/unit tests for negative + clamp; fixtures for the
  omit=block and `0`=immediate paths (device-free where possible).
- Coverage check: the negative and clamp branches are in the suite denominator.
- Runtime proof: where a device is unavailable in CI, rely on `.ncodesum` codegen
  proof; note this limitation explicitly (no silent gap).
- Doc sync: two audio man pages + `stdlib/11_audio.md` + citations.
- Acceptance: `cargo test`, `scripts/artifact-gate.sh` diffs=0.

## Open Decisions

- **Single negative check site** — shared audio prologue vs. `src/builtins/audio.rs`
  resolve. Recommended: descriptor-level so all three backends inherit it without
  triplicated codegen. (§3)

## Corrections

<Filled during execution.>

## Summary

Smallest family: a negative guard, a clamp, and a padding flip across three
backends, proven by codegen goldens (no CI audio device). No new wait machinery;
risk stays with net (C) and tls (D).
