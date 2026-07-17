# plan-52-B: record reclamation — free the pointed-at blocks at drop, `moved` as CLOSED bit 1

Status: **COMPLETE.** The drop path reclaims the STATE payload, the read buffer, and the
output buffer; `moved` is bit 1 of the CLOSED word at no size cost, and
`RESOURCE_RECORD_SIZE_BYTES` is still 80. **Leak proof, measured on one fixture at 20 000
cycles: 961 MB → 31 MB.** Retention no longer scales with the I/O a resource did.

Both traps the Summary predicted were real and are handled: the close runs **before** the
buffer free (or the drain strands buffered data), and the moved bit suppresses the frees so
a transferred record's blocks are not freed out from under the receiver (`bugs/bug-257`
confirms the STATE pointer is copied, not deep-copied). A third, unpredicted one bit
harder: the close's OK/already-closed/failure paths all converged on the same `done` label
the null-slot guard jumps to, so freeing there would have re-opened bug-246 — the label had
to be split.

Last updated: 2026-07-16
Effort: medium (1h–2h)
Depends on: nothing (independent of A/C/D — land in any order)

Every resource a program opens leaks, for the life of its thread. Not just the 80-byte
record — its **output buffer, its read buffer, and its STATE payload** too. Nothing frees
any of them. A long-lived thread that opens and closes files in a loop grows without
bound.

This sub-plan reclaims what the record *points at* — at **drop**, not at close — and adds
a `moved` flag as bit 1 of the existing CLOSED word, at no size cost.

The single outcome: **a loop opening and closing N files retains O(N × 80) bytes instead
of O(N × (80 + buffers + state)), and the retained bytes are flat per resource regardless
of how much I/O it did.**

References:

- `planning/res.md` §2 fact #10, §5 Q2 — where this was found, and the retention analysis.
  **Read first.**
- `./mfb spec language resource-management` §15 — "[STATE] is freed when the resource
  drops or is closed"; the spec already claims the free this sub-plan implements.
- plan-38 (`planning/old-plans/`) — the offset-8 closed-flag invariant this extends.
- plan-14-B / plan-14-C — the output and read buffers being reclaimed.

## 1. Goal

- `fs::openFile` → write → scope exit reclaims the output buffer, the read buffer, and the
  STATE payload. A loop of N open/close cycles shows flat per-resource retention.
- A `moved` flag exists at **bit 1** of the CLOSED word (offset 8); every existing
  closed-guard refuses a moved resource with no new code.
- `x.state` after an explicit `fs::close(x)` still works (it must not become a null
  dereference).
- `RESOURCE_RECORD_SIZE_BYTES` stays **80**.

### Non-goals (explicit constraints)

- **The 80-byte record is NOT freed.** It is the tombstone — it holds the closed flag that
  makes re-close idempotent and that every alias reads. Keeping it is the design (res.md
  §3.1), not an oversight. A resource-handle LUT was considered and **rejected** (§3).
- **`RESOURCE_RECORD_SIZE_BYTES` must stay 80.** Every resource kind shares the size "so
  the generic thread-transfer copy stays uniform", with per-backend asserts in
  `audio/mod.rs`, `tls/mod.rs`, `tls/macos.rs`. Growing it churns all of them.
- **`RESOURCE_OFFSET_CLOSED` stays 8**, and bit 0 keeps meaning exactly what it means
  today. plan-38 made this a compiler-enforced invariant; `moved` must be additive.
- **`fs::close(f)` semantics.** It releases the OS handle and reports failure. It must not
  start freeing memory — see §4.
- **`emit_resource_state_init`'s null-check.** "Allocate once; a carried state survives a
  move" is required by plan-52-D.
- **Track B (resource-scoped ownership).** res.md §1. This sub-plan is correct under the
  current borrow rule and does not presuppose Track B.

## 2. Current State

**Record layout** (`src/target/shared/code/error_constants.rs:646-689`), one uniform
80-byte block for *every* resource kind.

> **Correction (found in Phase 2):** the table below, and the layout's own comment, say the
> buffer words are carried "inertly" by non-`File` resources. That means *nothing reads
> them* — **not** that they are zeroed. Only `File`'s open helpers zero words 24–72 after
> the PRNG-poisoned arena alloc; `net`'s `emit_make_handle` initializes 0/8/16 and leaves
> the rest poisoned. Any future reader of those words must ask the resource kind first, or
> it is reading a poison value. This cost 14 segfaulting fixtures.

| Offset | Field | |
|---|---|---|
| 0 | `FILE_OFFSET_FD` | the OS handle |
| 8 | `FILE_OFFSET_CLOSED` | u64 holding **0 or 1** — 63 bits spare |
| 16 | `FILE_OFFSET_STATE` | → STATE payload block |
| 24/32/40 | `BUF_PTR` / `BUF_FILLED` / `BUF_ENABLED` | → output buffer block (plan-14-B) |
| 48/56/64/72 | `READ_PTR` / `READ_POS` / `READ_FILL` / `READ_AT_EOF` | → read buffer block (plan-14-C) |

`RESOURCE_RECORD_SIZE_BYTES = 80`; `RESOURCE_OFFSET_CLOSED = 8`, enforced by `const`
asserts here and in the three backend modules. A user cannot add fields — `ResourceDecl`
is `{visibility, name, close_fn, thread_sendable, line}` (`src/ast/types.rs:246-254`).

**Nothing frees anything.** `lower_fs_close_helper` (`src/target/shared/code/fs_helpers_io.rs:840`)
drains, closes the fd, sets `CLOSED = 1`, returns — no free. At a `RES` bind
(`src/target/shared/code/builder_control.rs:255-297`) the cleanup chain registers
`ActiveCleanup::Resource` (call the close op); the `OwnedValue` → `arena_free` branch is an
`else if` a resource never reaches. `emit_resource_cleanup_call`
(`src/target/shared/code/builder_codegen_primitives.rs:1512`) only calls the close symbol.
Grepping `BUF_PTR`/`READ_PTR` against `free` returns **nothing**.

**The `.state` read path has no closed-guard** — `src/target/shared/code/builder_value_semantics.rs:175-190`
loads offset 16 and hands back the pointer. This is what forces free-at-drop (§4).

**Precedent to mirror:** `ActiveCleanup::OwnedValue` → `arena_free` for flat values. Its
soundness rests on an explicit invariant — *"Copy-insertion (`lower_value_owned`)
guarantees this block is unaliased, so the free is sound and once-only."* The blocks freed
here are reached only through the record, so the same once-only property holds by a
different argument (§3).

## 3. Design Overview

Two independent pieces.

**(a) Free the pointed-at blocks at drop.** Extend the resource drop path to `arena_free`
the STATE payload, the output buffer, and the read buffer, then null their pointer words.
The 80-byte record stays.

Once-only argument: these blocks are reachable **only** through the record's pointer
words. Nulling as we free makes a second drop a no-op, exactly as the closed flag does for
close. No aliasing analysis needed — unlike `OwnedValue`, whose soundness needs
copy-insertion's unaliased guarantee.

**(b) `moved` as CLOSED bit 1.** The CLOSED word is a u64 storing 0 or 1. Bit 0 keeps
meaning closed; bit 1 means moved. Every existing guard is `load; compare 0; branch_ne`, so
**a moved resource refuses every operation with no new code** — the flag is free. Only the
paths that want to *distinguish* `ErrResourceMoved` from `ErrResourceClosed` change.

Correctness risk concentrates in **(a)**, specifically in *when* the free runs (§4) and in
the drop path already being subtle: `emit_resource_cleanup_call` carries a null-slot guard
(bug-246 — a trapped initializer leaves the slot at its entry-zeroed 0, and the close
helper dereferences offset 8, so a null read would SIGSEGV). The new frees sit behind that
same guard.

**Rejected: a resource-handle LUT.** A heap table of `Integer` entries, RES holding an
index, flags packed into the top bits, so the record itself could be freed and only 8
bytes retained. Rejected because (i) it puts a dependent load + mask on **every** resource
access, including `.state`, where today there is a direct pointer; (ii) arenas are
per-thread, so `thread::transfer` would need to re-register the record in the receiver's
LUT and rewrite the index, while the sender's entry still points at the record — a
cross-thread double-free, the exact hazard the flag was meant to prevent; (iii) a global
LUT would need atomics on grow, cutting against "threads do not share… resources". It buys
72 bytes per resource for that. Full analysis: res.md §10 lineage.

**Rejected: freeing the 80-byte record itself.** Then the closed flag has nowhere to live,
and every alias would dereference freed memory. The record *is* the tombstone.

## 4. Free at drop, not at close

`fs::close(f)` must **not** free the blocks.

The `.state` read path has no closed-guard: `s.state` loads offset 16 unconditionally. If
close freed the STATE payload and nulled offset 16, then

```basic
RES f AS File STATE Cursor = fs::openFile("x")
fs::close(f)
LET p = f.state.pos      ' -> null dereference
```

would segfault. §15 does not say `.state` is illegal after an explicit close, and nothing
enforces it.

At **drop**, the binding is gone and nothing can name the resource, so the free is safe.
This also preserves the split the design rests on: **close releases the OS handle; drop
reclaims memory.** They are different events and §15 already treats them as such.

(§15's "freed when the resource drops **or is closed**" is looser than what is safe to
implement. Phase 3 tightens the wording to "drops"; the observable behavior — you cannot
outlive your own scope — is unchanged for every program that does not read `.state` after
an explicit close.)

Order within the drop, per resource:

1. Existing null-slot guard (bug-246).
2. Call the close op — **must be first**: the output-buffer drain writes `BUF_PTR[0..BUF_FILLED]`
   to the fd. Freeing the buffer before the drain strands buffered data on the floor.
3. `arena_free` + null: `BUF_PTR`, `READ_PTR`, `STATE`.

Step 2 before step 3 is the load-bearing ordering.

## Compatibility / Format Impact

- **Layout: unchanged.** 80 bytes, same offsets. `moved` lives in spare bits of an
  existing word.
- **`.mfp`: untouched.**
- **Observable behavior:** only `x.state` after an explicit `fs::close(x)` — legal today
  (reads a live payload), still legal after (§4 keeps the free at drop). No program changes
  meaning.
- **Codegen goldens: will shift** — the drop path gains frees. Expected and intended;
  every resource-using fixture moves. Regenerate and confirm the delta is only that.
  **(Wrong, as it turned out: the artifact gate showed 0 diffs across 1141 goldens. No
  in-tree fixture carries a native-code golden for a resource-dropping function, so the
  frees are invisible to the gate. This mattered — it means the gate could NOT have caught
  the `net::` segfault, and only the runtime suite did.)**

## Phases

### Phase 1 — `moved` as CLOSED bit 1

Lowest risk: additive, no free path touched.

- [x] Define `RESOURCE_CLOSED_BIT = 0` / `RESOURCE_MOVED_BIT = 1` in
      `src/target/shared/code/error_constants.rs`; keep `RESOURCE_OFFSET_CLOSED = 8` and
      the existing `const` asserts. Added `RESOURCE_MOVED_CLOSED_VALUE = "3"` — a moved
      record sets **both** bits, so every existing `!= 0` guard rejects it unchanged.
- [x] Have `thread::transfer` set bit 1 on the **sender's** record, in
      `copy_resource_to_current_arena`. **Ordering trap found:** the mark must come
      AFTER the flag-word copy — flagging first hands the *destination* an
      already-moved record and makes the transferred handle unusable.
- [x] Add `ErrResourceMoved` (`7-703-0009`, registry + `standard_error_messages`);
      the close helper's `already_closed` guard now tests bit 1 and reports it.
- [ ] ~~Tests: `tests/rt-error/` — using a transferred resource from the sender reports
      moved~~ — **not constructible; see below.**

**Why there is no sender-use rt-error fixture.** Use-after-move is a *compile* error
(`TYPE_USE_AFTER_MOVE`, pinned by `ownership-use-after-move-invalid`), and
`deactivate_resource_cleanup` removes the sender's cleanup at compile time, so the sender's
drop never runs either. There is therefore no source-level program that reaches an
operation on a moved handle at runtime — which is exactly why the flag is a *backstop*, the
same status §15.6 gives the closed flag ("only a backstop that keeps the single close
idempotent when a handle is reachable by more than one path"). What the bit buys is real
but defensive: before it, the sender's record kept a **live fd the receiver now owns**, so
any alias the static rules do not track would silently operate on another thread's handle.
Claiming a runtime test here would mean writing one that cannot fail.

Acceptance: a moved resource refuses every op (via the existing `!= 0` guards, unchanged)
and reports `ErrResourceMoved`; `RESOURCE_RECORD_SIZE_BYTES` is still 80; the backend
asserts still hold.
Commit: —

### Phase 2 — free the pointed-at blocks at drop

The reclamation itself, behind Phase 3's leak test.

- [x] Extend the resource drop path (`emit_resource_cleanup_call`) to `arena_free` + null
      `BUF_PTR`, `READ_PTR`, `STATE` **after** the close call, inside the existing
      null-slot guard. Added `emit_resource_block_reclaim` + two helpers.
      **Control-flow subtlety:** the close's OK / already-closed / recorded-failure paths
      all converged on `done`, which the null-slot guard also branches to. Freeing at
      `done` would have dereferenced the null slot (re-opening bug-246). Split the label:
      the close paths now converge on a new `reclaim`, and only the null-slot guard jumps
      past it to `done`.
- [x] Confirm `arena_free`'s caller-saved clobber is handled: the record pointer is
      **reloaded from its stack slot after every `arena_free`** before nulling the pointer
      word, never held in a register across the call (`.ai/compiler.md` — no survivor set).
- [x] **Only a `File` may be asked for its buffers.** This sub-plan's §2 repeats the record
      layout's claim that the buffer words are "inert" on a socket/TLS/thread handle. They
      are inert only in the sense that **nothing read them** — they are not zeroed. Every
      resource kind shares the 80-byte record, but only `File`'s open helpers zero words
      24–72 after the **PRNG-poisoned** arena alloc; `net`'s `emit_make_handle` writes
      offsets 0/8/16 and leaves the rest as poison. Freeing them unconditionally therefore
      handed `arena_free` a poison value and **segfaulted every `net::` program during
      cleanup** — 14 acceptance fixtures, caught only because the suite runs the binaries.
      A null-guard is not enough (poison is not null); the frees are gated on the resource
      kind (`resource_uses_io_buffers`). The reclaim made "inert" words live, and the
      layout comment silently stopped being true.
- [x] The `STATE` payload is sized via `emit_inlined_block_size_from_ptr_slot`, not a
      constant — a STATE record inlines its `String` fields, so its block size is dynamic.
      This required carrying the binding's `state_type` on `ResourceCleanup` (it is known
      only at the bind).
- [x] The moved bit suppresses the frees: a transferred record's blocks belong to the
      receiver (bug-257 confirms the STATE pointer is copied, not deep-copied). The
      transfer also deactivates the sender's cleanup at compile time, so this path is not
      normally reached for a moved resource — the guard makes it a property of the code
      rather than of the caller.
- [x] Do **not** touch `lower_fs_close_helper`. Close stays memory-neutral (§4).

Acceptance: Phase 3's leak test shows flat per-resource retention; `x.state` after an
explicit close still reads correctly; `tests/rt-behavior/resources/resource-state-drop-valid`
still passes.
Commit: —

### Phase 3 — leak proof + validation

- [x] `tests/rt-behavior/resources/resource-reclaim-loop-valid` — a loop dropping two
      handles per cycle that carry, between them, all three reclaimable blocks (a 16 KiB
      read buffer, a 4 KiB output buffer, two STATE payloads). The STATE carries a `String`
      field on purpose, so the free must size the block from the type rather than a
      constant (a STATE record inlines its Strings).
- [x] **Confirmed it fails before Phase 2 and passes after — measured, same fixture,
      20 000 cycles:**

      | | peak RSS |
      |---|---|
      | reclaim disabled | **961 MB** |
      | reclaim enabled | **31 MB** |

      ~48 KiB/cycle retained → ~1.1 KiB/cycle. Retention no longer scales with the I/O a
      resource did, which is the sub-plan's single outcome. (The ~1.1 KiB/cycle residual is
      the by-design 80-byte tombstones — 160 B for the two records — plus arena free-list
      bookkeeping; it is flat per resource, not per byte of I/O.)

      **The first version of this fixture was wrong and is worth recording**: it opened one
      file `"readWrite"` and let the buffered write land at EOF, so the file grew a byte per
      cycle and `fs::readLine` returned an ever-longer String. It "leaked" ~27 KiB/cycle
      *with the reclaim working perfectly* — it was measuring file content, not retention.
      A leak test that grows its own input measures the input. Both files are now
      fixed-size (the reader only reads; `"write"` truncates).
- [x] Regenerate codegen goldens; confirm the delta is only the added frees. **The
      artifact gate shows 0 diffs across 1141 goldens** — no in-tree fixture carries a
      `-ncode`/`-nir` golden for a resource-dropping function, so the drop-path frees are
      invisible to it. The Compatibility note above predicted "every resource-using fixture
      moves"; that was wrong, and the *runtime* suite is what actually exercises this
      (which is how the `net::` segfault was caught).
- [x] Tighten §15's "drops or is closed" → "drops", per §4. Also documented the
      close-releases-the-handle / drop-reclaims-memory split and `ErrResourceMoved`.

Acceptance: the leak test fails on Phase 1's tree and passes on Phase 2's; golden delta is
exactly the drop-path frees; full suite green.
Commit: —

## Validation Plan

- Tests: the loop/leak fixture; the moved-flag rt-error fixture; the `.state`-after-close
  guard (proves §4's choice); `resource-state-drop-valid` unchanged.
- Runtime proof: **required, and it is the whole point.** Only running a loop and watching
  arena high-water proves reclamation (`.ai/compiler.md` runtime completion gate). A build
  assertion proves nothing here.
- Doc sync: `src/docs/spec/language/15_resource-management.md` — the drop/close split (§4)
  and `ErrResourceMoved` in the error tables.
- Acceptance: `scripts/test-accept.sh`, `scripts/artifact-gate.sh` (delta = the drop-path
  frees only), `cargo test --bin mfb`.

## Open Decisions

- **Is `.state` after an explicit close legal?** **Kept legal** (§4's assumption), and §15
  now says so explicitly rather than leaving it to inference: an explicit close releases the
  OS handle but reclaims no memory, so `s.state` still reads its payload; drop is what
  frees. Free-at-drop cost nothing real.
- **Should the record itself ever be reclaimed?** Not here. Retention is 80 bytes ×
  resources-ever-opened, per thread, bounded by arena teardown. A thread opening 10M
  resources holds 800 MB. If that is ever a real workload, the LUT (§3, rejected) with a
  generation counter bounds it by *peak concurrency* instead. Revisit only with a real
  workload.

  **Measured residual, for whoever revisits:** ~1.1 KiB per cycle in the leak fixture, of
  which the two 80-byte tombstones are 160 B. The rest is arena bookkeeping/fragmentation,
  not a leak of the reclaimed blocks — the 16 KiB and 4 KiB buffers are demonstrably gone
  (they would have shown up as ~500 MB at 20 000 cycles; the measured total is 31 MB). The
  important property holds: retention is flat **per resource** and no longer scales with
  how much I/O each one did. Anyone chasing the residual should look at the arena's
  free-list reuse, not at this sub-plan's frees.
- **Does `thread::transfer` free the sender's blocks?** **RESOLVED: no, and doubly so.**
  Confirmed by reading `copy_resource_to_current_arena`: the transfer copies the fd, the
  flag word, and the STATE **pointer** into a fresh record in the receiver's arena (the
  buffers are zeroed, not copied), so the payload the receiver now reads still lives in the
  sender's arena. Two independent things stop the sender freeing it:
  1. `deactivate_resource_cleanup` removes the sender's cleanup at **compile time** on the
     success path, so the sender's drop does not run at all; and
  2. Phase 2's reclaim tests `RESOURCE_MOVED_BIT` and skips, so the free is suppressed even
     if that path is ever reached.
  The second is redundant today and deliberately kept: it makes the invariant a property of
  the reclaim code rather than of every caller that might later register a cleanup.
  `thread-transfer-state-rt` still prints `99`.

  The cross-arena pointer this exposes (the receiver's record pointing at the sender's
  arena) is a **pre-existing** issue, out of scope here and recorded in `bugs/bug-257`.

## Summary

Two small, independent changes with one real trap each: the close must run **before** the
buffer free (or buffered data is stranded), and the sender's moved record must **not** free
blocks the receiver now owns. The 80-byte record stays deliberately — it is the tombstone
that makes re-close idempotent, and freeing it was rejected along with the LUT that would
have made freeing safe. Everything reclaimed here is reachable only through the record, so
nulling-as-we-free gives once-only for free, with no aliasing analysis.
