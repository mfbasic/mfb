# plan-58-B: marshal `OUT CBuffer` — allocate, hand over, truncate

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: plan-58-A (the `CBuffer` vocabulary, `BUFFER … SIZE`, the position
rules). Feature-wide precondition: **plan-57 complete** — plan-58-A §Prerequisite.
Produces: the `CBuffer` staging pass, the `"CBuffer"` result-marshal arm, the
`LENGTH` clause, `IrLinkExpr::{Mul,Add,Sub}`, `CBUFFER_MAX_BYTES`. Consumed by C
(wire format) and D (the binding).

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
- `emit_alloc_list` and `emit_collection_data_pointer_into` — plan-57-B's shared
  byte-list constructor and data-pointer helper. This sub-plan consumes both;
  the precondition (§0) guarantees they exist and are `pub(crate)`.
- `src/target/shared/code/error_constants.rs:777` (`COLLECTION_HEADER_SIZE` = 40),
  `:786` (`COLLECTION_ENTRY_SIZE` = 40), the `COLLECTION_OFFSET_*` table, `:815`
  (`COLLECTION_TYPE_BYTE` = 7), `:582` (`_mfb_arena_alloc`), `:587`
  (`_mfb_arena_free`).
- `src/target/shared/code/builder_collection_layout.rs:241`
  (`emit_flat_block_size` — stride-parameterized via `list_entry_stride`, so it
  is already correct for `kind = 2`).
- `src/docs/spec/memory/05_collections.md:24-100` (block layout), `:173-198`
  (**Capacity Headroom** — the rule that decides §4.4).
- `src/target/shared/code/fs_helpers_io.rs:1956`
  (`lower_fs_read_all_bytes_helper`), especially `:2057-2114` — preallocate, then
  `read(2)` **directly into the data region**. The closest structural precedent.
- `.ai/compiler.md` — Hard Completion Gate **and** the Native Codegen Register
  Lifetimes section. Both bind here.

## 0. Precondition

plan-58's single hard stop is **plan-57 complete**, checked once at the feature's
entry gate — see `plan-58-A` §Prerequisite. It is not re-litigated here and there
is no separate blocker in this sub-plan.

What that precondition guarantees, and what everything below is written against:

- ~~`emit_alloc_list` and `emit_collection_data_pointer_into` exist and are
  `pub(crate)`~~ **Corrected 2026-07-20:** those two NAMES never existed — plan-57
  declined to mint them because they would have had no callers. The *capability*
  is present under different names, and at sufficient visibility:
  `crypto_ec::emit_build_byte_list` / `audio::emit_alloc_byte_list` construct a
  byte list, and `push_collection_data_pointer_into` /
  `emit_collection_data_pointer_for` give a data pointer. All are `pub(super)`
  within `target::shared::code`, which is where `link_thunk.rs` lives, so **no
  visibility widening is needed**. This sub-plan consumes them; it does not build,
  promote, or port them.
- `kind = 2` is the live, ungated representation: a byte-list block is `40 + N`,
  `dataBase = block + 40` is a constant offset, and there is no entry table.
- There is therefore exactly **one** representation to target. No `MFB_KIND2`
  branch, no dual-mode staging, no 41×-cost fallback.

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
- A `SIZE` that is negative or exceeds `CBUFFER_MAX_BYTES` raises
  `ErrInvalidArgument` before allocating.
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
- **Do not touch the byte-list representation, the `MFB_KIND2` gate, or any
  plan-57 deliverable.** plan-57 is a precondition (§0), not work this sub-plan
  finishes, ports, or works around.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| `COLLECTION_HEADER_SIZE` | 40 | `error_constants.rs:777` |
| `COLLECTION_ENTRY_SIZE` | 40 | `error_constants.rs:786` |
| External int arg registers (budget check) | 6 x86-64 SysV, 8 elsewhere | `link_thunk.rs:659-675` |

### 2.2 How a slot works today

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

### 2.3 The representation this lands into

One representation, guaranteed by the precondition (§0): `kind = 2`.

| | `kind = 2` |
|---|---|
| Block size for `N` bytes | `40 + N` |
| `dataBase` | `block + 40` — a constant offset |
| Arena cost of 64 MiB | 64 MB (1.0×) |
| Entry-fill loop on alloc | none |

The 2026-07-19 draft tried to be correct under both the old and new layouts,
hedging in its §2.1 that plan-57 was "a strong preference, not a correctness
prerequisite" while writing §4.2 and §4.4 only for `kind = 2`. That
contradiction is gone: plan-57 is a precondition, so there is nothing to hedge.
Do not reintroduce a capacity-scaled data-base computation "just in case" — if
`dataBase` is not `block + 40`, the precondition was not met.

### 2.4 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| `emit_alloc_list` exists | **FALSE** (name) — capability CONFIRMED | 0 hits. plan-57 declined the name (no callers, AGENTS.md bans dead code). `crypto_ec.rs:215 emit_build_byte_list` and `audio/mod.rs:135 emit_alloc_byte_list` are the real constructors |
| `emit_collection_data_pointer_into` exists | **FALSE** (name) — capability CONFIRMED | 0 hits. `builder_collection_layout.rs:2179 push_collection_data_pointer_into` and `:1935 emit_collection_data_pointer_for` |
| A byte-list constructor is callable from `link_thunk.rs` | ~~**FALSE**~~ **CONFIRMED 2026-07-20** | The draft assumed `pub(super)` was too narrow. It is not: `pub(super)` in `target/shared/code/*` means visible throughout `target::shared::code`, and `link_thunk.rs` **is** `target::shared::code::link_thunk`. **No visibility widening is required** — plan-58-A §Prerequisite flagged this as the thing to confirm in B, and it confirms clean |
| `kind = 2` is live | ~~**PRECONDITION**, not met~~ **CONFIRMED 2026-07-20** | plan-57 landed the flip. `kind2_enabled()` (`builder_collection_layout.rs:2275`) is a plain `true`; no env read anywhere in the file |
| `_mfb_arena_alloc` destroys all caller-saved registers | **CONFIRMED** | `.ai/compiler.md`; spill pattern read at `link_thunk.rs:1311`, `:1317`, `:1800`, `:1823`, `:1897` |
| `alloc_fail` label is always emitted | **CONFIRMED** | `link_thunk.rs:1020` |
| `emit_flat_block_size` sizes from capacity/dataCapacity | **CONFIRMED** | `builder_collection_layout.rs:241` — so leaving slack is what makes `arena_free` correct |
| `emit_flat_block_size` is correct under **both** representations | **CONFIRMED** | It calls `list_entry_stride(&element)` (`builder_collection_layout.rs:~266`), which returns 0 under `kind = 2`. It is *not* hardcoded to `COLLECTION_ENTRY_SIZE`. A review pass claimed otherwise and that plan-57-D's `[x]` for this site was stale — **that claim is false**; checked before acting on it. This matters because plan-57-D names this as the one site whose failure mode is arena free-list corruption (bug-02 class) rather than wrong data |
| The OUT-result `_` arm is a silent raw 8-byte load | **CONFIRMED** | `link_thunk.rs:805-859`; this is the bug-238 mechanism |
| Allocating mid-staging preserves other slots' values | **UNVERIFIED — this is Phase 1** | no precedent: no existing slot kind allocates during staging |

That last row is the design uncertainty in this sub-plan, and it is why Phase 1
is a spike rather than a refactor.

## 3. Design Overview

Four pieces, layered:

1. **A callable constructor** — ~~resolve §0 first~~ **already resolved**: the
   constructors and the data-pointer helpers exist and are reachable from
   `link_thunk.rs` without a visibility change (§2.4). Nothing to do here.
2. **Frame layout.** `CBuffer` consumes one `out_base` word — the *pointer* to
   the allocated block, not the data. It counts in `n_out` (`:348-352`) and in
   `expr_offsets` (`:728-747`); these two must agree or every `SUCCESS_ON`
   variable after the buffer resolves to the wrong slot.
3. **Staging** (before the call): evaluate `SIZE`, gate it, allocate, compute
   `dataBase` *via the helper, not by adding 40*, store the **block** pointer to
   the out word and the **dataBase** pointer to the cslot word.
4. **Truncation** (after the call): evaluate `LENGTH`, clamp, store to `count` and
   `dataLength`.

**Where design uncertainty concentrates — one place.** No existing slot kind
allocates *during* staging. The allocation destroys `x0`–`x17` in the middle of a
loop that is also computing other slots' values. Whether the mitigation (a
separate pass before the main slot loop, frame-only storage) actually holds is
unproven, and if it does not, the shape of this sub-plan changes. **Phase 1
falsifies this cheaply, before any of the rest is built.**

**Where correctness risk concentrates — two places, both after Phase 1:**

- **`dataBase` vs block pointer.** Two different pointers. The C function gets
  `dataBase`; the result marshal and the truncation get the block. Swapping them
  makes the callee overwrite the header. The gap is 40 bytes, so
  a short write may corrupt only `count`/`capacity` and still look plausible.
  Test with a callee that fills the buffer completely.
- **The clamp.** `sf_read_short` returns `-1` on error and `0` at EOF; `read(2)`
  returns `-1`/`0`. An unclamped negative length stored to `count` is a huge
  unsigned value, and every later collection read walks off the block.

**Rejected alternative:** *realloc-tight on truncation* — allocate at capacity,
then allocate a second block at the final length, copy, and free the first. This
is what the audio partial-read paths do (`alsa.rs:1615-1675`, `macos.rs:2240`).
Rejected: it doubles peak memory at exactly the moment the buffer is largest, and
`05_collections.md:173-198` sanctions `capacity > count` as headroom, with
`emit_flat_block_size` (`builder_collection_layout.rs:241`) sizing from
`capacity`/`dataCapacity` so `arena_free` stays correct. See §4.4 and its caveat.

**Rejected alternative:** *let the wrapper return the byte count and take the
buffer as a second output*. Rejected: the ABI surfaces exactly one result
(`17_native-libraries.md`, "Multiple outputs — not implemented"), and the count
without the bytes is useless.

**Rejected alternative:** *open-code `dataBase = block + 40`.* Rejected: correct
correct only if the precondition holds — and if it holds, `block + 40` *is*
correct (§2.3). Prefer the shared helper anyway: it keeps the offset in one
place, and costs nothing.

## 4. Detailed Design

### 4.1 `LENGTH` and expression arithmetic

```
returnCl := "RETURN" linkExpr [ "LENGTH" linkExpr ]
```

`LENGTH` is valid only when `RETURN` names a `CBuffer` slot (else
`NATIVE_BUFFER_INVALID`, plan-58-A's rule). Its value is in **bytes**.

`IrLinkExpr` (`src/ir/link.rs:515-534`) currently carries **six** variants:
`Var(String)`, **`Int(i64)`**, `Compare`, `And`, `Or`, `Not`. The 2026-07-19 draft
omitted `Int` — it already exists and `link_expr_var_names` (`:163-175`) already
has an arm for it, so this extension is smaller than the draft implied: the
integer literal is done, only the operators are new.

Add `Mul`/`Add`/`Sub` over integers, evaluated by the existing
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
2. **Runtime size gate**: if `N < 0` or `N > CBUFFER_MAX_BYTES`, branch to a new
   `buffer_size_fail` block raising `ErrInvalidArgument`. A negative `N` would
   otherwise compute a nonsense block size. `CBUFFER_MAX_BYTES` = **64 MiB**,
   which under `kind = 2` is 64 MB of arena (plan-58-A §Prerequisite).
3. Call the shared byte-list constructor (§0) with `N` and element type `Byte`.
   It writes the header; there is no entry table. It branches to the
   existing `alloc_fail` label on failure (`link_thunk.rs:1020` always emits it).
4. Spill the block pointer to its `out_base` word **immediately** —
   `_mfb_arena_alloc` has destroyed every caller-saved register.

**And the step the draft omitted:** add a `continue` guard for `CBuffer` in the
main slot loop, mirroring the CSTRUCT one at `link_thunk.rs:562`. Without it the
loop's `writes_back()` branch (`:564-575`) runs for the same slot and
**overwrites the cslot with `&out_word` and zeroes the out word that now holds
the block pointer** — the two stagings clobber each other. plan-58-A adds this
guard as a refusal (its §2.4); B converts it from `Err` to `continue`.
5. Obtain `dataBase` from the shared data-pointer helper — under `kind = 2` it
   is `block + COLLECTION_HEADER_SIZE`, a constant. Store it to the **cslot**
   word, so the
   generic argument-register loop (`:686-692`) passes it unchanged.

`count`/`dataLength` are initialized to `N` by the constructor so that a wrapper
with **no** `LENGTH` clause needs no post-call work and the value is well-formed
even if the callee writes nothing.

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
it is safe on three independent grounds — **all three hold under both
representations**:

- **Reads**: element addressing goes through the shared element-offset helper in
  the shared element-offset helper, and bounds by `count`. Leaving slack in `capacity`
  cannot mis-address anything.
- **Free**: `emit_flat_block_size` (`builder_collection_layout.rs:241`) sizes a
  block from `capacity`/`dataCapacity`, so leaving them at `N` is exactly what
  makes `arena_free` reclaim the whole block. Setting them to `k` would leak.
- **Spec**: `capacity > count` is sanctioned headroom
  (`05_collections.md:173-198`), and a value-semantic copy is shrink-to-fit
  (`copy_collection_tight`), so the slack is erased the first time the list is
  copied.

**The one caveat, stated so it is not discovered later.** `:185-198` frames
headroom as a property of a *mutable working buffer*, not of a value, and
`:191-193` says known-size builders allocate exactly. A short-read `CBuffer`
returns a value carrying slack, which is a small departure from that guidance.
Accepted here because the alternative (realloc tight and copy) doubles peak
memory precisely when the buffer is largest. Under `kind = 2` the slack is
`(N - k)` **bytes**, which is small enough that the case for tolerating it is
clear-cut.

The bytes between `k` and `N` are uninitialized arena memory. They are never read:
copies are `count`-tight and every consumer bounds by `count`. No information
leak, because the arena is process-private — but do **not** relax this without
rechecking.

## Compatibility / Format Impact

- **Changes:** `OUT CBuffer` becomes usable. `IrLinkExpr` gains three variants,
  and `IrLinkFunction` gains the `LENGTH` expression — both on the `.mfp` wire
  (plan-58-C). If §0 resolves as **(b)**, `emit_alloc_byte_list` moves module and
  gains visibility; `audio/`'s two call sites move with it.
- **Unchanged:** the collection block layout and every header offset;
  `_mfb_arena_alloc`/`_mfb_arena_free` contracts; every existing ctype's
  marshaling; `emit_return_passthrough`; `audio::write` and every other byte-list
  consumer. `scripts/artifact-gate.sh` must show existing thunks byte-identical
  after every change in this sub-plan.

## Phases

Ordered uncertainty-first: Phase 1 exists to falsify the one unproven premise
(§2.4's last row) before anything is built on it.

### Phase 1 — spike: allocate during staging (falsifies the premise)

The smallest thing that proves an arena allocation can happen mid-staging without
destroying another slot's staged value. **Do not build the rest until this passes.**

- [ ] A LINK wrapper with a fixed-size `OUT CBuffer` (`SIZE 64`, no `LENGTH`) and
      **at least two other scalar slots staged around it**, so a register-lifetime
      violation is observable rather than latent.
- [ ] Implement the separate staging pass in `link_thunk.rs`: allocate, spill the
      block to `out_base`, take `dataBase` from the helper, store to the cslot.
- [ ] Bind libc `read(2)` through it; read a file written by the test.
- [ ] Assert the *other two slots'* values are intact after the call — that is
      what this phase is actually testing.

Acceptance: the wrapper returns the file's first 64 bytes byte-for-byte **and**
both neighbouring scalar slots carry their correct values, on aarch64 and x86-64.
If the neighbouring slots are corrupted, the separate-pass mitigation is wrong and
§3 must be redesigned before proceeding.
Commit: —

### Phase 2 — `IrLinkExpr` arithmetic

- [ ] Add `Mul`/`Add`/`Sub` to `IrLinkExpr` (`src/ir/link.rs:514-534`), the AST
      parse for them, and their evaluation in the thunk expression emitter
      (`link_thunk.rs`, `LINK_EXPR_VREG_BASE` range).
- [ ] Tests: unit coverage for each operator, including a `SIZE items * 2`
      expression resolving to the right byte count.

Acceptance: a wrapper declaring `BUFFER buf SIZE items * 2` allocates exactly
`2 * items` bytes, asserted against the block header.
Commit: —

### Phase 3 — `LENGTH`, the clamp, and the size gate

- [ ] `LENGTH <expr>` on `RETURN`: parse, IR, and post-call truncation (§4.4).
- [ ] The clamp: `k < 0 → 0`, `k > N → N`.
- [ ] `CBUFFER_MAX_BYTES` (64 MiB) and the `buffer_size_fail` block raising
      `ErrInvalidArgument`.
- [ ] Remove plan-58-A's `CBuffer` exclusion from `every_known_ctype_lowers`.
- [ ] Tests: short read (callee writes fewer bytes than capacity — the list's
      `count` is the short value and its bytes are correct); callee returns `-1`
      (clamps to 0, no OOB); callee returns more than capacity (clamps to `N`);
      `SIZE` negative and `SIZE` over the cap (both `ErrInvalidArgument`);
      allocation failure routes to `ErrOutOfMemory`.

Acceptance: the libc `read(2)` runtime test passes for a full read **and** a
short read, byte-for-byte; every clamp and gate case above produces its stated
error rather than a crash or a corrupt list; `every_known_ctype_lowers` covers
`CBuffer`.
Commit: —

## Validation Plan

- Tests: as listed per phase. Negative cases (`-1`, over-capacity, negative
  `SIZE`, over-cap `SIZE`, allocation failure) are mandatory — the clamp is the
  difference between a short read and an out-of-bounds walk.
- Coverage check: LINK thunk changes are golden-backed via
  `scripts/artifact-gate.sh`. Confirm the new fixtures actually produce goldens;
  `tests/acceptance/` has **no** `golden/` dir by design, so a proof placed there
  is *not* in the gate's denominator.
- Runtime proof: the libc `read(2)` binding, full read and short read,
  byte-compared against the file the test wrote. This must run on aarch64 and
  x86-64 — the register-lifetime hazard in Phase 1 is where the two could differ.
- Doc sync: `17_native-libraries.md` (the `LENGTH` clause, `CBUFFER_MAX_BYTES`,
  and the `IrLinkExpr` operators).
- Acceptance: the project's full suite, plus `scripts/artifact-gate.sh` showing
  every pre-existing thunk byte-identical.

## Open Decisions

1. **`CBUFFER_MAX_BYTES` value.** Recommended 64 MiB — under `kind = 2` that is
   64 MB of arena, and it is what sets plan-58-D's ~5.8 min audio ceiling.
   Alternative: 16 MiB, if a single wrapper allocating 64 MB is judged too blunt
   an instrument. (§4.2)

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **`emit_alloc_list` and `emit_collection_data_pointer_into` do not
  exist**, and `kind = 2` is gated off (`builder_collection_layout.rs:2191`). The
  2026-07-19 draft named both functions as landed plan-57-B deliverables and built
  its §3 on them, while hedging in §2.1 that plan-57 was "a strong preference, not
  a correctness prerequisite" — then writing §4.2/§4.4 only for `kind = 2`.
  Both problems have the same root: **plan-57 was treated as a soft dependency.**
  It is now a hard precondition (§0, plan-58-A §Prerequisite), which deleted the
  hedge, the dual-representation staging, the 8 MiB/41× fallback, and the
  "promote `emit_alloc_byte_list` here" option along with it.
- 2026-07-20 — **Removed the two phases that existed only to bridge plan-57.**
  The interim draft had a Phase 0 (resolve/port the constructor — plan-57's work)
  and a Phase 4 (prove both representations). With plan-57 a precondition,
  neither is this sub-plan's business.
- 2026-07-20 — **Phase order inverted.** The draft ran `IrLinkExpr` arithmetic
  (inert) first and "staging, marshal, truncation (highest risk)" second. The
  unproven premise is allocation-during-staging, so that is now a Phase 1 spike;
  arithmetic moved to Phase 2.

## Summary

The real engineering risk is a single unproven premise — that a thunk can
allocate mid-staging without destroying neighbouring slots' values — and Phase 1
exists to answer it before anything depends on the answer. After that, the risk is
ordinary: two pointers 40 bytes apart, and a clamp on a signed count.

What is left untouched: the block layout, the arena contracts, every existing
ctype's marshaling, every byte-list consumer, and **all of plan-57** — which this
sub-plan consumes and never edits.

There is no blocker inside this document. The feature's one hard stop is checked
before any of plan-58 begins (plan-58-A §Prerequisite); past that gate this
sub-plan runs to completion.
