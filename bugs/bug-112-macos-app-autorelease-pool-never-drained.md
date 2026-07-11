# bug-112 — macOS app worker's autorelease pool pushed once, never drained → unbounded leak in GUI mode

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G7).
**Severity:** MED — RSS grows without bound proportional to output volume in
app-mode GUI programs.
**Class:** memory-safety (leak).

## Finding

`src/target/macos_aarch64/app/bootstrap.rs:514-530` (`emit_worker_shim` pushes
`objc_autoreleasePoolPush`) and :736-743 (`emit_finish_helper` parks in
`pause()` forever).

The worker pushes an autorelease pool at start; the pool is **never popped**
(no `objc_autoreleasePoolPop` anywhere in src/target — verified by grep) and
the worker deliberately never exits (parks in `pause()`). Every GUI
`io::print`/`io::write` autoreleases at least an attributes NSDictionary
(bootstrap.rs:582-592 `dictionaryWithObject:` per append,
`userFixedPitchFontOfSize:` per append) and the newline NSString
(`stringWithUTF8String:` in app_io.rs) into that pool, so pool entries and the
autoreleased objects accumulate for the process lifetime.

bug-53 fixed the *owned* (alloc/init) objects but not the autoreleased ones.

## Trigger

macOS app-mode GUI program printing in a loop (`DO io::print("x") LOOP`): RSS
grows without bound; Instruments shows NSDictionary/pool-page growth.

## Fix sketch

Wrap each append/write marshaled to the main thread in its own
push/pop pair (pop immediately after the `performSelectorOnMainThread`
completes), or drain periodically: push a fresh pool and pop the old one every
N appends. The one-pool-forever pattern can't work for a non-exiting worker.
