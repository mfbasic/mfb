# plan-58-B: marshal `OUT CBuffer` — allocate, hand over, truncate

Last updated: 2026-07-19
Effort: medium (1h–2h)
Depends on: plan-58-A (the `CBuffer` vocabulary, `BUFFER … SIZE`, and the
position rules); **plan-57** (`emit_alloc_list` and the `kind = 2` byte-list
representation — see §2.1)

Makes `OUT CBuffer` actually work: the thunk allocates a `List OF Byte` of the
declared byte capacity, hands the C function a pointer to its data region, and on
return sets the list's length from a `LENGTH <expr>` clause, clamped to the
capacity.

The single behavioral outcome: a LINK wrapper declaring an `OUT CBuffer` returns
a `List OF Byte` holding exactly the bytes the C callee wrote, with a length the
callee determined — proven by a runtime test that reads a file through libc
`read(2)` and gets back the file's real contents.

References (read first):

- `planning/old-plans/plan-50-E-struct-slot-marshaling.md` — the precedent for
  adding a slot kind that stages storage and marshals it back.
- `src/target/shared/code/link_thunk.rs` — the whole thunk. Specifically:
  frame layout `:336-415` (`out_base` at `:359`, `n_out` counted at `:348-352`
  and rebuilt in `expr_offsets` at `:728-747` — **these must stay in step**, per
  the warning comment at `:340-342`); scalar-OUT staging `:564-575`; CSTRUCT
  staging `:518-562`; argument register load + call `:659-703`; OUT-slot result
  marshaling `:805-859`; `emit_copy_cstring_to_string` `:1280-1359`; the
  register-lifetime doc `:1662-1672`; `LINK_EXPR_VREG_BASE = 64` `:1366`; the
  external-ABI register budget check `:659-675`.
- **The byte-list allocator to reuse:** `emit_alloc_list` and
  `emit_collection_data_pointer_into`, both introduced by plan-57-B. Do not
  hand-roll a header write; plan-57-B exists partly because there were three
  copies of that routine.
- `src/target/shared/code/error_constants.rs:762-779` (`COLLECTION_HEADER_SIZE`
  = 40, `COLLECTION_ENTRY_SIZE` = 40, the `COLLECTION_OFFSET_*` table), `:815`
  (`COLLECTION_TYPE_BYTE = 7`), `:582` (`_mfb_arena_alloc`), `:587`
  (`_mfb_arena_free`).
- `src/docs/spec/memory/05_collections.md:24-100` (the block layout),
  `:173-198` (**Capacity Headroom** — the rule that decides §4.4).
- `planning/plan-57-D-kind-2-drop-the-entry-table.md` — the `kind = 2` layout
  this sub-plan allocates into, and `plan-57-C` for the index-order guarantee.
- `src/target/shared/code/fs_helpers_io.rs:1956` (`lower_fs_read_all_bytes_helper`),
  especially `:2057-2114` — preallocate, then `read(2)` **directly into the data
  region**. This is the closest structural precedent to this sub-plan.
- `.ai/compiler.md` — Hard Completion Gate **and** the Native Codegen Register
  Lifetimes section. Both bind here.

## 1. Goal

- An `OUT CBuffer` slot lowers: the thunk allocates a `List OF Byte` sized by the
  `BUFFER … SIZE` expression, passes its data-region pointer as the C argument,
  and produces the list as the wrapper's result.
- A new `LENGTH <expr>` clause on `RETURN` sets the produced list's final length;
  without one the length is the full capacity. The value is clamped to
  `[0, capacity]` — a callee returning a negative count or one exceeding the
  buffer cannot produce a list that reads out of bounds.
- `IrLinkExpr` gains integer `*`, `+`, `-` so a callee's element/frame count can
  be scaled to bytes (`LENGTH got * 2`).
- A runtime allocation failure routes to the thunk's existing `alloc_fail` block
  and surfaces `ErrOutOfMemory`, not a crash.
- `every_known_ctype_lowers` covers `CBuffer` for real (the plan-58-A exclusion is
  removed).
- Runtime proof: a test binds libc `read(2)` through an `OUT CBuffer` and reads
  back a file it just wrote, byte-for-byte, including the **short-read** case
  where the callee writes fewer bytes than the capacity.

### Non-goals (explicit constraints)

- **No new arena or collection primitive.** The block layout, the header field
  offsets, and `_mfb_arena_alloc`'s contract are unchanged.
- **Do not change how any existing ctype marshals.** `scripts/artifact-gate.sh`
  must show every existing thunk byte-identical.
- No input (`IN`/`INOUT`) buffer direction — still rejected by plan-58-A.
- No change to `audio::write`, `fs::readAllBytes`, `net` recv, or any other
  byte-list producer or consumer.
- The byte-list representation itself is **plan-57's** work, not this sub-plan's.
  Consume `emit_alloc_list`; do not reshape it.

## 2. Current State

**Every ABI slot today is one 8-byte frame word.** The thunk's frame
(`link_thunk.rs:336-415`) is laid out once, up front: `cslot_base` gives each slot
a word holding the value-or-address handed to the C call, and `out_base` gives
each writes-back slot a word of storage. A scalar `OUT` slot zeroes its word,
takes its address, and stores that address into its cslot (`:564-575`). A CSTRUCT
slot does the same with a sized, aligned, fully-zeroed frame buffer
(`:518-562`). Both are **frame** storage, sized at compile time.

`CBuffer` breaks that assumption in two ways: its size is a runtime value, and its
storage must **outlive the call** — it becomes the returned MFBASIC value. So it
cannot live in the frame at all; it must be an arena block, and the frame holds
only the pointer.

### 2.1 The byte-list block, after plan-57

`List OF Byte` is a fixed-width-scalar list, so plan-57-D gives it the
`kind = 2` representation — one contiguous arena block with **no lookup table**:

```
CollectionHeader   40 bytes    count@8 capacity@16 dataLength@24 dataCapacity@32
Data[dataCapacity] 1 byte each
```

`dataBase = block + 40`, constant. Element `i` is at `dataBase + i`, and
plan-57-C guarantees index order physically, so a pointer to `dataBase` handed to
a C function addresses exactly the bytes the list logically holds.

**plan-57 is a strong preference here, not a correctness prerequisite.** An
`OUT CBuffer` allocates a *fresh* list and the callee fills its data region
front-to-back, so the produced value is ordered by construction and is correct
under either representation — bug-365 cannot reach it. What plan-57 buys is the
41× memory drop, `emit_alloc_list` (without which this sub-plan hand-rolls a
header write and becomes the fourth copy of that routine), and a `dataBase` that
is a constant offset instead of a `capacity`-scaled computation.

If plan-58 must ship first, it can — at 41× memory, with a hand-rolled header,
and with `CBUFFER_MAX_BYTES` at 8 MiB instead of 64 MiB. Say so in the code
comments so the constants stay traceable.

**The register-lifetime rule.** `_mfb_arena_alloc` destroys every caller-saved
register (`x0`–`x17`) with no survivor set (`.ai/compiler.md`;
`link_thunk.rs:1662-1672`). Existing code spills across it structurally:
`emit_copy_cstring_to_string` stores the strlen result to the freed status slot
before allocating and reloads after (`:1311`, `:1317`); `marshal_struct_out`
allocates the record *first*, spills the pointer, and reloads it **per field**
(`:1800`, `:1823`, `:1897`).

**The precedent that matches exactly.** `fs::readAllBytes`
(`fs_helpers_io.rs:2057-2114`) preallocates a byte list at a known size, computes
the data base, and hands that pointer straight to `read(2)`. That is this
sub-plan's shape, with a LINK thunk in place of a builtin lowering.

## 3. Design Overview

Four pieces, layered:

1. **Allocate through `emit_alloc_list`** — the shared constructor plan-57-B
   introduced. Do not hand-roll a header write here; there were three copies of
   that routine before plan-57 and this sub-plan would have been the fourth.
2. **Frame layout.** `CBuffer` consumes one `out_base` word — the *pointer* to
   the allocated block, not the data. It counts in `n_out` (`:348-352`) and in
   `expr_offsets` (`:728-747`); these two must agree or every `SUCCESS_ON`
   variable after the buffer resolves to the wrong slot.
3. **Staging** (before the call): evaluate `SIZE`, allocate via
   `emit_alloc_list`, compute `dataBase`, store the **block** pointer to the out
   word and the **dataBase** pointer to the cslot word.
4. **Truncation** (after the call): evaluate `LENGTH`, clamp, store to `count` and
   `dataLength`.

**Where the correctness risk concentrates — three places, all subtle:**

- **Register lifetimes.** This is the first slot kind that allocates *during*
  staging, in the middle of a loop that is also computing other slots' values. The
  allocation destroys `x0`–`x17`, so any partially-staged slot value in a
  caller-saved register is lost. Mitigation: allocate into frame slots only, and
  run the `CBuffer` staging as a **separate pass before** the main slot loop, so
  no other slot's value is live across it. Do not interleave.
- **`dataBase` vs block pointer.** Two different pointers, 40 bytes apart under
  `kind = 2`. The C function gets `dataBase`; the result marshal and the
  truncation get the block. Swapping them makes the callee overwrite the header —
  and because the gap is now one header rather than a whole entry table, a short
  write may corrupt only `count`/`capacity` and still look plausible. Test with a
  callee that fills the buffer completely.
- **The clamp.** `sf_read_short` returns `-1` on error and `0` at EOF; `read(2)`
  returns `-1`/`0`. An unclamped negative length stored to `count` is a huge
  unsigned value, and every later collection read walks off the block.

**Rejected alternative:** *realloc-tight on truncation* — allocate at capacity,
then allocate a second block at the final length, copy, and free the first. This
is what the audio partial-read paths do (`alsa.rs:1615-1675`,
`macos.rs:2240`). Rejected here: it doubles peak memory at exactly the moment the
buffer is largest, and `05_collections.md:173-198` explicitly sanctions
`capacity > count` as headroom, with `emit_flat_block_size`
(`builder_collection_layout.rs:214-227`) sizing from `capacity`/`dataCapacity` so
`arena_free` stays correct. See §4.4 for the full argument and its one caveat.

**Rejected alternative:** *let the wrapper return the byte count and take the
buffer as a second output*. Rejected: the ABI surfaces exactly one result
(`17_native-libraries.md`, "Multiple outputs — not implemented"), and the count
without the bytes is useless.

## 4. Detailed Design

### 4.1 `LENGTH` and expression arithmetic

```
returnCl := "RETURN" linkExpr [ "LENGTH" linkExpr ]
```

`LENGTH` is valid only when `RETURN` names a `CBuffer` slot (else
`NATIVE_BUFFER_INVALID`, plan-58-A's rule). Its value is in **bytes**.

`IrLinkExpr` (`src/ir/link.rs:514-534`) currently carries `Var`, comparisons, and
`And`/`Or`/`Not`. Add `Mul`/`Add`/`Sub` over integers, evaluated by the existing
expression emitter using the `LINK_EXPR_VREG_BASE = 64` vreg range
(`link_thunk.rs:1366`). This is what makes `LENGTH got * 2` (items → bytes) and
`SIZE items * 2` expressible; without it, a `CBuffer` can only ever be sized and
truncated in raw bytes, which no real C audio API speaks.

Encode the new variants in the existing `IrLinkExpr` wire encoding
(`src/ir/binary.rs`); plan-58-C owns the format question.

### 4.2 Staging (a pass before the main slot loop)

For each `CBuffer` slot, in declaration order:

1. Evaluate the `SIZE` expression → `N` (bytes). Store `N` to a frame scratch
   word; it must survive the allocation.
2. **Runtime size gate** (§Open Decisions): if `N < 0` or `N > CBUFFER_MAX_BYTES`,
   branch to a new `buffer_size_fail` block raising `ErrInvalidArgument`. A
   negative `N` would otherwise compute a nonsense block size.
3. Call `emit_alloc_list("Byte", N, ...)` (plan-57-B), which sizes
   `COLLECTION_HEADER_SIZE + N` for the `kind = 2` representation, allocates,
   and writes the header — `kind = COLLECTION_KIND_LIST_FIXED`,
   `valueType = COLLECTION_TYPE_BYTE`, and
   `count`/`capacity`/`dataLength`/`dataCapacity` all `= N`. There is **no entry
   table and no entry-fill loop** for a fixed-width element type (plan-57-D). It
   branches to the existing `alloc_fail` label on failure
   (`link_thunk.rs:1020` always emits it).
4. Spill the block pointer to its `out_base` word **immediately** —
   `_mfb_arena_alloc` has destroyed every caller-saved register
   (`.ai/compiler.md`).
5. `dataBase = block + COLLECTION_HEADER_SIZE` — a constant offset under
   `kind = 2`. Store it to the slot's **cslot** word, so the generic
   argument-register loop (`:686-692`) passes it unchanged. Use
   `emit_collection_data_pointer_into` (plan-57-B) rather than open-coding the
   arithmetic.

`count`/`dataLength` are initialized to `N` by the allocator so that a wrapper
with **no** `LENGTH` clause needs no post-call work and the value is well-formed
even if the callee writes nothing.

Note how much plan-57 removes from this sub-plan: without it, steps 3–5 were an
overflow-checked `40 + 41N` size computation, a hand-written 8-field header, an
`N`-iteration entry-fill loop, and a `block + 40 + N*40` data-base computation —
all inside a thunk where every value must be spilled across the allocation. The
`kind = 2` representation reduces the whole staging sequence to one helper call
and one add.

A `CBuffer` slot is an **integer** argument slot: it counts against
`external_int_argument_registers` in the budget check at `:659-675` (6 on
x86-64 SysV, 8 elsewhere).

### 4.3 The result marshal

`RETURN <cbuffer-slot>` sets `result_out_off` to the slot's `out_base` word and
`result_out_ctype` to `"CBuffer"` (`link_thunk.rs:511-513`, `:571-574`). Add a
`"CBuffer"` arm to the OUT-slot result match at `:805-859`:

```
load_u64(RESULT_VALUE_REGISTER, sp, result_out_off)   ; the block pointer
```

That arm's `_` default is a silent raw 8-byte load, which would coincidentally do
the right thing here — **add the explicit arm anyway**, with a comment. Relying on
a silent default is how bug-238 happened (that default is why a `CInt32` OUT
surfaced `-1` as `4294967295`).

Note `CBuffer` never reaches `emit_return_passthrough` (`:1121-1220`), because it
is a slot, not the C return. Its `Err` default arm therefore stays correct and
untouched.

### 4.4 Truncation

After the call and after the `SUCCESS_ON` gate, if a `LENGTH` expression exists:

1. Evaluate it → `k`.
2. Clamp: `k < 0 → 0`; `k > N → N`. Reload `N` from its frame scratch word.
3. Reload the block pointer from the `out_base` word.
4. `store_u64(k, block, COLLECTION_OFFSET_COUNT)` and
   `store_u64(k, block, COLLECTION_OFFSET_DATA_LENGTH)`.

`capacity` and `dataCapacity` stay at `N`. That is the load-bearing decision, and
it is safe on three independent grounds:

- **Reads**: under `kind = 2` the data base is `block + 40` and element `i` is at
  `dataBase + i` — neither depends on `capacity` or `count`, so leaving the slack
  in place cannot mis-address anything.
- **Free**: `emit_flat_block_size` sizes a `kind = 2` block as
  `HEADER + dataCapacity` (plan-57-D §4.3), so leaving `dataCapacity = N` is
  exactly what makes `arena_free` reclaim the whole block. Setting it to `k`
  would leak `N - k` bytes.
- **Spec**: `capacity > count` is sanctioned headroom
  (`05_collections.md:173-198`), and a value-semantic copy is shrink-to-fit
  (`copy_collection_tight`), so the slack is erased the first time the list is
  copied.

**The one caveat, stated so it is not discovered later.** `:185-198` frames
headroom as a property of a *mutable working buffer*, not of a value, and
`:191-193` says known-size builders allocate exactly. A short-read `CBuffer`
returns a value carrying slack, which is a small departure from that guidance.
Accepted here because the alternative — realloc tight and copy, as the audio
partial-read paths do (`alsa.rs:1615-1675`) — doubles peak memory precisely when
the buffer is largest. Under `kind = 2` the slack is at most `N - k` **bytes**
rather than `41(N - k)`, so the case for tolerating it is stronger than it was,
and the case for the copy weaker.

The bytes between `k` and `N` are uninitialized arena memory. They are never read:
copies are `count`-tight and every consumer bounds by `count`. No information
leak, because the arena is process-private — but do **not** relax this without
rechecking.

## Compatibility / Format Impact

- **Changes:** `OUT CBuffer` becomes usable. `IrLinkExpr` gains three variants,
  and `IrLinkFunction` gains the `LENGTH` expression — both on the `.mfp` wire
  (plan-58-C).
- **Unchanged:** the collection block layout and every header offset;
  `_mfb_arena_alloc`/`_mfb_arena_free` contracts; every existing ctype's
  marshaling; `emit_return_passthrough`; `audio::write` and every other byte-list
  consumer. `scripts/artifact-gate.sh` must show existing thunks byte-identical,
  **including** after the §3 deduplication.

## Phases

### Phase 1 — `IrLinkExpr` arithmetic

- [ ] Add `Mul`/`Add`/`Sub` to `IrLinkExpr` (`src/ir/link.rs:514-534`), the AST
      counterpart, the parser, and the thunk's expression emitter
      (`LINK_EXPR_VREG_BASE` range, `link_thunk.rs:1366`).
- [ ] Tests: unit tests for expression lowering; a syntax fixture using
      `SUCCESS_ON status * 2 = 4` to prove arithmetic evaluates before the
      `CBuffer` feature depends on it.

Acceptance: a LINK wrapper with an arithmetic `SUCCESS_ON` gates correctly at
runtime (extend an existing `tests/rt-behavior/native/` test rather than adding a
libsndfile dependency this early).
Commit: —

### Phase 2 — `CBuffer` staging, marshal, and truncation (highest risk)

- [ ] Frame layout: count `CBuffer` slots in `n_out` (`link_thunk.rs:348-352`) and
      in the `expr_offsets` rebuild (`:728-747`). Add an assertion or a shared
      helper so the two cannot drift — the comment at `:340-342` warns about
      exactly this.
- [ ] Add the `CBuffer` staging pass **before** the main slot loop (§4.2),
      including the size gate and `buffer_size_fail` block.
- [ ] Add the `"CBuffer"` arm to the OUT-slot result match (`:805-859`), §4.3.
- [ ] Add the truncation sequence after the `SUCCESS_ON` gate (§4.4).
- [ ] Remove plan-58-A's `CBuffer` exclusion from
      `every_known_ctype_lowers` (`link_thunk.rs:2000-2085`) and add the
      writes-back loop that sub-plan's notes call for — the existing loop 2 uses
      `AbiDirection::In` + a `CONST` pin, which a `CBuffer` can never satisfy.
- [ ] Spec: replace plan-58-A's *"declared but not yet lowered"* status note in
      `src/docs/spec/language/17_native-libraries.md` with the real semantics —
      byte capacity, `LENGTH` clamping, the `capacity > count` headroom
      consequence, and the runtime size cap.
- [ ] Tests: `tests/rt-behavior/native/native-buffer-read-rt/` — bind libc
      `read(2)` (`ABI (fd CInt32, buf OUT CBuffer, n CInt64) AS got CInt64`,
      `BUFFER buf SIZE n`, `RETURN buf LENGTH got`), write a known file with
      `fs::writeAllBytes`, open it via libc `open`, read it back, and compare
      byte-for-byte. `libc`/`libSystem.B.dylib` is already wired as a test library
      (`tests/rt-behavior/native/native-struct-scalar-rt/project.json`).
- [ ] Tests: the **short-read** case in the same fixture — request a capacity far
      larger than the file, assert the returned list's length equals the file
      size, not the capacity. This is the case the clamp and the `count` write
      exist for, and the one a full-buffer test cannot see.
- [ ] Tests: a zero-length read (EOF) returns an empty list, not a crash.
- [ ] Tests: `LENGTH` scaling — a wrapper using `LENGTH got * 2` returns twice the
      callee's count (exercises Phase 2 through the buffer path).

Acceptance: `native-buffer-read-rt` prints the exact contents of a file it wrote,
in all four cases (exact fit, short read, EOF, scaled `LENGTH`), on
macOS/aarch64 and Linux/{aarch64,x86_64,riscv64} × {glibc,musl}. `artifact-gate`
shows no change to any pre-existing thunk. `scripts/test-accept.sh` green.
Commit: —

## Validation Plan

- Tests: `tests/rt-behavior/native/native-buffer-read-rt/` (four runtime cases
  above); `every_known_ctype_lowers` extended with a writes-back loop; unit tests
  for `IrLinkExpr` arithmetic; the plan-58-A negative fixtures must still reject.
- Runtime proof: **required (Hard Completion Gate).** Reading a real file's real
  bytes back through an `OUT CBuffer` is the proof; goldens and IR dumps are not.
  Run on every target per `.ai/remote_systems.md` — the frame layout and the
  argument-register budget differ across them, and x86-64's 6-register SysV limit
  is the one most likely to bite.
- Register-lifetime review: per `.ai/compiler.md`, walk every value live across
  the `_mfb_arena_alloc` call in §4.2 and confirm it is in a frame slot, not a
  register. Note that this bug class *passes small tests* — a corrupted length
  only faults past a threshold. Test with a buffer large enough to span multiple
  entry-loop iterations, and with N in the megabytes.
- Doc sync: `src/docs/spec/language/17_native-libraries.md` (the `CBuffer`
  semantics replacing the plan-58-A status note). No new runtime error code unless
  the size gate mints one — if it does, `02_error-codes.md` is build input and
  must be updated in the same change.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` and
  `scripts/artifact-gate.sh`.

## Open Decisions

- **The runtime size cap `CBUFFER_MAX_BYTES`.** With plan-57 landed a
  `List OF Byte` costs `40 + N` bytes rather than `40 + 41N`, so a `CBuffer` is
  no longer the memory hazard it was: 3 minutes of stereo 48 kHz PCM is 34.6 MB
  allocated, not 1.4 GB. A cap is still wanted — the size is a caller-supplied
  runtime value and an unbounded one is an allocation primitive — but it can be
  set for sanity rather than for survival. Recommend **64 MiB**, raising
  `ErrInvalidArgument` naming the requested size. Alternative: no cap, and let
  `_mfb_arena_alloc` fail — still rejected, because the failure would surface as
  `ErrOutOfMemory` at a size the programmer never wrote.
  **If plan-57 has not landed, use 8 MiB instead** (≈344 MB allocated at 41×) and
  say why in the code comment, so the number is traceable to the representation
  rather than looking arbitrary.
- **Realloc-tight instead of headroom on truncation?** Recommend headroom (§4.4).
  Revisit only if a reviewer objects to returning a value with slack; the
  realloc-tight implementation already exists at `alsa.rs:1615-1675`.

## Summary

The engineering risk is concentrated in three places: register lifetimes across
the first mid-staging `_mfb_arena_alloc` in the thunk (mitigated by a separate
staging pass and frame-only storage), the `dataBase`-vs-block pointer distinction
(two pointers that are easy to swap and silently corrupting when swapped), and
the length clamp (without which a `-1` from the callee becomes a huge unsigned
`count`). Each has a dedicated runtime case in `native-buffer-read-rt`.

Untouched: the collection block layout, the arena contracts, and every existing
ctype's marshaling — enforced by `artifact-gate` byte-identity, including across
the Phase 1 refactor.
