# bug-53: macOS app-mode transcript output leaks an owned `NSAttributedString` (and an owned `NSString`) on every `io::print`/`io::write` — memory grows unbounded with output volume

Last updated: 2026-07-09
Effort: small (<1h)

In macOS AppKit app mode, every transcript write allocates owned Cocoa objects that are
never released. `emit_app_io_write_helper` builds `[[NSString alloc]
initWithBytes:length:encoding:]` (owned, retain count +1) and hands it to
`_mfb_macapp_append`, which builds `[[NSAttributedString alloc]
initWithString:attributes:]` (owned, +1) and calls `[textStorage
appendAttributedString:]`. `appendAttributedString:` **copies** the argument's
contents; it does not take ownership. Neither the `NSString` nor the
`NSAttributedString` is ever `release`d — a whole-directory grep of
`src/target/macos_aarch64/app/` finds **no** `release`/`autorelease` selector send at
all, only a single `objc_autoreleasePoolPush` that is never popped and would not cover
`alloc`/`init` objects anyway. So both objects leak on every write, and a program that
prints in a loop grows memory without bound until OOM.

The single correct behavior a fix produces: each transcript write releases the Cocoa
objects it allocates, so a long-running or output-heavy app has flat memory.

References:

- `src/target/macos_aarch64/app/bootstrap.rs:emit_append_helper` (`:585-605`+): allocs
  `NSAttributedString` via `alloc`/`initWithString:attributes:`, calls
  `appendAttributedString:`, never releases it.
- `src/target/macos_aarch64/app/app_io.rs:emit_app_io_write_helper` (`:69-80` and
  `:123-134`): allocs `NSString` via `alloc`/`initWithBytes:length:encoding:`, hands it
  to `_mfb_macapp_append` / `mfbWriteString:` (both copy), never releases it.
- No release anywhere: `grep -n "release\|autorelease" src/target/macos_aarch64/app/*.rs`
  returns only the `objc_autoreleasePoolPush` at `bootstrap.rs:520` (pushed on the
  worker thread, never popped).
- Contrast: keyDown handlers use autoreleased `stringWithUTF8String:`
  (`build_nsstring_from_cstring`) and run on the main thread whose per-event autorelease
  pool drains — so they do not leak. The leak is specific to the owned `alloc`/`init`
  objects created on the worker thread's write path.
- Found during the goal-01 compiler source review of `src/target/macos_aarch64/app/`.

## Failing Reproduction

```
IMPORT io
SUB main()
  MUT i AS Integer = 0
  WHILE i < 10000000
    io::print("line " & toString(i))
    i = i + 1
  WEND
END SUB
```

Build as a macOS `.app` (`mfb build -app`) and run it.

- Observed: RSS climbs steadily; Instruments/`leaks` reports two leaked objects
  (`NSAttributedString` + `NSString`) per `io::print`.
- Expected: memory stays flat; the objects are released after each append.

Contrast (no leak today): headless (non-app) mode writes via `write()`; the app-mode
keyDown path uses autoreleased strings drained by the main-thread pool.

## Root Cause

The write/append helpers use owned constructors (`alloc` + `initWith…`, retain count 1)
but emit no matching `release`. `appendAttributedString:`/`mfbWriteString:` copy their
argument rather than retaining it, so after the call the caller holds the sole
reference and must release it. The worker thread's autorelease pool is pushed once and
never drained, and in any case does not manage `alloc`/`init` objects (which are not
autoreleased). Every write thus adds two permanently-referenced objects.

## Goal

- Each `io::print`/`io::write`/`io::printError` in app mode releases the `NSString` and
  `NSAttributedString` it creates.
- An output-in-a-loop app shows flat memory under `leaks`.

### Non-goals (must NOT change)

- The transcript's visible content (the copy into `textStorage` is correct).
- The keyDown/autoreleased-string paths.
- Headless mode.

## Blast Radius

- `emit_append_helper` (`bootstrap.rs`) — release the `NSAttributedString` after
  `appendAttributedString:`.
- `emit_app_io_write_helper` (`app_io.rs`, both the transcript and TUI-surface arms) —
  release the `NSString` after it is consumed by append/`mfbWriteString:`.
- The `attributes` dictionary built at `emit_append_helper` (via
  `dictionaryWithObject:forKey:` — autoreleased) — verify it is autoreleased (no leak)
  vs owned (also needs release).
- keyDown handlers — unaffected.

## Fix Design

After the consuming call in each helper, emit a `release` selector send on the owned
object (spilling any live pointer across the msgSend). Alternatively switch to
autoreleased constructors **and** ensure a draining pool on the worker thread — but the
worker pool is currently push-once/never-pop, so explicit `release` is the simpler,
self-contained fix. Confirm the `attributes` dictionary is autoreleased before assuming
it needs no release.

## Phases

### Phase 1 — failing test

- [x] Add a macOS app-mode output-loop test under `leaks`/Instruments (or a codegen
      assertion that each write helper emits a `release`). Confirm the two leaks today.

### Phase 2 — the fix

- [x] Emit `release` for the `NSAttributedString` in `emit_append_helper` and the
      `NSString` in `emit_app_io_write_helper` (both arms).

### Phase 3 — validation

- [x] Regenerate macOS app goldens (delta = release sends on the write paths).
- [x] `scripts/test-accept.sh`; run the loop under `leaks` on macOS — zero growth.

## Validation Plan

- Regression test(s): the output-loop leak test.
- Runtime proof: `leaks`/Instruments flat across millions of prints.
- Doc sync: none expected.
- Full suite: `scripts/test-accept.sh`.

## Summary

Owned `NSString`/`NSAttributedString` objects created on every app-mode write are never
released (there is no `release` send anywhere in the app backend), so output-heavy apps
leak two objects per print. The fix adds a `release` on each write path; visible output
and the keyDown paths are untouched.

## Resolution

Fixed by sending `-release` to each owned Cocoa object on the worker-thread write
paths once its consuming call has copied it. Three release sends added, one selector
constant, one codegen regression test.

Files changed:

- `src/target/macos_aarch64/app/mod.rs` — new `SEL_RELEASE` selector constant
  (`"release"`), registered in `app_mode_data_objects()` so the selector C-string is
  emitted; a `#[cfg(test)] mod bug53_release_tests` asserting the release sends.
- `src/target/macos_aarch64/app/bootstrap.rs` (`emit_append_helper`) — `-release` the
  owned `NSAttributedString` after `appendAttributedString:`.
- `src/target/macos_aarch64/app/app_io.rs` (`emit_app_io_write_helper`) — `-release`
  the owned `NSString` in both the GUI transcript arm and the TUI-surface arm.

Ownership argument for each release (only `alloc`/`copy`/`new`/`mutableCopy` results
are owned; everything else is autoreleased and must NOT be released):

- Append helper's `attr` comes from `[[NSAttributedString alloc]
  initWithString:attributes:]` → owned, +1. `appendAttributedString:` copies its
  contents, so we hold the sole reference → release. (`font` from
  `userFixedPitchFontOfSize:` and `attrs` from `dictionaryWithObject:forKey:` are
  autoreleased — left untouched.)
- Both write arms' `text` comes from `[[NSString alloc]
  initWithBytes:length:encoding:]` → owned, +1. The GUI arm's `_mfb_macapp_append`
  copies it into the text storage; the TUI arm's `mfbWriteString:` only reads its
  glyphs via `characterAtIndex:` synchronously (`waitUntilDone:YES`) and does not
  retain it. Either way we hold the sole reference → release.

Register-lifetime safety (`.ai/compiler.md`): each released pointer is held in a
callee-saved register that survives its consuming `bl` before the release msgSend —
append's `attr` in `x20` (survives `appendAttributedString:`), the GUI arm's `text`
saved to `x22` (preserved by `_mfb_macapp_append`, which saves x19–x22), the TUI arm's
`text` in `x21` (survives the `performSelectorOnMainThread:` msgSend). No live value is
kept in a caller-saved register across any `bl`.

Runtime proof (macOS aarch64, GUI window, 60,000 `io::print` calls, `leaks` tool):

- Before: `419,134 leaks for 17,234,928 total leaked bytes` — a leaked
  `NSConcreteAttributedString` + `NSString` per print (scales with output volume).
- After: `34 leaks for 2,608 total leaked bytes` — constant one-time bootstrap
  allocations that do NOT scale with print volume; the per-print leak is gone.
- Peak RSS over the run fell 228 MB → 217 MB; the remaining growth is the
  `NSTextView`/`NSTextStorage` transcript legitimately retaining all printed text
  (by design), not a leak.

Codegen regression test (`bug53_release_tests`, `cargo test --bin mfb bug53_release`):
asserts the append helper emits one release send, the write helper emits two (both
arms) / one (no-term), and the `release` selector string is emitted. Verified
fail-before (count 0) / pass-after.

Goldens shifted (release sends + the new selector data object) — regenerate:
`tests/syntax/app/macos-app-mode-io`, `tests/syntax/app/macos-app-mode-plumbing`,
`tests/syntax/app/macos-app-mode-term` (`.ncode`/`.nir`/`.nplan`/`build.log`). Linux
GTK app mode and all other targets are unaffected.
