# bug-117 — G7 platform LOW cluster: riscv armed-but-dead GTK hooks, aarch64 flush dead imports, GTK term grid race, macOS headless busy-spin, stale comments

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G7). Independent LOW /
latent / docs findings across the platform targets, batched per goal-02.

## 1. riscv64 backend wires GTK app hooks that emit unported aarch64-convention code (dead but armed)

`src/target/linux_riscv64/code.rs:106-231` (`emit_program_exit` MACAPP arm,
`emit_app_program_entry` → `gtk::emit_app_program_entry`, all
`emit_app_io_*`/`emit_app_term_helper` hooks). `linux_riscv64/mod.rs:180-185`
declares `supports_app_mode() = false` ("GTK4 app-mode toolkit not ported to
rv64"), yet the riscv Platform delegates every app-mode hook to
`target::linux_gtk` — the plain aarch64-register variants, without the x86-style
`wrap_x86_helper`/per-ISA trampoline the x86 backend needed. Unreachable today
(CLI gates -app); latent trap for a future rv64 app port. Fix: leave the hooks
`unimplemented!`/panic until actually ported, so a flip of `supports_app_mode`
can't silently emit unaudited register conventions.

## 2. linux_aarch64 io.flush still declares dead fsync/__errno_location imports (bug-71 residual)

`src/target/linux_aarch64/plan.rs:108-111` (`runtime_imports`, "io.flush" arm).
`lower_io_flush_helper` (io_helpers.rs:444+) is drain-only since plan-14-A and
never calls fsync or reads errno. bug-71 removed these dead imports from
linux_x86_64/linux_riscv64/macos_aarch64 (each got a guard test) but explicitly
excluded linux_aarch64, which still declares both dynamic symbols for every
program using io.flush. Fix: drop the fsync + __errno_location imports from the
aarch64 io.flush arm and add the matching guard test.

## 3. GTK term grid mutated from worker while main thread draws (benign race) + stale doc

`src/target/linux_gtk/term_draw.rs:506-511,605-611` — `term_write`/`term_scroll`
mutate the fixed grids from the worker thread while a queued draw may read them
on the GTK main thread; no lock/atomic, so a draw can render torn rows mid
`memmove` (visual artifacts only — buffers are fixed-size, no memory unsafety).
macOS marshals every grid write to the main thread
(`performSelectorOnMainThread` waitUntilDone, §6.4) precisely to serialize
this. Also the "rows clamp at the bottom (no scroll in v1)" comment at :510 is
stale — the function scrolls. Fix: marshal grid writes (or lock), and correct
the comment.

## 4. macOS headless app bootstrap busy-spins the main thread at 100% CPU

`src/target/macos_aarch64/app/bootstrap.rs:459-466` — in headless mode
(MFB_MACAPP_HEADLESS, the automated-test path) `_main` spawns the worker then
executes `b .` (tight infinite loop), burning a full core for the whole run.
The finish helper's park path uses the intended `pause()` idiom (:741-743); the
GTK headless equivalent `_exit`s instead. Wastes CI core-time/power on every
headless app-mode test. Fix: replace `branch_self` with a `pause()` park loop.

## 5. Stale/misleading comments (docs)

Three comments verified wrong against behavior: (a) net/mod.rs:341-343 "An empty
host on a listener binds all interfaces" (false — see bug-113); (b)
macos_aarch64/app/app_io.rs:394-395 "_mfb_arena_alloc clobbers x10/x11/x20-x28"
(actual contract per .ai/compiler.md: clobbers all caller-saved x0–x17,
preserves x19–x28); (c) linux_gtk/term_draw.rs:510 "no scroll in v1" (it
scrolls, covered in item 3).

## Related (already tracked, not re-filed)

- **D1**: `net.accept` ignores its `timeoutMs` argument (blocks forever;
  poll/close imports dead) — **dup of audit-1-fs-net-thread.md OS-02** (HIGH,
  has the fix sketch). Documented API `net::accept(listener, timeoutMs)` "waits
  at most timeoutMs" is violated. Not re-filed; see OS-02.
- GTK worker passes argc=0/argv=NULL (bootstrap.rs:268) — explicit
  `TODO(plan-05)`, known.
