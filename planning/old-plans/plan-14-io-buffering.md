# MFBASIC Opt-In Output Buffering Plan — overview

Last updated: 2026-07-05 (reference refresh — design unchanged; still unimplemented)
Overall Effort: large

> **2026-07-05 reference refresh.** Code moved since this was written: the io/read
> helpers now live in `src/target/shared/code/io_helpers.rs` (not the old
> `mod.rs:NNNN` lines — `mod.rs` was split and is now ~2.9k lines); the arena-state
> constants live in `src/target/shared/code/error_constants.rs`; the runtime symbol
> specs are in `src/target/shared/runtime/io_specs.rs`; spec/man trees moved to
> `src/docs/spec/**` and `src/docs/man/**`. `ARENA_STATE_SIZE` is no longer 104 — the
> allocator-01 quick bins + designated-victim carve were appended after this plan, so
> it is now `ARENA_CARVE_SIZE_OFFSET + 8` (~1144 B); the new buffer words append
> *after* the carve, and `ENTRY_STACK_SIZE`/`ENTRY_ARGC/ARGV` are already **derived**
> from `ARENA_STATE_SIZE` so they shift automatically. Coordinate the `ARENA_STATE`
> bump with **plan-15** (not plan-02 Phase 5, which plan-15 superseded).

**Split plan** (by effort into two small/medium sub-plans; see §Sub-plans). This file is the
overview holding the shared design; the phases live in the lettered sub-plans.

Give programs explicit, opt-in buffering of **output streams** — standard output
and `fs::` writable `File` handles — so a write-heavy loop collapses one `write()`
syscall per call into one per buffer-full, without changing the observable
behavior of any program that does not opt in. **Standard error is never buffered**
(§Non-goals). A correct implementation: with buffering **off** (the default) every
`io::print`/`io::write` and `fs::writeAll` reaches the OS exactly as it does today;
with buffering **on**, output is held in a per-stream buffer and drained on
`io::flush()`, before any stdin read, on buffer-full, on close of a `File`, and at
normal/​signalled program exit — and the bytes ultimately written (to the terminal
or to disk) are byte-identical to the unbuffered run.

This is the explicit escape hatch that plan-02's decision **D6** deferred:
plan-02 keeps output unbuffered because *silent* buffering changes observable
behavior (partial output on crash, stdout/stderr reordering, unflushed prompts,
data not yet on disk). This plan makes buffering a deliberate, user-controlled
mode instead. Scope note (your direction): the buffering controls are mirrored
per stream — `io::setBuffered`/`isBuffered`/`flush` for **stdout**, and the same
trio in `fs::` **per `File` handle** so each file is independently buffered or not,
the developer's choice. **stderr is never buffered** — the one stream whose bytes
must never be "lost" is the one stream with no buffer and no flush.

It complements:

- `./mfb spec io` and `src/docs/man/builtins/io/{flush,flushError,print,write}.txt`
  (the buffering contract the flush man page already promises; this plan makes it
  real). New builtins `io::isBuffered`/`io::setBuffered` get man pages + spec.
- `./mfb spec language memory-semantics` (unchanged — no value/copy/move impact).
- `planning/plan-02-perf-hotpaths.md` §4.5 / D5 / D6 (the per-arena
  `ARENA_STATE` buffer machinery this reuses, and the decision this unblocks).

## 1. Goal

- Add the io:: trio for **stdout**: `io::setBuffered(enabled AS Boolean)`,
  `io::isBuffered() AS Boolean`, and a load-bearing `io::flush()` (drains the
  stdout buffer; no-op when off). Default off, per thread.
- **Mirror the same trio into `fs::`, per `File` handle** (your direction):
  `fs::setBuffered(file, enabled)`, `fs::isBuffered(file) AS Boolean`,
  `fs::flush(file)` — each open file buffered or not, independently, the
  developer's choice. Default off per handle. (§4.5)
- **Remove `io::flushError`** — with stderr never buffered it can only ever be a
  no-op (§4.2.1). io:: and fs:: each flush their own buffers; nothing flushes
  stderr because nothing buffers it.
- Buffer the syscall-storm output paths: stdout (`io::print`/`io::write`) and
  incremental `fs::writeAll`/`writeAllBytes` on an open `File`. Whole-file
  `fs::writeText`/`writeBytes`/`append*`/`*Atomic` are NOT buffered — they already
  issue one write per call (Non-goals).
- Buffering defaults **off**, per thread. A program that never calls
  `setBuffered(TRUE)` has byte-for-byte identical output (terminal and on-disk)
  and identical golden/acceptance results to today.
- With buffering on, a 100k-line print loop (or 100k small `fs::writeAll`s) issues
  ~1 `write()` per buffer-full (e.g. 4 KiB) instead of 100k — closing most of the
  io-write gap (74ms→~15ms) *for programs that opt in*.

### Non-goals (explicit constraints)

- **No change to default behavior.** Buffering off is the default and is exactly
  today's unbuffered path; this plan adds nothing to the hot path of programs
  that don't opt in.
- **stderr is *never* buffered — unconditionally.** `io::printError`/
  `io::writeError` always write immediately to the OS, regardless of
  `io::setBuffered`. Error output can never sit in a buffer and so can never be
  "lost" on a crash, signal, or abort — that immediacy is the whole reason stderr
  exists. Because stderr is never buffered, **`io::flushError` is removed** (§4.2.1):
  `io::flush()` drains io's only buffer (stdout), `fs::flush(file)` drains a file,
  and stderr has nothing to flush.
- **Whole-file `fs::` writes are not buffered.** `fs::writeText`/`writeBytes`/
  `appendText`/`appendBytes`/`writeTextAtomic`/`writeBytesAtomic` already issue one
  `write` per call (the scan confirmed this), so buffering them buys nothing and
  would only add crash-loss risk. Only incremental `File`-handle writes
  (`fs::writeAll`/`writeAllBytes`) are buffered.
- **No value/copy/move/freeze impact.** This is runtime I/O state only; it does
  not touch `mfb spec language memory-semantics`. No new value kinds, no layout
  change to strings/collections. A `File` is still an owned non-copyable resource
  (§15); buffering only attaches a runtime byte buffer to it.
- **Buffers are per-stream and lock-free, never a shared global.** The stdout
  buffer is per-thread (per-arena state); each `File` buffer is per-handle, owned
  by that resource. No buffer is shared across threads or across streams. Output
  interleaves at flush granularity, never mid-write within one stream.
- **No silent auto-enable.** v1 does not auto-enable buffering for non-tty stdout
  (that would change observable behavior); left as an Open Decision.

## 2. Current State

- **`io::flush`/`io::flushError` already exist** as builtins (`src/builtins/io.rs:7`
  `FLUSH`, `:8` `FLUSH_ERROR`; both `AS Nothing`), with man pages
  (`src/docs/man/builtins/io/flush.txt`, `flushError.txt`). The flush man page already
  documents that "standard output may be buffered" and that flush "forces that
  drain" — but the runtime "does not maintain an additional MFBASIC output buffer
  of its own," so today flush only nudges host-stream buffering and is effectively
  a no-op. **This plan implements the buffer the man page already promises.**
- **`io::write`/`print` issue one `write()` syscall per call**, unbuffered
  (`lower_io_write_helper`, `src/target/shared/code/io_helpers.rs:3`). 100k prints =
  100k syscalls (io-write benchmark 74ms vs C 11ms).
- **`_mfb_shutdown` is the teardown hook** (`SHUTDOWN_SYMBOL`,
  `error_constants.rs:113`): runs on normal exit and is invoked by the SIGINT/SIGTERM
  handler before `_exit(128+signo)`; "internally gated and idempotent"
  ([[shutdown-and-signal-handlers]]). It already restores the terminal and frees
  the arena — the exact place to drain a pending output buffer.
- **Per-arena `ARENA_STATE` block** (`error_constants.rs`, `ARENA_STATE_REGISTER` =
  `x19`) is the established home for runtime-owned mutable state (RNG at offsets
  88/96; [[math-rng-pcg64]]). `ARENA_STATE_SIZE` now ends past the allocator-01 quick
  bins + carve (`ARENA_CARVE_SIZE_OFFSET + 8`); this plan appends the output buffer
  words after it. plan-15 (the stdin sibling) appends its stdin words in the same
  region — coordinate the two bumps if landing together.

## 3. Design Overview

Three pieces, layered low-risk first:

1. **Runtime output buffer** in the per-arena `ARENA_STATE` block: `OUT_PTR`,
   `OUT_FILLED`, `OUT_ENABLED`. Lazily arena-alloc a 4 KiB buffer on first
   buffered write. When `OUT_ENABLED == 0` (default), `io::write`/`print` take the
   exact path they do today (direct syscall) — zero overhead for non-opt-in code.
2. **The three control points** (`setBuffered`, `isBuffered`, real `flush`) plus
   the **mandatory drain hooks**: `_mfb_shutdown`, before every stdin read, on
   buffer-full, and on the `setBuffered(FALSE)` transition.
3. **Diagnostics/spec/man**: register the two new builtins, write man pages,
   update the `io` package spec and the flush man page to state the now-real
   semantics.

Correctness risk concentrates entirely in **completeness of the drain hooks** —
every path by which a program (or the runtime) stops producing or starts
consuming must flush first, or buffered bytes are lost or misordered. There is no
copy/move/aliasing risk; the buffer holds raw bytes, not values.

## 4. Detailed Design

### 4.1 Runtime state and the write path

Reserve three `ARENA_STATE` words: `OUT_PTR` (buffer, NULL until first use),
`OUT_FILLED` (bytes pending), `OUT_ENABLED` (0/1), appended after
`ARENA_CARVE_SIZE_OFFSET` (`error_constants.rs`). Bump `ARENA_STATE_SIZE` and
zero-init in the same place the block is already zeroed (so `OUT_ENABLED` starts 0
= off, `OUT_PTR` NULL = lazy-alloc on first buffered write). Coordinate the size
bump with plan-15's stdin words (one combined `ARENA_STATE_SIZE` change is
cleaner than two) — the layout move is sensitive, so the derived `ENTRY_ARGC/ARGV`
offsets and any hardcoded size must move with it ([[macos-codegen-latent-bugs]],
[[shutdown-and-signal-handlers]]).

`lower_io_write_helper` (`io_helpers.rs:3`) gains a one-branch prologue: if
`OUT_ENABLED == 0`, fall straight into today's direct-`write` path (no added cost
for the default). If enabled: if the incoming bytes + `OUT_FILLED` exceed buffer
capacity, drain first (one `write(1, OUT_PTR, OUT_FILLED)`), then either copy the
bytes into the buffer, or — for a write larger than the buffer — drain and write
it directly (never split a single `io::write` across two syscalls unnecessarily).
`io::print` (write + newline) routes through the same helper.

### 4.2 The control builtins

- **`io::setBuffered(enabled AS Boolean)`** — `enabled=TRUE` sets `OUT_ENABLED=1`
  (buffer lazily allocates on first write). `enabled=FALSE` **drains the buffer
  first, then** sets `OUT_ENABLED=0` (never strand pending bytes on the off
  transition). Returns `Nothing` (see Open Decisions for a prior-state variant).
- **`io::isBuffered() AS Boolean`** — returns `OUT_ENABLED != 0`.
- **`io::flush()`** — drains the MFBASIC stdout buffer (`write(1, OUT_PTR,
  OUT_FILLED)`; reset `OUT_FILLED=0`). **No-op when `OUT_ENABLED == 0`** (nothing
  pending; stderr is never buffered so there is nothing else for io:: to flush).
  May `FAIL` with the ErrIO-family error the write path already raises if the
  underlying `write` fails (auto-propagates like any io error). File buffers are
  flushed via `fs::flush(file)` (§4.5), not this call — each stream owns its flush.

These are native builtins (register in `src/builtins/io.rs` alongside `FLUSH`):
`IS_BUFFERED => Some("Boolean")`, `SET_BUFFERED => Some("Nothing")`, arity 0 and 1.

### 4.2.1 Removing `io::flushError`

`io::flushError` exists today (`io.rs:8` `FLUSH_ERROR`, runtime helper
`_mfb_rt_io_io_flushError`) purely as the stderr twin of `io::flush`. Since stderr
is never buffered (Non-goals), it can only ever be a no-op, so it is **removed**
and `io::flush()` (§4.2) absorbs the "drain stderr too" intent. This is a breaking
change to a public builtin — acceptable because the builtin is a no-op and the
language is pre-1.0, but it must be done completely. Touch points (verified):

- **Builtin + runtime + codegen:** drop `FLUSH_ERROR` from `src/builtins/io.rs:8`
  (the const and all match arms), the `io.flushError` runtime spec/symbol
  (`_mfb_rt_io_io_flushError`) from `src/target/shared/runtime/io_specs.rs:80`, the
  `"io.flushError"` dispatch arms (`src/target/shared/code/mod.rs:1102,1118`;
  `src/target/shared/code/data_objects.rs:112`), the app-mode bodies
  (`macos_aarch64/app/app_io.rs`, `linux_gtk/app_io.rs`), the platform plans
  (`macos_aarch64/plan.rs:161`, `linux_aarch64/plan.rs:91`,
  `macos_aarch64/mod.rs:52`, `linux_aarch64/mod.rs:52`). The shared `io.flush` path
  stays and gains the both-streams host drain.
- **Docs/spec:** delete `src/docs/man/builtins/io/flushError.txt`; remove the
  `io::flushError` cross-references in `flush.txt`, `printError.txt`,
  `writeError.txt`, `isErrorTerminal.txt`, `package.txt`; drop it from the builtin
  list (`src/docs/spec/language/18_builtin-functions.md:59`), the app console-io table
  (`src/docs/spec/app/03_console-io.md:22`), and the native list
  (`src/docs/spec/architecture/06_native.md:77`).
- **Tests:** update `tests/native_io_runtime.rs:442` (calls `io::flushError()`) and
  the `tests/package-import-as` fixture + goldens
  (`tests/package-import-as/src/lib.mfb:10` `console::flushError()` and its
  `golden/package_import_as.ast`/`.ir`) to use `io::flush` / `console::flush`. A new
  `tests/func_io_flush_*` proves the both-streams drain.

### 4.3 Mandatory drain hooks (the correctness core)

With buffering on, output MUST be drained at every boundary where holding bytes
back would be observable:

1. **Normal + signalled exit** — add a buffer drain to `_mfb_shutdown`
   (`SHUTDOWN_SYMBOL`, `error_constants.rs:113`) before/with the arena free, so `RETURN` from `main`,
   `EXIT PROGRAM`, and the SIGINT/SIGTERM handler all flush. `_mfb_shutdown` is
   idempotent, so a double-call is safe.
2. **Before any stdin read** — `io::readLine`/`io::input`/`io::readChar`/
   `io::readByte` drain the output buffer before blocking, so any buffered output
   already produced appears before the program waits (the flush man page calls
   this out). **`io::input` auto-flushes on both counts:** it drains any pending
   buffered stdout *and* its own prompt argument before blocking on the read — a
   buffered prompt must never sit unseen while the program waits for input. This
   generalizes the flush-before-read the prompt path in `lower_io_read_line_helper`
   (`io_helpers.rs:895`) already does for `io::input` to every stdin read.
3. **`setBuffered(FALSE)`** transition (§4.2).
4. **Buffer-full** during a write (§4.1).

What we explicitly do NOT (and cannot) flush: a hard crash (`SIGSEGV`/`SIGBUS`/
`SIGKILL`/`abort`) terminates before `_mfb_shutdown` can run, so buffered output
may be lost. This is inherent to opt-in buffering and is documented, not fixed —
it is precisely the trade-off the user accepts by calling `setBuffered(TRUE)`.

### 4.4 Threading

The stdout buffer lives in per-arena state (`x19`), so each thread buffers
independently with no lock. A spawned thread starts with `OUT_ENABLED=0` (default
off) unless we choose to inherit the parent's setting (Open Decision). Each thread
drains its own buffers in its own `_mfb_shutdown`/exit path. Cross-thread stdout
interleaving happens at flush granularity (whole buffers), which is no worse than —
and usually cleaner than — today's per-`write` interleaving.

### 4.5 `fs::` `File`-handle buffering — the mirrored, per-handle trio

stdout is one well-known fd governed by one thread-wide flag; a `File` is a
dynamic, owned resource and there can be many open at once, each with different
needs. So `fs::` mirrors the io:: control trio **per handle**, independently
(your direction — the developer decides per file):

- **`fs::setBuffered(file AS File, enabled AS Boolean)`** — turn buffering on/off
  for *this* `File`. `enabled=TRUE` attaches a 4 KiB output buffer (lazily, on
  first write); `enabled=FALSE` flushes then detaches. Default per handle: **off**.
- **`fs::isBuffered(file AS File) AS Boolean`** — this handle's mode.
- **`fs::flush(file AS File)`** — drain this handle's buffer now; no-op if the
  handle is unbuffered.

The buffer attaches to the `File` resource (not `ARENA_STATE`), so each handle is
independent and lock-free. `fs::writeAll`/`writeAllBytes` copy into the handle's
buffer when on and drain on buffer-full. Whole-file `fs::writeText`/etc. ignore
buffering entirely (Non-goals).

The load-bearing correctness rule is **flush-on-close**, stricter than stdout's: a
`File` is closed by lexical drop at scope exit (resource §15) or by an early
`fs::close`, and that close path **must flush the handle's buffer before the fd is
closed** — otherwise *persistent on-disk data the program already "wrote" is
silently lost*, worse than dropping terminal lines. So the resource drop/`close`
lowering flushes first, then closes; a failing final flush surfaces the same ErrIO
the write path raises. (A hard crash before drop can still lose an unflushed file
buffer — the same opt-in trade-off applied to disk; exactly why whole-file and
`*Atomic` writes stay unbuffered.)

### 4.6 Two independent flush surfaces, no cross-package coupling

With the trio mirrored per stream, the flush model is fully orthogonal — no
registry, no io:: reaching into fs::: `io::flush()` drains the stdout buffer (the
only io:: buffer; stderr has nothing to flush), and `fs::flush(file)` drains one
handle. Each owns its buffers and its flush. `_mfb_shutdown` drains the stdout
buffer on exit; each `File`'s drop/close drains its own. The only shared discipline
is "flush before you stop producing or close the fd," implemented independently on
each side.

## Layout / ABI Impact

- **No language-value layout change.** Strings, records, collections, copy/move/
  freeze, thread-transfer — all untouched. The buffer holds raw output bytes.
- **`ARENA_STATE_SIZE` grows by 3 words** for the stdout buffer (internal runtime
  state), appended after `ARENA_CARVE_SIZE_OFFSET`. Layout-sensitive (the derived
  `ENTRY_ARGC/ARGV` and any hardcoded size move with it); coordinate with plan-15's
  stdin words in one bump.
- **The `File` resource gains a buffer field set.** A `File` is an opaque resource
  record; adding buffer pointer/fill/enabled to its runtime layout is internal
  (resource handles are not copyable and never cross a value copy), so it does not
  touch `mfb spec language memory-semantics` — but the `File` record layout in
  `mfb spec memory` / the fs runtime grows. Document it there.
- **No new error codes** — `flush` reuses the existing write-failure (ErrIO)
  route. Man/spec updates: `io` package (`flush`, `isBuffered`, `setBuffered`) and
  `fs` package (`setBuffered`, `isBuffered`, `flush`).

## Sub-plans

Split by effort into three small/medium sub-plans; each holds its phases, tasks, and acceptance. The
design sections above (§1–§4) are the shared source of truth A/B reference; C carries its own
read-side design (it reuses only §4.5's per-handle buffer machinery).

| Doc | Effort | Phases | Depends on |
|---|---|---|---|
| [plan-14-A](plan-14-A-stdout-buffer.md) — opt-in stdout buffering | medium | buffer + controls + drain hooks (§4.1–§4.3) · read-flush · threading/signal (§4.4) | — |
| [plan-14-B](plan-14-B-fs-buffer.md) — `fs::` per-handle + remove `flushError` | medium | `fs::` per-handle buffering (§4.5) · remove `io::flushError` (§4.2.1) | A |
| [plan-14-C](plan-14-C-fs-read-buffer.md) — `fs::` per-handle **read** buffering | medium | transparent read buffer + buffered `fs::readLine`/`eof` · `readByte`/`readChar` + seek/write reconcile | B |

Note: A and B are opt-in **output** buffering; C is **transparent** (always-on) `fs::` **read**
buffering — the read-side sibling that fixes the `io read` benchmark (O(N²) `fs::readLine` → O(N)),
which is *not* plan-15 (plan-15 buffers stdin, not `fs::` files).

## Validation Plan

- **Function tests:** `tests/func_io_isBuffered_*`, `tests/func_io_setBuffered_*`,
  `tests/func_io_flush_*`, and the mirrored `tests/func_fs_setBuffered_*`,
  `tests/func_fs_isBuffered_*`, `tests/func_fs_flush_*` — `_valid/**` + `_invalid/**`
  (arg-type errors; fs forms take a `File`). Full coverage of the `Boolean`/`File`
  args and the 0-arg `io::flush`/`io::isBuffered` forms.
- **Runtime proof:** (a) buffering-off byte-identity vs today (golden diff = ∅);
  (b) buffering-on stdout byte-identity for a clean run; (c) syscall-count collapse
  under `strace`/`dtruss` for a buffered print loop AND a buffered `fs::writeAll`
  loop; (d) prompt-before-read with buffering on; (e) SIGINT mid-run flushes
  already-buffered stdout; (f) **flush-on-close**: a buffered `File` written then
  dropped/closed has byte-identical on-disk contents to the unbuffered run.
- **Soundness regression:** acceptance suite must be unchanged with the default
  (off) — this is the key guard that the feature is truly opt-in.
- **Doc sync:** new man pages `io/{isBuffered,setBuffered}.txt` and
  `fs/{setBuffered,isBuffered,flush}.txt`; update `io/flush.txt` (drop the "does
  not maintain an additional buffer" caveat; state the no-op-when-off rule),
  `io/package.txt`, `fs/package.txt` (per-handle buffering + flush-on-close), and
  the builtin lists in `src/docs/spec/language/18_builtin-functions.md`. Delete
  `io/flushError.txt` and all `io::flushError` cross-refs + spec-list entries
  (§4.2.1). Document the crash-loses-buffer caveat, the flush-on-close rule for
  files, and the stderr-never-buffered rule.
- **Acceptance:** `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- **Benchmark:** optionally add a buffered variant of io-write to demonstrate the
  win without changing the existing (unbuffered) io-write baseline.

## Open Decisions

- **`setBuffered` return type** (both `io::` and `fs::`) — `AS Nothing`
  (recommended; simplest, matches the signatures) vs. `AS Boolean` returning the
  prior state for a save/restore idiom. The save/restore pattern is already
  expressible via the matching `isBuffered`, so `Nothing` is sufficient.
- **Thread inheritance (stdout)** — a spawned thread starts stdout buffering
  **off** (recommended; explicit) vs. inherits the parent's `OUT_ENABLED`. (`File`
  buffering is per-handle, so it has no inheritance question.)
- **Non-tty auto-default** — keep buffering off for non-tty stdout in v1
  (recommended; preserves observable behavior) vs. auto-enable like C's
  full-buffering-for-pipes. Deferred until the at-exit drain is proven across all
  termination paths; auto-enable would change observable crash/ordering behavior.
- **Buffer size** — 4 KiB (recommended; matches the plan-02 stdin buffer, cheap
  per thread) vs. larger. A larger buffer reduces syscalls further but strands
  more output on a crash.
- **Line-buffered mode** — boolean = full buffering for v1 (recommended). A future
  three-state mode (off / line / full, flushing on `\n`) could mirror C's tty
  default, but a boolean covers the print-throughput use case with read-flush
  handling prompts.

## Non-Goals

- Buffering stderr. (`io::flushError` is removed, not made load-bearing — §4.2.1.)
- Auto-enabling buffering for any stream without an explicit `setBuffered(TRUE)`.
- Guaranteeing buffered output survives a hard crash / `SIGKILL`.
- Any change to the default (unbuffered) hot path or to value semantics.

## Summary

The language already exposed `io::flush` and a man-page promise that "standard
output may be buffered" — this plan implements that promise behind an explicit
`io::setBuffered(TRUE)` switch, adds `io::isBuffered`, gives `io::flush` real teeth,
and **mirrors the same trio into `fs::` per `File` handle** (`fs::setBuffered`/
`isBuffered`/`flush`) so each output stream is independently buffered at the
developer's discretion — while removing the now-redundant `io::flushError` and
keeping the default (off) path byte-identical to today. The two mechanisms are
orthogonal (one per-arena stdout buffer, one per-resource file buffer) with no
cross-package coupling. The only real engineering work is making the drain hooks
exhaustive — for stdout: flush on exit (the idempotent `_mfb_shutdown`), before
every stdin read, on buffer-full, on disable; for files: the **mandatory
flush-on-close/drop** so opt-in buffering never loses on-disk data — so that
turning buffering on is a pure performance change for any program that flushes or
exits/closes cleanly. There is no value/copy/move impact (`File` stays a
non-copyable resource);
the one documented limitation (a hard crash may drop unflushed bytes) is the
inherent cost the caller opts into.
