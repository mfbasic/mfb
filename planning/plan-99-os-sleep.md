<!-- Feature plan. plan-99: remove thread::sleep, add os::sleep(ms). -->

# os::sleep — remove thread::sleep, add a handle-free os::sleep(ms)

Last updated: 2026-08-15
Effort: large (3h–1d)

Replace the two handle-bound `thread::sleep` overloads with a single handle-free
`os::sleep(ms)` that blocks the *calling* thread and is automatically
cancellation-aware inside a worker. The single behavioral outcome a correct
implementation produces: `os::sleep(ms)` blocks the calling thread for at least
`ms` milliseconds on **any** thread (main or worker); in a worker it wakes early
with `ErrInterrupted` when the parent requests cancellation (exactly today's
worker `thread::sleep` behavior); a negative `ms` raises `ErrInvalidArgument` and
`ms == 0` returns immediately; and `thread::sleep` no longer exists in the
language surface.

The mechanism: the worker's thread-control-block (TCB) pointer is published into
the reserved, zero-initialized arena-state word at **offset +8**. `os::sleep`
reads `[arena+8]`; `0` (the main thread's zero-init default) takes a plain
`nanosleep`/`Sleep`, non-zero takes the existing cancellation-aware wait. This
reuses both existing sleep bodies verbatim and needs no new arena-state slot, so
the arena-state size is unchanged.

References:

- `mfb spec memory arenas` — arena-state layout; `+8 U64 reserved ; zero-initialized`
  (`src/docs/spec/memory/04_arenas.md`).
- `mfb spec threading` — per-worker arenas, TCB, cooperative cancellation
  (`src/docs/spec/threading/06_thread-runtime-helpers.md`).
- `mfb spec stdlib` os page (`src/docs/spec/stdlib/14_os.md`).
- `.ai/resources-packages.md` (builtin-package authoring seams),
  `.ai/arch-abi.md` (per-target nanosleep/Sleep), `.ai/testing-gates.md`.
- `AGENTS.md` — man templates + `scripts/update_man.sh`; spec-sync obligation.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Arena-state offset +8 is the reserved zero-init word | `grep -n '+8' src/docs/spec/memory/04_arenas.md` → `+8 U64 reserved ; zero-initialized`; no constant maps to +8 (`sed -n 330,480p src/codegen/error/constants/error_constants.rs`) and the only raw arena-register literals are 0 and 32 (`rg -no 'ARENA_STATE_REGISTER,\s*[0-9]+' src/ \| sed 's/.*,\s*//' \| sort -nu` → `0`, `32`) | **MET** (re-measured 2026-08-29) |
| No source program in-tree calls `thread::sleep` in a way the migration can't mechanically rewrite | `rg -l 'thread::sleep' --glob '*.mfb' .` → 10 files, all `(handle, ms)` → `(ms)` drops | **MET** (Phase 0 census below; every hit is a mechanical rewrite) |

Everything below is written against the world where +8 is free and reusable. If a
future change has claimed +8 (re-run the spec command), this plan appends a new
arena-state slot instead — a one-line offset change in Phase 1, not a redesign.

## 1. Goal

- `os::sleep(ms)` exists, is importable in every build (console and `--app`), and
  blocks the calling thread for ≥ `ms` ms on the main thread and on any worker.
- In a worker, `os::sleep` wakes early with `ErrInterrupted` on cancellation —
  byte-for-byte the cancellation semantics `thread::sleep(worker, ms)` has today.
- `os::sleep(negative)` → `ErrInvalidArgument`; `os::sleep(0)` → immediate return.
- `thread::sleep` is gone: `IMPORT thread` exposes no `sleep`, and the compiler
  rejects `thread::sleep(...)` as an unknown member.

### Non-goals (explicit constraints)

- **No new arena-state slot.** `ARENA_STATE_SIZE` (3768) must not change; reuse
  the reserved +8 word. (If +8 is unexpectedly occupied — Prerequisites — that is
  the one sanctioned deviation.)
- **No change to worker cancellation semantics.** The interruptible-sleep behavior
  moves from `thread::sleep(worker,…)` to `os::sleep` unchanged; it is not removed.
- **No sub-millisecond unit.** `os::sleep` takes `Integer` **milliseconds**,
  matching every existing thread timeout (`thread::sleep(t, ms)`, send/receive/poll
  all ms). No ns/ps variant. (Open Decisions records why.)
- **No new source keyword.** `os::sleep` is an ordinary package function; there is
  no `YIELD`.
- `os::sleep` on the main thread must never gain a spurious wakeup path — a plain
  uninterruptible delay there, identical to today's parent `thread::sleep`.

## 2. Current State

`thread::sleep` is the language's only sleep, and it is two helpers behind one
source name:

- **Source surface** — `src/builtins/thread.rs`: `SLEEP = "thread.sleep"` (`:22`),
  `P_SLEEP` requires a `Thread` handle + `Integer` (`:126`), registered
  `tf(SLEEP, "sleep", …)` (`:149`); resolution arm returns `Nothing` for both
  handle sides (`:278`); expected-args/param-name/type-hint entries (`:229,:340`);
  and unit tests (`resolve_sleep_both_handle_sides`, `:842`; plus `:644,:688,:708`).
- **Direction split** — `src/target/shared/code/builder_values.rs:2127`: a source
  `thread::sleep` lowers to `thread.sleepWorker` when the handle is a worker,
  else `thread.sleep`.
- **Parent body** — `lower_thread_sleep_helper`
  (`src/target/shared/code/runtime_helpers.rs:534`): reads the handle **only** for
  a liveness check (`:555` — "the sleep needs nothing else from the handle"),
  then a relative `nanosleep`/Win32 `Sleep`. Spec `THREAD_SLEEP_SPEC`
  (`src/target/shared/runtime/thread_specs.rs:64`).
- **Worker body** — `lower_thread_sleep_worker_helper`
  (`src/target/shared/code/runtime_helpers_thread.rs:811`): cancellation-aware —
  waits on the worker's inbound-queue condvar (the one `thread::cancel`
  broadcasts) via the `ThreadReadMode::WorkerSelf` structure, which reads the
  pinned current-thread register `x20` and checks the cancel flag; wakes with
  `ErrInterrupted`. Spec `THREAD_SLEEP_WORKER_SPEC` (`thread_specs.rs:91`).
- **Dispatch** — `runtime_helpers.rs:419-421` routes `thread.sleep` /
  `thread.sleepWorker` to the two bodies.

`os` builtins follow a clean, mirrorable pattern: source surface in
`src/builtins/os.rs` (`tf`/`ov`/`req`, e.g. `os.pid` `:15`); runtime-helper spec
in `src/target/shared/runtime/os_specs.rs` (`OS_PID_SPEC` `:53`) registered in
`src/target/shared/runtime/catalog.rs:121`; native body in
`src/target/shared/code/os/mod.rs`; per-target imports in each backend's
`plan.rs`/`mod.rs`. `os` is already importable in every build.

The arena-state layout (`src/docs/spec/memory/04_arenas.md`,
`src/target/shared/code/error_constants.rs`) has `+0 blockHead`, **`+8 reserved
; zero-initialized`**, `+16/+24 fillRng`, `+32 exitStatus`, … The entry shim
zeroes the whole `ARENA_STATE_SIZE` range before first use, and the thread-spawn
path zeroes a worker's freshly allocated arena state with the same size-derived
loop — so +8 is reliably `0` on the main thread and on a just-spawned worker
until something writes it. The TCB→arena link already exists (spec: the worker
arena is "referenced from its thread control block"); this plan adds the
arena→TCB back-reference at +8. The worker trampoline establishes both the arena
register (`x19`) and the current-thread register (`x20` = the TCB) —
`runtime_helpers_thread.rs:1032` (`move_register(CURRENT_THREAD, c_arg(0))`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `thread.rs` lines referencing `SLEEP`/`P_SLEEP` | 17 | `rg -c 'SLEEP\|P_SLEEP' src/builtins/thread.rs → 17` |
| Native sleep helper bodies to fold | 2 | `lower_thread_sleep_helper`, `lower_thread_sleep_worker_helper` (rg -n 'fn lower_thread_sleep' src/target → 2) |
| Existing sleep test fixtures to migrate | **7** (4 rt + 3 syntax), not 2 | `rg -l 'thread::sleep' --glob '*.mfb' tests/` — see Corrections |
| `os` man pages (add `sleep.md`) | **0 — moot** | `ls src/docs/man/builtins/` → no such dir; member docs are registry descriptor fields (`intro`/`desc`/`example`), see Corrections |
| In-tree `thread::sleep` `.mfb` callers | **10 files** | `rg -l 'thread::sleep' --glob '*.mfb' .` |

### Verified properties

- **+8 is free.** Spec labels it `reserved ; zero-initialized`; no named
  arena-state constant maps to +8 (`rg '= 8;' src/target/shared/code/error_constants.rs`
  → none in the arena block) and no raw-literal arena access hits +8
  (`rg 'ARENA_STATE_REGISTER, 8\b' src/target` → none; the only raw literals off
  the arena register are 0 and 32). **Verified by reading** `error_constants.rs`
  offsets 0–3760 and `arenas` spec.
- **TCB pointer is a U64 on every target.** It is a native pointer; aarch64,
  x86_64, and riscv64 are all LP64. It is exactly the value `x20`
  (`CURRENT_THREAD`) holds in a worker (`abi.rs:227`). Fits +8's `U64` exactly.
- **Detection-via-+8, TCB-access-via-x20 is sound.** In a worker both `x20` and
  `[arena+8]` name the same TCB (trampoline sets `x20`; Phase 1 sets `+8` from
  the same value), so `os::sleep`'s worker branch can reuse the existing
  worker body unchanged (it reads `x20`); on the main thread `[arena+8] == 0` so
  that branch — and `x20`'s main-thread unreliability — is never reached.
  **Verified by reading** `lower_thread_sleep_worker_helper` and the WorkerSelf
  path in `thread_queue_read_helper` (`runtime_helpers_thread.rs:1257+`).

## 3. Design Overview

Three independent pieces, layered additive-first:

1. **Publish the TCB at arena+8** (thread-spawn trampoline). One store, right
   after `x19`/`x20` are both live. Zero-init already gives the correct main-thread
   default, so this is the *only* write needed.
2. **`os::sleep` helper** = validate `ms` → load `[arena+8]` → branch: `0` runs
   the parent (relative `nanosleep`/`Sleep`) body, non-zero runs the worker
   (cancellation-aware condvar) body. Both bodies already exist; extract them as
   emit-subroutines and call them from one `os.sleep` helper. New source surface
   in `os.rs`, spec in `os_specs.rs` + `catalog.rs`.
3. **Remove `thread::sleep`** — source surface, direction split, the two old
   dispatch entries, both old specs — once `os::sleep` subsumes them.

**Where correctness risk concentrates:** the worker branch of `os::sleep` must
reproduce the interruptible-wait semantics exactly (early `ErrInterrupted` on
cancel, no spurious shortening from a parent `send`). This is *identical code* to
today's worker body, so the risk is in the reuse plumbing (does the extracted
subroutine get called with the same register/frame contract?), not new logic.
Schedule it behind the migrated `thread-sleep-worker-cancel-rt` behavior test.

**Where design uncertainty concentrates:** the arena+8 detection branch — proven
above, but Phase 1 makes it the first, cheapest experiment: land `os::sleep`
additively and prove both branches with new tests *before* touching
`thread::sleep`.

**Byte-identity is NOT this plan's gate.** Behavior and codegen change on every
target: the trampoline gains a store (expected diff), `os::sleep` is new codegen,
and `thread.sleep`/`thread.sleepWorker` codegen is removed. Targets expected to
diff: **all four** (`macos_aarch64`, `linux_x86_64`, `win_x86_64`,
`linux_riscv64`). The gate is rt-behavior/rt-error tests + acceptance goldens,
not `.ncode` byte-identity. A golden diff here is the plan working; regenerate and
review it, don't treat it as a stop.

**Rejected alternatives:**
- *Scavenge a different reserved word / append a new slot.* +8 is genuinely free
  and reusing it is arena-size-neutral; appending would grow `ARENA_STATE_SIZE`
  and churn every target's arena-clear for no benefit.
- *Detect worker-ness via `x20` directly.* Unreliable on the main thread (the
  entry stub reuses `x20` as scratch; `runtime_helpers.rs:655`). `[arena+8]` is
  reliable on both, because `x19` is pinned for every thread.
- *Keep a handle-bound worker `thread::sleep` alongside `os::sleep`.* Two sleeps,
  split vocabulary, and a worker that forgets the handle loses cancellation. One
  context-aware `os::sleep` is the whole point.
- *Nanosecond/picosecond unit or a `YIELD` keyword.* See Non-goals / Open Decisions.

## 4. Detailed Design

### 4.1 `os.sleep` source surface (`src/builtins/os.rs`)
- `const SLEEP: &str = "os.sleep";`
- `const P_SLEEP: &[Parameter] = &[req("ms", &[], "Integer")];`
- Register `tf(SLEEP, "sleep", &[ov(P_SLEEP)])`; return type `Nothing`.
- Error surface documented as `ErrInvalidArgument` (negative `ms`) + `ErrInterrupted`
  (worker cancellation). Note the **uniform** `ErrInterrupted` (Open Decisions):
  it is declared on every call but can only fire in a worker.

### 4.2 `os.sleep` runtime helper
- Spec `OS_SLEEP_SPEC` in `os_specs.rs` (`call: "os.sleep"`, returns `Nothing`),
  registered in `catalog.rs`.
- Emit body: reuse `nanosleep`/`Sleep` per-target import already used by
  `thread.sleep` (present in each backend's `plan.rs`; no new import).
- Extract the current parent-sleep and worker-sleep instruction sequences from
  `lower_thread_sleep_helper` / `lower_thread_sleep_worker_helper` into two
  emit-subroutines. `os.sleep` body:
  1. `ms < 0` → `ErrInvalidArgument`; `ms == 0` → return `Nothing`.
  2. Load `tcb = [ARENA_STATE_REGISTER + 8]`.
  3. `tcb == 0` → parent-sleep subroutine (relative timespec), return `Nothing`.
  4. else → worker-sleep subroutine (absolute deadline, WorkerSelf condvar wait
     reading `x20`), return `Nothing` or `ErrInterrupted`.
- Dispatch `"os.sleep"` in the `runtime_helpers.rs` match (co-located with the
  folded bodies).

### 4.3 Publish TCB at arena+8 (`runtime_helpers_thread.rs`)
- In the worker trampoline, immediately after `x19` (arena) and `x20` (TCB) are
  both established (anchor `:1032`), emit
  `store_u64(CURRENT_THREAD, ARENA_STATE_REGISTER, 8)`.
- Main thread writes nothing: the entry shim's whole-`ARENA_STATE_SIZE` zero leaves
  +8 = 0. No entry-path change.

### 4.4 Remove `thread::sleep`
- `thread.rs`: delete `SLEEP`, `P_SLEEP`, the `tf(SLEEP,…)` registration, the
  resolution arm, expected-args/param/type-hint entries, and the sleep unit tests.
- `builder_values.rs:2127`: delete the `thread.sleep` direction split.
- `runtime_helpers.rs:419-421`: delete both dispatch arms (bodies now live under
  `os.sleep`).
- `thread_specs.rs`: delete `THREAD_SLEEP_SPEC` and `THREAD_SLEEP_WORKER_SPEC`
  (+ their `catalog.rs` registrations).

## Compatibility / Format Impact

- **Source API:** `thread::sleep` removed (breaking); `os::sleep(ms)` added. A
  `.result`-style rejection is not needed — an unknown-member error on
  `thread::sleep` is the correct diagnostic.
- **Arena-state layout:** unchanged size/offsets; `+8` changes role from
  `reserved` to `workerThread` (a TCB back-pointer). Spec text updates; no byte
  layout change.
- **Codegen:** changes on all four targets (see §3). No wire/file format touched.

## Phases

### Phase 0 — Census in-tree callers

- [x] Run `rg -l 'thread::sleep' --glob '*.mfb' .` plus `rg -l 'thread::sleep'
      --glob '*.rs' tests/ src/` and `rg -n 'thread::sleep' src/docs`, and record
      every hit; classify each as mechanically rewritable to `os::sleep` (drop the
      handle arg) or needing thought. Fill the Prerequisites row.

**Census (2026-08-29).** The plan's own command (`rg -l 'thread::sleep' tests/
examples/`) has a blind spot: `tools/thread-package-sources/` (compiled worker
packages) also calls it. The `.mfb` census below is the whole-repo glob.

MFB source callers (10 files) — every one is the same mechanical rewrite
`thread::sleep(h, ms)` → `os::sleep(ms)` + `IMPORT os`; none needs thought:

| File | Hits | Rewrite |
|---|---|---|
| `tests/byte-identity/thread/src/main.mfb:30` | 1 (`t1, 0`) | mechanical; drives a `.ncode` golden — regenerate |
| `tools/thread-package-sources/thread_runtime_workers/src/lib.mfb:199,204` | 2 (worker 200 / 5000) | mechanical; **package source** → re-run `scripts/sync-package-mfp.sh` |
| `tools/thread-package-sources/thread_cover_worker/src/lib.mfb:32` | 1 (`w, 0`) | mechanical; same `.mfp` re-sync |
| `tests/rt-error/threads/thread-sleep-negative-rt` | 1 | fixture MIGRATES to `tests/rt-error/os/os-sleep-negative-rt` |
| `tests/rt-behavior/threads/thread-sleep-parent-rt` | 2 | fixture MIGRATES to `tests/rt-behavior/os/os-sleep-main-rt` |
| `tests/rt-behavior/threads/thread-sleep-worker-rt` | 1 | fixture MIGRATES to `tests/rt-behavior/os/os-sleep-worker-rt` |
| `tests/rt-behavior/threads/thread-sleep-worker-cancel-rt` | 1 | fixture MIGRATES to `tests/rt-behavior/os/os-sleep-worker-cancel-rt` |
| `tests/syntax/threads/func_thread_sleep_valid` | 2 | MIGRATES to `tests/syntax/os/func_os_sleep_valid` |
| `tests/syntax/threads/func_thread_sleep_worker_valid` | 1 | folds into `func_os_sleep_valid` (one sleep, no handle side) |
| `tests/syntax/threads/func_thread_sleep_invalid` | 5 | REPURPOSED: an unknown-member fixture for `thread::sleep(t, 1)` + a new `func_os_sleep_invalid` for arity/type negatives |

Rust tests embedding MFB source: `tests/codegen_thread_c_return_x86_64.rs:59`
(`thread::sleep(t, 1)`, asserts the `nanosleep` emission) — mechanical rewrite to
`os::sleep(1)`. The other `*.rs` hits (`tests/common/mod.rs`,
`rt_macos_d4_union_state_tls.rs`, `rt_macos_tls_write_capacity.rs`,
`rt_http_async_stream.rs`) are Rust `std::thread::sleep` in harness code — **not
callers**, no change.

Docs: `src/docs/spec/threading/06_thread-runtime-helpers.md` (:48,:51,:81,:83),
`src/docs/spec/language/16_threads.md` (:48,:49,:73,:77),
`src/docs/spec/language/18_builtin-functions.md:73` — Phase 2 doc edits.

Acceptance: a complete list of `thread::sleep` source callers with a rewrite note
each; zero unclassified. **MET** — 10 `.mfb` files + 1 Rust-embedded + 3 spec
pages, all classified above.
Commit: 5a4f59dba (census) + this phase's plan update

### Phase 1 — Add `os::sleep` additively (both branches proven)

Delivers a working `os::sleep` while `thread::sleep` still exists — safe to land
alone; nothing depends on the removal yet.

- [x] `src/codegen/builtins/os/func_sleep.rs`: the `os.sleep` descriptor +
      authored docs + `Body::abi_function` body, registered from
      `builtins/os/mod.rs` (§4.1; the path/shape corrections are Correction 1).
- [x] ~~`os_specs.rs` + `catalog.rs`: add `OS_SLEEP_SPEC`~~ — moot: neither file
      exists. Runtime specs are DERIVED from the registry
      (`rg -l 'os_specs|thread_specs' src/` → no matches;
      `registry::runtime_specs` builds the catalog), so registering the descriptor
      IS registering the spec. Proven by the working fixtures below.
- [x] Extract parent/worker sleep bodies into emit-subroutines
      (`emit_relative_sleep` in `runtime/thread/runtime_helpers.rs`;
      `emit_cancellable_sleep_wait` + `emit_cancellable_sleep_interrupted` in
      `runtime_helpers_thread.rs`), add `lower_os_sleep_helper` calling both
      behind the arena+8 branch. No dispatch arm needed — `Body::abi_function`
      routes by registry name.
- [x] `runtime_helpers.rs`: store `%thread`→`[arena+8]` in the worker trampoline,
      right after the arena register is loaded from the TCB (§4.3), with the new
      `ARENA_WORKER_THREAD_OFFSET` constant naming the reserved word.
- [x] **Added task** — per-target wiring the plan did not list: `os.sleep` in each
      backend's runtime-call surface (`linux_common`/`win_x86_64`/`macos_aarch64`
      `mod.rs`) and an import arm in each `plan.rs`. `os::sleep` carries BOTH
      branches in one body, so it declares `nanosleep`/`Sleep` AND the
      mutex/condvar/clock subset the worker wait uses.
- [x] **Added task** — `ErrInterrupted`'s message data object is gated on
      `os.sleep` in `memory/data/data_objects.rs`. Without it an `os::sleep`-only
      program links no `_mfb_rt_thread_*` symbol, so it misses the standard
      error-message set and fails with a dangling
      `_mfb_str_error_interrupted` relocation (the bug-256 class) — hit on the
      first smoke build.
- [x] **Added task** — `tools/thread-package-sources/os_sleep_workers`: a new
      worker package (`sleepThenReturn`, `sleepUntilCancel`,
      `waitForCancelForever`) calling `os::sleep` inside a worker. A NEW package
      rather than new members on `thread_runtime_workers`, whose `.mfp` is
      committed in 26 consumer fixtures (`find tests -name
      thread_runtime_workers.mfp | wc -l` → 26) that would all have churned.
- [x] Tests: `tests/rt-behavior/os/os-sleep-main-rt` — main-thread `os::sleep(50)`
      returns after ≥ the delay (monotonic check), and `os::sleep(0)` is immediate.
- [x] Tests: `tests/rt-behavior/os/os-sleep-worker-cancel-rt` — a worker in
      `os::sleep(5000)` wakes early with `ErrInterrupted` when the parent
      `thread::cancel`s.
- [x] Tests: `tests/rt-error/os/os-sleep-negative-rt` — `os::sleep(-1)` →
      `ErrInvalidArgument`.
- [x] **Added task** — `tests/rt-behavior/os/os-sleep-worker-rt`: an uncancelled
      worker `os::sleep(200)` runs to COMPLETION (the census's fourth fixture; the
      cancel test alone cannot tell a working wait from one that returns instantly).
- [x] Unit test `codegen::builtins::os::tests::sleep_resolves_handle_free` — the
      descriptor half the deleted `resolve_sleep_both_handle_sides` covered:
      `Integer`→`Nothing`, arity/type negatives, and strict rejection of a thread
      handle in the `ms` slot.
- [x] **Added task (bug found, fixed here)** — three hand-written Win64 shims in
      `emit_windows_thread_call` kept a live value in `c_arg(1)` (rdx) across an
      `unsigned_divide_registers`, which writes the division REMAINDER there. See
      Correction 5; without the fix `os::sleep` could not meet this phase's
      acceptance on Windows.

Acceptance: the new fixtures pass on all four targets; a worker `os::sleep` is
demonstrably interruptible and a main-thread `os::sleep` is a plain delay.
`thread::sleep` still works (untouched). **MET, measured per target:**

| Target | How | Result |
|---|---|---|
| macos-aarch64 (host) | `scripts/test-accept.sh target/release/mfb /tmp/p99-accept 'os-sleep-*'` | `acceptance tests passed (4 test(s) ran)` |
| linux-x86_64 glibc | `FILTER=os-sleep scripts/linux-runtime-proof.sh … 2228 linux-x86_64 glibc` | 4 passed, 0 failed |
| linux-x86_64 musl | `… 2227 linux-x86_64 musl` | 4 passed, 0 failed |
| linux-riscv64 musl | `… 2229 linux-riscv64 musl` | 4 passed, 0 failed |
| windows-x86_64 | built `-target windows-x86_64`, shipped to box 2230, run | `immediate ok/slept ok/done`, `result 5/slept full`, `interrupted`, `ErrInvalidArgument` |
| linux-aarch64 | `mfb build -target linux-aarch64` (box 2224 is down: "Connection refused") | compiles + links; runtime proof deferred to the box returning |

Commit: ac231d499

### Phase 1.5 — Windows uncaught-error code line (found in Phase 1)

An unlisted prerequisite discovered by Phase 1's Windows acceptance run: the
`os-sleep-negative-rt` fixture's expected output includes the error CODE line, and
on Windows that line rendered empty. Not a sleep bug (Correction 6), but Phase 2
migrates this exact fixture, so its golden must be honest on every target.

- [x] Root-cause the empty `Error: ` code line on Windows. **It is the Win64
      scratch/argument-bank aliasing hazard `map_scratch_register`'s own plan-47-B
      note predicted** ("a hand-written helper that parks a value in low scratch and
      then stages call arguments over it would corrupt it — but only under Win64
      codegen, which no backend selects yet … so it is not rediscovered as a silent
      Windows-only miscompile"). In
      `entry.rs:emit_write_integer_to_stderr_with_labels`, the digit buffer's cursor
      is `SCRATCH[13]` (`x23` → `r8`) and the write's length argument is
      `string_length_register()` = MFB argument 2, which the **Win64** bank also
      realizes as `r8` (`CALL_ARGS_WIN64[2]`). The length was computed first, so it
      overwrote the cursor, and the following `mov <data>, SCRATCH[13]` handed the
      write the digit COUNT as its buffer ADDRESS — `WriteFile` failed silently and
      printed nothing. On SysV/AArch64/RISC-V the two are different registers, which
      is why only Windows showed it.
- [x] Fix it: read the cursor into the data-pointer register BEFORE computing the
      length. Safe on every ABI, and the length's now-possible `dst == rhs` case is
      already encoded correctly (`alu3` negates and adds). No separate bug document
      — the fix is three lines and lands here.
- [x] Re-run the rt-error fixture on box 2230.

Acceptance: an uncaught runtime error on Windows prints its dotted code, matching
macOS/Linux. **MET** — box 2230 `os_sleep_negative_rt.exe` now prints
`Error: 7-705-0002` then the message (was `Error: ` then the message); macOS host
prints the same with `[exit 255]`, and `linux-x86_64` glibc re-ran 4/4 green after
the change.
Commit: 9332abd4b

### Phase 2 — Remove `thread::sleep` + migrate (largest blast radius last)

- [x] Rewrite every Phase-0 caller to `os::sleep`: the two
      `tools/thread-package-sources` package sources (`thread_runtime_workers`'s
      `sleepThenReturn`/`sleepUntilCancel`, `thread_cover_worker`'s coverage row),
      `tests/byte-identity/thread/src/main.mfb`, and the embedded program in
      `tests/codegen_thread_c_return_x86_64.rs`. Re-ran
      `scripts/sync-package-mfp.sh target/release/mfb` for the committed `.mfp`
      copies (see Correction 7).
- [x] Delete the `thread::sleep` source surface (`thread/mod.rs` member +
      `sleep_params`), the `builder_values` direction split, the `sleepWorker`
      companion-symbol emission in `builder/mod.rs`, the `Nothing`-return entry in
      `builder_value_semantics.rs`, both helper bodies
      (`lower_thread_sleep_helper`, `lower_thread_sleep_worker_helper`) and
      `lowering::lower_sleep`, the per-target import arms, and the
      `thread.sleep`/`thread.sleepWorker` rows in all three runtime-call surfaces.
      No specs to delete — they are derived (Correction 1).
- [x] Migrate the fixtures: the four `thread-sleep-*` rt fixtures are removed
      (`tests/rt-behavior/threads/thread-sleep-{parent,worker,worker-cancel}-rt`,
      `tests/rt-error/threads/thread-sleep-negative-rt`), each covered by its
      Phase-1 `os` counterpart. `func_thread_sleep_valid` +
      `func_thread_sleep_worker_valid` merge into `tests/syntax/os/func_os_sleep_valid`
      (one spelling, both contexts) and `func_thread_sleep_invalid` splits into
      `tests/syntax/os/func_os_sleep_invalid` (arity/type negatives) and the
      unknown-member fixture below. Goldens regenerated with `sync-goldens.sh`.
- [x] Add a compile-error fixture: `tests/syntax/threads/func_thread_sleep_removed`
      calls `thread::sleep` on a parent AND a worker handle; both produce
      `SYMBOL_UNKNOWN_IDENTIFIER: Built-in package `thread` does not export
      `thread.sleep``.
- [x] ~~Docs: remove `src/docs/man/builtins/thread/sleep.md`; add
      `src/docs/man/builtins/os/sleep.md` via `scripts/update_man.sh`~~ — moot:
      that tree and script are retired (Correction 1). The man page IS the
      descriptor: `os::sleep`'s intro/desc/example live in `func_sleep.rs`, and
      `mfb man os sleep` renders them.
- [x] Spec: `memory/04_arenas.md` `+8 reserved` → `+8 workerThread` with a
      paragraph on why the pinned current-thread register cannot serve;
      `threading/06_thread-runtime-helpers.md` drops both sleep symbols, the
      direction-split row, and the `thread::sleep` section, gaining a
      "Sleeping inside a worker" section; `language/16_threads.md` drops both
      signatures and re-states the semantics on `os::sleep` (including the
      cancellation-point list); `language/18_builtin-functions.md` moves `sleep`
      from the `thread` list to the `os` list; `stdlib/14_os.md` gains a
      "Blocking the calling thread (sleep)" section and two error-table rows.

Acceptance: `rg 'thread\.sleep|thread::sleep|sleepWorker' src/ tests/ src/docs`
returns only intentional history/removal references; `mfb man thread` shows no
`sleep`; `mfb man os sleep` renders; full `cargo test` + acceptance suite green on
all four targets. **MET:**

- Census after removal (`rg -n 'thread\.sleep|thread::sleep|sleepWorker' src/ tests/`):
  the only hits are the deliberate ones — the `func_thread_sleep_removed` fixture
  (its source and its golden diagnostic), the `thread/tests.rs` assertions that
  `thread.sleep`/`thread.sleepWorker` resolve to nothing, and the prose in
  `threading/06_thread-runtime-helpers.md` / `language/16_threads.md` that says the
  call no longer exists.
- `mfb man thread | grep -i sleep` → no match; `mfb man os | grep -i sleep` →
  `os::sleep  Block the calling thread for a number of milliseconds`;
  `mfb man os sleep` renders the full page.
- `cargo test --no-fail-fast` — see Validation Plan.
- `scripts/test-accept.sh` and `scripts/artifact-gate.sh all` — see Validation Plan.

Commit: —

## Validation Plan

Everything below was run; the results are recorded inline.

- **Tests.** Four new `os` fixtures (main delay, worker completes, worker cancel,
  negative arg) replacing the four `thread-sleep-*` ones; `func_os_sleep_valid` /
  `func_os_sleep_invalid` replacing the three `thread` sleep syntax fixtures; a new
  `func_thread_sleep_removed` unknown-member fixture; and the unit test
  `codegen::builtins::os::tests::sleep_resolves_handle_free` replacing the deleted
  `resolve_sleep_both_handle_sides`, with `thread/tests.rs` now asserting that
  `thread.sleep`/`thread.sleepWorker` resolve to nothing.
- **Coverage.** The `os.sleep` body is exercised by the release-subprocess rt
  fixtures, which are in the acceptance denominator (`test-accept.sh` ran all four;
  the harness reports `1275 test(s) ran`).
- **Runtime proof.** `os-sleep-main-rt` prints `slept ok` only when a monotonic
  clock shows ≥ 40 ms elapsed for a 50 ms sleep; `os-sleep-worker-rt` proves an
  uncancelled 200 ms worker sleep runs to completion; `os-sleep-worker-cancel-rt`
  cancels a 5000 ms worker sleep after ~50 ms and requires `ErrInterrupted`.
  Cross-target results are in the Phase 1 table (macOS host, linux-x86_64
  glibc+musl, linux-riscv64 musl, windows-x86_64 on box 2230). Diagnostic timings
  measured on box 2230 after the Win64 shim fixes: main `os::sleep(1500)` → 1505 ms,
  main `os::sleep(50)` → 51 ms, worker `os::sleep(200)` → 205 ms.
- **Doc sync.** `mfb man os sleep` renders the authored page; `mfb man thread |
  grep -i sleep` → no match; `mfb man os | grep -i sleep` → the summary row.
  `mfb spec memory arenas` documents `+8 workerThread`; `mfb spec threading` has
  the "Sleeping inside a worker" section and no sleep helper symbols;
  `mfb spec stdlib os` has the sleep section and two error rows;
  `mfb spec language threads` / `builtin-functions` re-point to `os::sleep`.
  `cargo test --bin mfb citation` (spec citations resolve) — 5 passed.
- **Acceptance.** `scripts/test-accept.sh target/release/mfb /tmp/p99-accept2` →
  `acceptance tests passed (1275 test(s) ran)`, 0 mismatches.
- **Byte identity.** `scripts/artifact-gate.sh target/release/mfb all` →
  `1259 tests, 1406 build(s), 1732 golden(s) checked, 0 diff(s)`. The 125 diffs it
  reported before regeneration were classified against a main-tip baseline run of
  the same command (`0 diff(s)`), so every one was this plan's (Correction 7).
- **Full suite.** `cargo test --no-fail-fast` — green (see Phase 2 commit).
- **Formatting.** `rustup run 1.96.0 cargo fmt --all` plus the `repository/` pass.

## Open Decisions

- **Uniform `ErrInterrupted` on `os::sleep`.** *Recommended:* declare
  `ErrInterrupted` on every `os::sleep` (fires only in a worker); a shared `FUNC`
  can be called from both main and worker, so the compiler cannot localize the
  error surface, and this keeps one honest signature. *Alternative:* two
  functions (a main-only never-interrupted sleep + a worker interruptible one) —
  rejected by the "remove `thread::sleep`, one sleep" goal, recorded so it is not
  re-litigated. (§4.1)
- **Unit = milliseconds.** *Recommended:* ms — matches every existing thread
  timeout, is the unit Win32 `Sleep` speaks and the honest cross-target
  granularity floor, and makes the `thread::sleep`→`os::sleep` migration
  unit-preserving. *Alternative:* ns (false precision no scheduler delivers;
  inexpressible through `Sleep`) / ps (off by ~6 orders) — rejected. Sub-ms needs
  are served by `datetime::monotonicNanos()` + spin. (Non-goals)

## Corrections

1. **Every source path in §2/§4 is stale — the tree was reorganized after this
   plan was written (2026-08-15).** Measured 2026-08-29
   (`rg -l 'thread\.sleep|sleepWorker' src/`):

   | Plan says | Actually |
   |---|---|
   | `src/builtins/thread.rs` | `src/codegen/builtins/thread/mod.rs` (descriptor) + `lowering.rs` (bodies) + `tests.rs` |
   | `src/builtins/os.rs` | `src/codegen/builtins/os/mod.rs` + one `func_<name>.rs` per member |
   | `src/target/shared/code/builder_values.rs` | `src/codegen/engine/value/builder_values.rs` |
   | `src/target/shared/code/runtime_helpers.rs` | `src/codegen/runtime/thread/runtime_helpers.rs` |
   | `src/target/shared/code/runtime_helpers_thread.rs` | `src/codegen/runtime/thread/runtime_helpers_thread.rs` |
   | `src/target/shared/code/error_constants.rs` | `src/codegen/error/constants/error_constants.rs` |
   | `os_specs.rs` + `catalog.rs` registration | **Gone — moot.** Runtime specs are DERIVED from the registry (`registry::runtime_specs`); `thread_specs.rs`/`os_specs.rs` no longer exist. No spec to add and none to delete. |
   | `src/docs/man/builtins/os/sleep.md` (+ `scripts/update_man.sh`) | **Gone — moot.** `src/docs/man/builtins/` does not exist; member docs are the `intro`/`desc`/`example` fields of the `RegistryFunction` descriptor (`.ai/man-content.md`). `os::sleep`'s man page IS its `func_sleep.rs`. |

   Consequence for §4.4: removing `thread::sleep` is a descriptor-member deletion
   + a `builder_values` direction-split deletion + the two helper-body deletions;
   there are no hand-written specs or catalog rows to remove.

2. **§4.2's "extract emit-subroutines" is the right shape, but the two bodies do
   NOT share an argument contract.** Measured by reading
   `lower_thread_sleep_helper` / `lower_thread_sleep_worker_helper`: both take
   `(handle = c_arg(0), ms = c_arg(1))`. `os.sleep` has ONE argument
   (`ms = c_arg(0)`), so each extracted body is parameterized by where `ms` lives
   and (worker) where the TCB comes from. The parent body's `ErrResourceClosed`
   handle-state check is dropped for `os::sleep` — there is no handle — a
   deliberate narrowing.

3. **§3's "the worker body reads `x20`" is wrong; it reads its handle argument.**
   Measured: `lower_thread_sleep_worker_helper` stores `c_arg(0)` to
   `HANDLE_OFFSET` and loads the queue/cancel fields off it — `CURRENT_THREAD`
   (`x20`) is never referenced in that body. This makes the reuse *simpler*, not
   harder: the `[arena+8]` TCB value is fed straight into the worker body as its
   handle, so the branch does not depend on `x20` at all. The §2 "Verified
   properties" claim about `WorkerSelf` reading `x20` describes
   `thread_queue_read_helper`, not the sleep body.

4. **Fixture population is 7, not 2** (Measured populations, Phase 0 census): 4 rt
   fixtures (`thread-sleep-{negative,parent,worker,worker-cancel}-rt`) and 3
   syntax fixtures, plus 2 `tools/thread-package-sources` package sources that
   need `scripts/sync-package-mfp.sh` re-run — the plan never mentions the `.mfp`
   re-sync, and a stale committed `.mfp` is silently mis-lowered.

5. **Three pre-existing Win64 shim bugs blocked this plan's Windows acceptance —
   fixed here.** `emit_windows_thread_call` names ABI tokens directly, so its values
   land on the Win64 call bank (`c_arg(1)` = **rdx**). The x86-64 `div` expansion
   writes the quotient to rax and the REMAINDER to rdx (`div_seq`), and that is only
   sound for ALLOCATED code — rax/rcx/rdx are never allocatable
   (`implicit_clobber_registers_are_never_allocatable`), but hand-written token code
   is outside that guarantee. Three arms parked a live value in `c_arg(1)` across a
   divide:

   - `nanosleep` — the `sec*1000` term was destroyed, so a Windows sleep of a whole
     second or more slept only `ms % 1000`. Measured on box 2230 BEFORE the fix:
     `main os::sleep(1500) measured ms: 505`; after: `1505`.
   - `clock_gettime` — the `1e7` scale was destroyed, so `tv_nsec` was garbage and
     EVERY absolute deadline built by `emit_thread_deadline` was wrong.
   - `pthread_cond_timedwait` — this arm also *ignored* `abstime` entirely and polled
     a fixed 20 ms, on the stated theory that "the shared callers loop on their own
     deadline, re-checking the predicate after each wake". That theory is FALSE:
     every caller treats a non-zero return as "the deadline elapsed". So on Windows
     every timed wait expired ~20 ms in — a worker `thread::sleep(t, 200)` returned
     in ~20 ms and a bounded `thread::send`/`receive` gave up 20 ms into its timeout.
     The arm now honors `abstime` (and keeps its own math off `c_arg(1)`).

   **Pre-existing, verified at main tip 5f17afd7c** (`git worktree add --detach
   /tmp/p99-head main`, release build, `-target windows-x86_64` build of the
   UNCHANGED `tests/rt-behavior/threads/thread-sleep-worker-cancel-rt`, run on box
   2230): printed `no interrupt 0`, not its golden's `interrupted`. Measured after
   the fix: `worker os::sleep(200) measured ms: 205`,
   `worker thread::sleep(200) measured ms: 211`, and the fixture prints
   `interrupted`. Fixed here per AGENTS.md ("never leave a bug you found") and
   because Phase 1's acceptance ("passes on all four targets") is unreachable
   without it.

6. **Windows renders an uncaught runtime error's CODE line empty** — `Error: `
   instead of `Error: 7-705-0002`, with the message line correct. Confirmed
   pre-existing and unrelated to sleep at main tip 5f17afd7c (the untouched
   `thread-sleep-negative-rt`, built by the main-tip compiler, prints the same on box
   2230). Tracked as its own defect — see Phase 1.5.

7. **Two golden-regeneration steps the plan never mentions, both required.**

   - **Committed `.mfp` copies.** Editing a `tools/thread-package-sources/*`
     package source leaves every consumer's committed `.mfp` stale, and a stale
     `.mfp` is silently mis-lowered. `scripts/sync-package-mfp.sh
     target/release/mfb` updated **26** copies for this plan. It also updated 3
     that were ALREADY stale at main tip — `regex_thread_workers`,
     `union_xfer_workers`, `union_xfer_stateless_workers`. Verified pre-existing by
     running the same script in the main-tip worktree (`updated 3, unchanged 113`);
     they are kept regenerated rather than reverted, since a knowingly-stale
     committed artifact is the bug that memo warns about. The script's 6
     `build-failed` rows are identical at main tip (negative fixtures).
   - **`.ncode`/`.ncodesum` goldens outside `tests/byte-identity/`.**
     `scripts/regen-ncodesum.sh` only sweeps `tests/byte-identity/*/golden/`, so
     the 8 diffs under `rt-behavior/crypto/crypto-ec-valid` and
     `syntax/app/macos-app-mode-*` stayed red after a full regen. Added
     `scripts/regen-outside-ncode.sh` (same contract, handles the `--app` target
     suffix) so the next codegen change does not rediscover this.

   Diff classification, per `.ai/testing-gates.md`: the gate was run at main tip
   first — `artifact-gate.sh all` → **0 diff(s)** — so all 125 diffs on this branch
   are this plan's, and regenerating them is correct rather than masking drift. The
   bulk are the Phase-1.5 entry-stub instruction swap, which every binary carries.

## Summary

The real engineering risk is the worker branch of `os::sleep` faithfully reusing
the existing cancellation-aware wait through an extracted subroutine — proven code,
new call plumbing — gated behind the migrated worker-cancel behavior test. The
arena+8 detection is arena-size-neutral and reuses a spec-reserved word. Untouched:
arena-state size/offsets, worker cancellation semantics, every non-sleep `thread::`
and `os::` call, and all wire/file formats.
