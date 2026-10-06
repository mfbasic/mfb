# Control Block

A thread value is a **view**: a 112-byte resource record whose `HANDLE` word points
at the thread's **control block** (see § Views). The control block is a
`CONTROL_BLOCK_SIZE` (1072-byte) block allocated from the process heap
(`malloc`; `HeapAlloc` on Windows), not from any thread's arena, so either side
can free it. Everything the parent side and the worker share lives in it: one
lock, the run state and cancel flag, the stored result, the four queues, and the
block's own **block arena**. The layout is an implementation ABI between the
thread runtime helpers and generated code.
[[src/codegen/runtime/thread/control_block.rs:CONTROL_BLOCK_SIZE]]

```text
offset  field
0       lock                          (pthread_mutex_t; SRWLOCK on Windows)
64      state
72      cancelled
80      result tag
88      result value                  (block-arena copy, or the word itself)
96      result error                  (block-arena copy of the message)
104     result error source           (block-arena copy of the ErrorLoc, or 0)
112     refs                          (release count: handle side + worker)
120     table slot                    (index in the thread table)
128     block arena                   (arena state of the block's own arena)
136     result copy function          (per-type; 0 = a word-sized result)
144     OS handle                     (pthread_t / HANDLE; 0 once joined or detached)
152     entry function pointer        (closure)
160     seed                          (block-arena copy, or the word itself)
168     worker arena state            (heap block)
176     (unused)
192     inbound queue                 (data plane, parent -> worker)
384     outbound queue                (data plane, worker -> parent)
576     resource inbound queue        (resource plane, parent -> worker)
768     resource outbound queue       (resource plane, worker -> parent)
960     worker view                   (the ThreadWorker resource record, 112 bytes)
```
[[src/codegen/runtime/thread/control_block.rs:CONTROL_OFFSET_LOCK]]
[[src/codegen/runtime/thread/control_block.rs:CONTROL_OFFSET_RESOURCE_OUTBOUND_QUEUE]]
[[src/codegen/runtime/thread/control_block.rs:CONTROL_OFFSET_WORKER_VIEW]]

Every field is addressed with an immediate displacement off the block pointer, so
the block stays within rv64's signed 12-bit load/store range (a compile-time
assertion in `control_block.rs`).

`state = 0` means running (`THREAD_STATE_RUNNING`). `state = 1` means complete,
with the result stored (`THREAD_STATE_COMPLETED`); it stays there however many
times `thread::waitFor` reads the result. `state = 2` means the parent view was
dropped (`THREAD_STATE_CLOSED`); only that drop writes it, and the worker's exit
then stores no result, since nothing can read it.
[[src/codegen/runtime/thread/runtime_helpers.rs:THREAD_STATE_RUNNING]]

## Views

Both thread types are resource records (`RESOURCE_RECORD_SIZE`, the layout every
built-in resource shares): `TAG` at 0, `HANDLE` at 8 — the control block —
`CLOSED` at 16, and the optional `STATE` pointer at 24. Start initializes
`STATE` to zero; a stateful binding or worker parameter default-builds its own
payload. Parent and worker state types and payloads are independent.
[[src/codegen/engine/function/function_lowering.rs:lower_function]]
See `./mfb spec language threads` for the view-state contract.

- The **parent view** (`Thread`, tag `RESOURCE_TAG_THREAD`) is allocated by
  `thread::start` in the calling thread's arena, before anything else, and
  returned; a failed start frees it. Its close op is the hidden `thread.drop`
  (`builtin_resource_close_function` answers it structurally, since a thread type
  has no single nominal), so a parent view binding is an ordinary resource
  binding: its scope drop runs the close op and then frees the record
  (`resource_record_freed_at_drop`), `RETURN` moves it, and passing it to a
  function lends it.
- The **worker view** (`ThreadWorker`, tag `RESOURCE_TAG_THREAD_WORKER`) is
  embedded in the control block at `CONTROL_OFFSET_WORKER_VIEW`, filled in by
  `thread::start`; the trampoline passes its address as the entry's first
  argument. It has no close op: it lives exactly as long as the worker's
  reference.

Handle-based channel and parent lifecycle operations reach the block through
`emit_control_block_of`, which answers `ErrResourceClosed` for a null or closed
view, and `ErrResourceMoved` for a `Thread` view that `thread::transfer` handed to
another thread, before touching the block. The closed default an inline `TRAP` binds on the
error path of `thread::start` is a closed resource record with the thread tag and
a zero `HANDLE`, so no op ever reaches a block through it.
`thread::isCancelled` instead reads the pinned current-thread register; the
no-argument stdin subscription operations use the calling arena.
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:thread_is_cancelled_helper]]
[[src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_stdin_subscription_helper]]
[[src/codegen/error/constants/error_constants.rs:RESOURCE_TAG_THREAD]]
[[src/codegen/runtime/thread/control_block.rs:emit_control_block_of]]
[[src/codegen/resource/mod.rs:builtin_resource_close_function]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:resource_record_freed_at_drop]]

## The lock

One mutex guards the whole block: the state, the cancel flag, the result, every
queue, the release count, and every allocation or free in the block arena. Each
blocking wait — a full or empty queue in `send`/`receive`/`transfer`/`accept`/
`poll`, `thread::waitFor`, a worker's cancellable `os::sleep` — waits on one of the
queues' two condition variables with this mutex. A state change that can satisfy a
waiter broadcasts the condition variable that waiter uses; the worker's exit
broadcasts every one of them. [[src/codegen/runtime/thread/runtime_helpers_thread.rs:emit_block_lock]]

The mutex and the eight condition variables are initialized (`pthread_*_init`) by
`thread::start` before the worker exists. On Windows they are an SRWLOCK and
`CONDITION_VARIABLE`s through the thread seam's translation.

## The block arena

The block arena is an ordinary arena state (`ARENA_STATE_SIZE` bytes, a heap block,
zeroed, its entropy-fill stream seeded from the spawning thread's XOR the state's
address) whose chunks `_mfb_arena_alloc` maps on demand and `_mfb_arena_destroy`
unmaps. Every value that crosses between the two threads is deep-copied into it —
the seed at start, each message at its enqueue, the worker's result at its exit — so
nothing either thread reads lives in the other thread's arena.
[[src/codegen/runtime/thread/control_block.rs:lower_thread_block_arena_create]]

A copy into it runs under the block lock with the pinned arena register pointed at
the block arena for the length of one call, and restored afterwards
(`emit_with_arena`). The call is to a **per-type copy function**
(`_mfb_rt_thread_copy_value_<type>_<hash>`: `copy_value_to_current_arena` of that type,
returning a tagged result) whose address the call site passes to the helper; a
value that crosses as its word (a scalar or an enum member) has address 0 and is
stored as is. A matching **per-type drop function**
(`_mfb_rt_thread_drop_value_<type>_<hash>`) frees a value in whatever arena is current: a
flat block or a graph through the ordinary owned-value drop, and a resource record
(with its `STATE` block) after closing its handle if the record is still open. A
value of any other shape (a resource-carrying union or collection) is left to the
block arena's destruction. The functions are requested while the module's
functions lower and emitted once per module.
[[src/codegen/runtime/thread/control_block.rs:emit_with_arena]]
[[src/codegen/runtime/thread/value_functions.rs:lower_thread_value_function]]

A copy that runs out of memory is discarded (whatever it allocated stays in the
block arena until the block is freed) and the helper raises a plain
`ErrOutOfMemory`.

In a `--debug` build the block arena registers in the arena registry as a
`thread_block` arena (`./mfb spec tooling debug-report`).

## Lifetime

The block carries two references (`refs`, 2 at start): the handle side's and the
worker's. Each is given up exactly once, under the block lock:

- **The worker's**, by `_mfb_rt_thread_finish` at the end of its run, after it has
  stored its result and unmapped its own arena, in the same critical section that
  publishes `COMPLETED`.
- **The handle side's**, by the parent view's `thread.drop`. There is one parent
  view per thread and one drop of it — the resource rules give the record a single
  owner — so the drop is the handle side's one release. It closes the view first,
  then marks the block `CLOSED` and cancelled, closes every queue and wakes every
  waiter, joins a `COMPLETED` worker or detaches a `RUNNING` one (unless a
  `waitFor` already joined it), and gives up the reference.

Whichever side takes the count to zero frees the block
(`_mfb_rt_thread_block_release`): the table slot; every message a queue still owns,
through the queue's recorded drop function (so a resource transferred but never
accepted is closed); the block arena (the seed, the result and everything else in
it return to the OS); the worker's arena-state block (retired from the `--debug`
registry first); and the block.
[[src/codegen/runtime/thread/control_block.rs:lower_thread_block_release]]

A handle-side release that reaches zero first joins a worker it has not joined or
detached — the worker has already given up its reference, so it has nothing left
to do but unlock and return: its arena-state block is no longer in use, because the
worker unmaps its arena (`_mfb_arena_destroy`, which runs on that block) BEFORE it
releases. That ordering is what makes the detach path safe: a drop that detached a
`RUNNING` worker and then released last frees the block without waiting for the
worker, so the worker must not touch the block, or its arena state, after its own
release. The first `thread::waitFor` to see `COMPLETED` joins
the worker and zeroes the OS handle; the drop joins a `COMPLETED` worker and
detaches a `RUNNING` one (after cancelling it), zeroing the OS handle either way,
and skips both when the handle is already zero. A worker whose handle
side let go while it was running is detached, so it frees the block itself when it
finishes. [[src/codegen/runtime/thread/runtime_helpers_thread.rs:simple_thread_handle_helper]]

The worker's arena chunks are unmapped by the worker at the end of its run (see
`queue-semantics`); its arena-state block goes with the control block.

## The thread table

Every control block holds a slot in the process-global thread table
(`_mfb_rt_thread_table`, a writable data object). The table is bookkeeping only:
no per-message operation reads it, and its statically initialized mutex is taken
only by `thread::start` (acquire a slot) and by the block's final release (return
it). A used slot holds its block pointer; a free slot holds `(next << 1) | 1`, the
low bit telling the two apart, forming a free list whose head is the free index
plus one (0 = empty). The slot array is a heap block that starts empty and grows by
doubling from 64 slots when full; blocks never move, so growth is invisible
outside the table. [[src/codegen/runtime/thread/control_block.rs:lower_thread_table_acquire]]

```text
offset  field
0       mutex                         (statically initialized)
64      slots                         (heap array of capacity words)
72      capacity
80      used                          (live blocks)
88      free                          (free-slot list head: index + 1, 0 = empty)
96      high                          (slots ever handed out)
```
[[src/codegen/runtime/thread/control_block.rs:THREAD_TABLE_SIZE]]

## Queue layout

Each of the four queues is embedded in the block, `THREAD_QUEUE_SIZE` (192) bytes,
at the offsets above. A queue has no mutex of its own; the block lock guards it.

```text
offset  field
0       pthread_cond_t not_empty      (signalled on enqueue / broadcast on close)
64      pthread_cond_t not_full       (signalled on dequeue / broadcast on close)
128     capacity                      (requested limit)
136     count                         (current occupancy)
144     head index
152     tail index
160     closed flag
168     values pointer                (block-arena ring of capacity * 8-byte entries)
176     last read                     (the message the last read handed out, or 0)
184     drop function                 (the message type's; 0 = word-sized values)
```
[[src/codegen/runtime/thread/runtime_helpers.rs:THREAD_QUEUE_SIZE]]

A ring entry is the queued value: the block-arena copy the sender's helper made, or
the word itself. The ring is allocated from the block arena by `thread::start`
(`capacity * THREAD_QUEUE_ENTRY_SIZE` bytes; the limit is upper-bounded so the
product cannot wrap).

Each plane is split by direction so a thread's own send is never re-read by its own
receive: the inbound queue carries parent→worker traffic and the outbound queue
carries worker→parent traffic. The resource plane is fully independent of the data
plane, so a thread can carry both at once. The source-level contract is bounded
queues with the behavior specified by `./mfb spec threading queue-semantics`.

## See Also

* ./mfb spec threading thread-runtime-helpers — the helpers that read and write this block
* ./mfb spec threading queue-semantics — the queue contract and the message lifecycle
* ./mfb spec threading error-propagation — the result tag/value/error fields it stores
* ./mfb spec tooling debug-report — the `thread.` section and the `thread_block` arena kind
