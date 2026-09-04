# bug-498: `thread::send`/`emit`/`transfer` allocates from the destination thread's arena unlocked → free-list corruption

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: HIGH
Class: security (memory safety — data race / heap corruption)

Status: FIXED (f2bf55a86) — found in audit-3, Surface 3 MEM-70; reproduced live by the lead, SIGSEGV 3/3

Regression Test: an rt fixture that sends in a loop to a busy worker and asserts a clean exit (currently SIGSEGVs).

## Summary

Passing a value between threads (`thread::send`, `thread::emit`,
`thread::transfer`) deep-copies it into the **destination** thread's arena by
temporarily repointing the pinned arena-state register at that thread's arena and
calling the ordinary allocator — before the queue mutex is taken, and while the
destination thread is itself allocating from the same arena. The arena allocator
has no synchronization at all, so the two racing quick-bin free-list pops hand out
the same block twice or overwrite a `next` link, corrupting the heap. Reachable
from ordinary MFBASIC (the canonical worker-plus-forwarding-parent shape) and
reproducibly fatal.

## Mechanism

```rust
// src/codegen/cleanup/thread/builder_thread_cleanup.rs:152-163
self.emit(abi::load_u64(&scratch10, &scratch9, arena_offset));
self.emit(abi::move_register(ARENA_STATE_REGISTER, &scratch10)); // x19 := destination arena
...
let copied = self.copy_value_to_current_arena(&arg_values[1].type_, &scratch9)?; // allocates THERE
```

`arena_offset` is `THREAD_OFFSET_ARENA_STATE` (parent→worker) or
`THREAD_OFFSET_PARENT_ARENA_STATE` (`thread.emit`), so both directions allocate
from a live foreign arena. The queue mutex is not taken until
`emit_symbol_call(symbol)` at `:233`, so the whole copy is unlocked. The allocator
hot path is a plain load-modify-store of a free-list head:

```rust
// src/codegen/memory/arena/arena.rs:161-165
abi::load_u64(&bin_head, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
...
abi::load_u64(&bin_next, &bin_head, 0),                 // faults on a raced link
abi::store_u64(&bin_next, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
```

`grep -rn 'mutex|_lock|atomic|ldxr|stxr|compare_exchange' src/codegen/memory/arena/`
returns nothing (lead-verified). The project states the per-thread-arena rule
twice (`.ai/canvas-threading.md` §3; `runtime_helpers.rs`) but only for *frees*;
the send-path *allocation* breaks the same invariant.

## Reproduction (lead-run, live)

`spikes/audit-3/MEM-70/` — parent sends 200 000 messages to a worker that
allocates in a loop. `mfb build spikes/audit-3/MEM-70 && ./…/build/mfb_project.out`
→ `Segmentation fault` (exit 139), 3/3 runs (agent saw 5/5, both send
directions). lldb: both threads fault at the same PC in `_mfb_arena_alloc`'s
quick-bin pop on a garbage link.

## Best fix

A cross-thread send must not allocate from the peer thread's live arena. Either
copy into a process-global region guarded by the same mutex the queue already
uses (take the lock *before* the copy, at `:233`'s lock, not after), or copy into
the *sender's* arena and hand ownership across with the queue entry. Whichever, no
unlocked allocation may target another thread's arena. If a shared region is
introduced, the allocator's quick-bin pop for that region needs a real
CAS/lock — the current plain load/store is unsafe for any shared arena.

## Non-goals

Do not change `thread::send`/`emit`/`transfer` MFBASIC semantics; do not add a
lock to the common single-thread allocation path (per-thread arenas stay
lock-free); preserve the move semantics of `thread::transfer`.

## Prior art

None as a defect (searched `arena race`, `cross-thread`, `thread::send`, `x19`
across `bugs/`, `bugs/completed/`, `bugs/skipped/`, `audit-1-*`, `audit-2-*`).
The per-thread-arena invariant is documented for frees only
(`.ai/canvas-threading.md`, memory `arena-state-is-per-thread`); this is the
allocation-side gap.

## STATUS: FIXED (f2bf55a86)

Reproduced first. The committed spike (`spikes/audit-3/MEM-70`, worker never
drains) failed 3/3 here with `7-705-0009 ErrInterrupted` in 0.04 s — the parent
fills the small queue, blocks, and is woken when the worker completes — so it
barely exercised the race on this host. A draining variant (worker
`thread::receive`s every message while both threads allocate) SIGSEGVed 5/5, and
its mirror (worker emits, parent drains) 3/3; lldb showed BOTH threads stopped at
the same PC with `EXC_BAD_ACCESS address=0x23`, and the disassembly there is the
quick-bin pop (`ldr x9,[x8,#0x60]` — the bin head at
`ARENA_QUICK_BIN_BASE_OFFSET-8` — null check, `ldr x9,[x8]` on the torn `next`
link): exactly the mechanism above.

Fix (`src/codegen/cleanup/thread/builder_thread_cleanup.rs`): the send/emit/
transfer lowering no longer repoints `ARENA_STATE_REGISTER` at the peer's arena.
The deep copy lands in the SENDER's own arena and the block is handed across.
Neither option in "Best fix" was taken verbatim: taking the queue mutex before
the copy cannot serialize against the destination's *ordinary* `arena_alloc`
(which takes no lock, by the non-goal), and a shared locked region would have
needed a process-global mutex init plus an unlock on the copy's
`ErrOutOfMemory` path. The sender-arena copy needs neither: a free only touches
the FREEING thread's arena state (`arena_free` pushes onto its own quick/large
bins and never asks which arena carved the block — no counters, no chunk
membership), and no arena but the main one is ever destroyed (`arena_destroy` is
called only from `_mfb_shutdown`), so the handed-over block stays mapped and the
receiver adopts it exactly as it owned the destination-arena copy before. The
failed-send pending-free list is unchanged (the reader adopts the orphan). The
`error_arena_restore_slot` plumbing became dead and was removed.

Tests: `tests/rt_thread_send_cross_arena.rs` (both directions, draining receiver,
deterministic totals, 3 runs each — RED: SIGSEGV; GREEN after) and
`tests/codegen_thread_send_no_arena_repoint.rs` (no compiled function writes
`x19` — RED: 6 writes in `main`/`churn`; GREEN after). 40/40 additional stress
runs of both programs clean with identical totals.

Gates: `cargo test --bin mfb` green; `cargo check --all-targets` clean;
`scripts/test-accept.sh … 'rt-behavior/threads/*'` 47/47 passed (execution
proof incl. resource/STATE/bidirectional transfers); full artifact gate 10 diffs,
all in `byte-identity/{thread,resource-xfer-slots}` (the only goldened fixtures
containing a thread send) on all five targets — `.ast`/`.ir` unchanged, and the
macos-aarch64 `.ncode` delta is confined to the two send-containing functions
(frame 32 bytes smaller: four dropped saved-arena slots; 64 fewer instructions:
the repoint/restore sequences). Regenerated those ten `.ncodesum`; scoped gates
then 0 diffs.

Docs: spec `threading/{02_isolation,06_thread-runtime-helpers,07_control-block,
08_queue-semantics}.md` and `.ai/canvas-threading.md` §2 now state the
sender-arena hand-over rule.

Semantics: no MFBASIC surface or observable behavior of a correct program
changed — `thread::send`/`emit`/`transfer` keep their move/copy semantics; only
which arena carves the copy moved.
