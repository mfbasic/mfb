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

- [ ] Run `rg -n 'thread::sleep' tests/ examples/ src/docs` and record every hit;
      classify each as mechanically rewritable to `os::sleep` (drop the handle
      arg) or needing thought. Fill the Prerequisites row.

Acceptance: a complete list of `thread::sleep` source callers with a rewrite note
each; zero unclassified.
Commit: —

### Phase 1 — Add `os::sleep` additively (both branches proven)

Delivers a working `os::sleep` while `thread::sleep` still exists — safe to land
alone; nothing depends on the removal yet.

- [ ] `src/builtins/os.rs`: add the `os.sleep` source surface (§4.1).
- [ ] `os_specs.rs` + `catalog.rs`: add `OS_SLEEP_SPEC` (§4.2).
- [ ] Extract parent/worker sleep bodies into emit-subroutines; add the `os.sleep`
      helper + dispatch (§4.2). Reuse each target's existing `nanosleep`/`Sleep`.
- [ ] `runtime_helpers_thread.rs`: store `x20`→`[arena+8]` in the worker
      trampoline (§4.3).
- [ ] Tests: `tests/rt-behavior/os/os-sleep-main-rt` — main-thread `os::sleep(50)`
      returns after ≥ the delay (monotonic check), never raises.
- [ ] Tests: `tests/rt-behavior/os/os-sleep-worker-cancel-rt` — a worker in
      `os::sleep(long)` wakes early with `ErrInterrupted` when the parent
      `thread::cancel`s (mirror the existing `thread-sleep-worker-cancel-rt`).
- [ ] Tests: `tests/rt-error/os/os-sleep-negative-rt` — `os::sleep(-1)` →
      `ErrInvalidArgument`.

Acceptance: the three new fixtures pass on all four targets; a worker `os::sleep`
is demonstrably interruptible and a main-thread `os::sleep` is a plain delay.
`thread::sleep` still works (untouched).
Commit: —

### Phase 2 — Remove `thread::sleep` + migrate (largest blast radius last)

- [ ] Rewrite every Phase-0 caller to `os::sleep`.
- [ ] Delete the `thread::sleep` source surface, direction split, dispatch arms,
      and both specs (§4.4).
- [ ] Migrate `thread-sleep-negative-rt` → covered by `os-sleep-negative-rt`
      (remove the old fixture); migrate `thread-sleep-worker-cancel-rt` →
      `os-sleep-worker-cancel-rt` (remove the old). Update any golden that named
      `thread.sleep`/`thread.sleepWorker`.
- [ ] Add a compile-error fixture: `thread::sleep(t, 1)` → unknown member.
- [ ] Docs: remove `src/docs/man/builtins/thread/sleep.md`; add
      `src/docs/man/builtins/os/sleep.md` (per `.ai/man_template.md`, via
      `scripts/update_man.sh`). Update the `thread` and `os` package overview pages.
- [ ] Spec: in `memory/04_arenas.md` change `+8 reserved` → `+8 workerThread`
      (TCB back-pointer, 0 on the main thread); remove the sleep entries from
      `threading/06_thread-runtime-helpers.md`; add `os::sleep` to
      `stdlib/14_os.md`. Update the `thread` man overview + error table.

Acceptance: `rg 'thread\.sleep\|thread::sleep\|sleepWorker' src/ tests/ src/docs`
returns only intentional history/removal references; `mfb man thread` shows no
`sleep`; `mfb man os sleep` renders; full `cargo test` + acceptance suite green
on all four targets.
Commit: —

## Validation Plan

- Tests: three new `os` fixtures (main delay, worker cancel, negative arg); a new
  unknown-member compile-error fixture for `thread::sleep`; removal of the two old
  `thread`-sleep fixtures. Unit tests in `os.rs` for `os.sleep` resolution
  (`rt(SLEEP, &["Integer"]) == Some("Nothing")`, arity/type negatives), mirroring
  the deleted `resolve_sleep_both_handle_sides`.
- Coverage check: the new `os.sleep` helper is exercised by the release-subprocess
  rt fixtures (per `coverage-measurement-mechanics` — integration coverage comes
  from the uncaptured release binary, not `--bin mfb` unit tests); confirm the
  fixtures are in the acceptance denominator.
- Runtime proof: a program that `os::sleep(100)`s on the main thread and prints
  monotonic elapsed ≥ 100 ms; a second that spawns a worker sleeping 5 s, cancels
  after 50 ms, and observes `ErrInterrupted` promptly.
- Doc sync: `mfb man os sleep`, `mfb man thread` (no sleep), `mfb spec memory
  arenas` (+8 role), `mfb spec threading`, `mfb spec stdlib` os. Run
  `.ai/specifications.md` sync + `scripts/update_man.sh`.
- Acceptance: full `cargo test` (never a single module — AGENTS.md), the
  byte-identity/acceptance golden harness on all four targets (expect and review
  the diffs named in §3), and `rustup run 1.96.0 cargo fmt --all` + the
  `repository/` fmt at session end.

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

<Filled in during execution.>

## Summary

The real engineering risk is the worker branch of `os::sleep` faithfully reusing
the existing cancellation-aware wait through an extracted subroutine — proven code,
new call plumbing — gated behind the migrated worker-cancel behavior test. The
arena+8 detection is arena-size-neutral and reuses a spec-reserved word. Untouched:
arena-state size/offsets, worker cancellation semantics, every non-sleep `thread::`
and `os::` call, and all wire/file formats.
