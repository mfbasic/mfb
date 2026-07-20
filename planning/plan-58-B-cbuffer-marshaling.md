# plan-58-B: marshal `OUT CBuffer` — allocate, hand over, truncate

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: plan-58-A (the `CBuffer` vocabulary, `BUFFER … SIZE`, the position
rules); **an externally-visible byte-list constructor that does not yet exist —
see §0 here, and plan-58-A §Prerequisite. This sub-plan cannot start until that is resolved.**
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
- `src/target/shared/code/audio/mod.rs:135` (`emit_alloc_byte_list`) and
  `src/target/shared/code/crypto_ec.rs:208` (`emit_build_byte_list`) — the two
  byte-list constructors that **actually exist**. See §0.
- `src/target/shared/code/error_constants.rs:777` (`COLLECTION_HEADER_SIZE` = 40),
  `:786` (`COLLECTION_ENTRY_SIZE` = 40), the `COLLECTION_OFFSET_*` table, `:815`
  (`COLLECTION_TYPE_BYTE` = 7), `:582` (`_mfb_arena_alloc`), `:587`
  (`_mfb_arena_free`).
- `src/target/shared/code/builder_collection_layout.rs:241`
  (`emit_flat_block_size`), `:2191` (the `MFB_KIND2` gate).
- `src/docs/spec/memory/05_collections.md:24-100` (block layout), `:173-198`
  (**Capacity Headroom** — the rule that decides §4.4).
- `src/target/shared/code/fs_helpers_io.rs:1956`
  (`lower_fs_read_all_bytes_helper`), especially `:2057-2114` — preallocate, then
  `read(2)` **directly into the data region**. The closest structural precedent.
- `.ai/compiler.md` — Hard Completion Gate **and** the Native Codegen Register
  Lifetimes section. Both bind here.

## 0. The blocking prerequisite — verified 2026-07-20

The 2026-07-19 draft of this sub-plan opened its design with "allocate through
`emit_alloc_list` — the shared constructor plan-57-B introduced", and listed
`emit_alloc_list` and `emit_collection_data_pointer_into` in its references as
"both introduced by plan-57-B". **Neither function exists.**

```
rg -n 'fn emit_alloc_list' src/                      → no matches
rg -n 'fn emit_collection_data_pointer_into' src/    → no matches
```

What exists:

| Symbol | Location | Visibility | Fit |
|---|---|---|---|
| `emit_alloc_byte_list` | `audio/mod.rs:135` | private `fn` | Right shape — takes stack offsets (`count_off`, `list_off`) and an `alloc_fail` label, which is exactly a thunk's idiom. **Not callable from `link_thunk.rs`.** |
| `emit_build_byte_list` | `crypto_ec.rs:208` | `pub(super)` | Copies from a source buffer; wrong shape (we want an *empty* buffer for the callee to fill). |

Both still write a `kind = 1` entry table, because the `kind = 2` flip is gated
(§2.3).

**This sub-plan cannot begin until one of these is true:**

- **(a)** plan-57-B track 2 lands a `pub(crate)` byte-list constructor; or
- **(b)** this sub-plan promotes `emit_alloc_byte_list` out of `audio/` into a
  shared module with `pub(crate)` visibility — doing plan-57-B track 2's work
  here, and recording it as such in plan-57-B.

Recommended: **(b)** if plan-57-B track 2 is not already in flight. Promoting one
private function is smaller than blocking the feature, and the function is
already parameterized and documented as the shared copy. What is **not**
acceptable is hand-rolling a header write in `link_thunk.rs` — that would be the
fourth copy of a routine plan-57-B exists to collapse.

Whichever is chosen, record it in this document's §Corrections before writing
code.

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
- **Do not flip `MFB_KIND2`.** That is plan-57-D's decision, not this sub-plan's.
  This sub-plan must work correctly with the gate off (§2.3).

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| Byte-list constructors in the tree | 2 (`audio/mod.rs:135`, `crypto_ec.rs:208`) | `rg -n 'fn emit_(alloc\|build)_byte_list' src/` |
| Of those, callable from `link_thunk.rs` | **0** | both are module-private / `pub(super)` — see §0 |
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

### 2.3 Which representation this lands into — the gate is OFF

`kind = 2` (plan-57-D) is behind `std::env::var("MFB_KIND2")`
(`builder_collection_layout.rs:2191`). **The default build is `kind = 1`.** The
two differ in ways this sub-plan cannot paper over:

| | `kind = 1` (default, live) | `kind = 2` (`MFB_KIND2=1`) |
|---|---|---|
| Block size for `N` bytes | `40 + 41N` | `40 + N` |
| `dataBase` | `block + 40 + 40N` — **depends on capacity** | `block + 40` — constant |
| Arena cost of 8 MiB | 343.9 MB (41.0×) | 8.4 MB |
| Entry-fill loop on alloc | `N` iterations | none |

The 2026-07-19 draft hedged in §2.1 that plan-57 was "a strong preference, not a
correctness prerequisite", but then wrote §4.2 step 5 and §4.4's reads-argument
**only** for `kind = 2` (`dataBase = block + 40`, "a constant offset"). Those two
statements contradict each other. Resolved here in favour of the hedge: this
sub-plan must be correct under **both**, which means it must not open-code
`block + 40` and must obtain the data base from the shared helper (§4.2).

### 2.4 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| `emit_alloc_list` exists | **FALSE** | §0 — no matches in `src/` |
| `emit_collection_data_pointer_into` exists | **FALSE** | §0 — no matches in `src/` |
| A byte-list constructor is callable from `link_thunk.rs` | **FALSE** | both existing ones are private / `pub(super)` |
| `kind = 2` is the live representation | **FALSE** | gated on `MFB_KIND2`, `builder_collection_layout.rs:2191` |
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

1. **A callable constructor** — resolve §0 first. Everything else assumes one
   exists with `pub(crate)` visibility and a data-base helper alongside it.
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
  makes the callee overwrite the header. Under `kind = 2` the gap is 40 bytes, so
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
only under `kind = 2`, which is not the live representation (§2.3). Use the
helper so the gate can flip without touching this sub-plan.

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
   otherwise compute a nonsense block size. `CBUFFER_MAX_BYTES` = **8 MiB** while
   `MFB_KIND2` is off (§2.3) — at `kind = 1` that is already 344 MB of arena.
   Comment the constant with the gate so the two stay traceable.
3. Call the shared byte-list constructor (§0) with `N` and element type `Byte`.
   It writes the header and, under `kind = 1`, the entry table. It branches to the
   existing `alloc_fail` label on failure (`link_thunk.rs:1020` always emits it).
4. Spill the block pointer to its `out_base` word **immediately** —
   `_mfb_arena_alloc` has destroyed every caller-saved register.

**And the step the draft omitted:** add a `continue` guard for `CBuffer` in the
main slot loop, mirroring the CSTRUCT one at `link_thunk.rs:562`. Without it the
loop's `writes_back()` branch (`:564-575`) runs for the same slot and
**overwrites the cslot with `&out_word` and zeroes the out word that now holds
the block pointer** — the two stagings clobber each other. plan-58-A adds this
guard as a refusal (its §2.4); B converts it from `Err` to `continue`.
5. Obtain `dataBase` **from the shared data-pointer helper**, not by adding
   `COLLECTION_HEADER_SIZE`. Under `kind = 1` it is `block + 40 + 40N`; under
   `kind = 2` it is `block + 40`. Store it to the slot's **cslot** word, so the
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
  either representation, and bounds by `count`. Leaving slack in `capacity`
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
memory precisely when the buffer is largest. Note the slack is `41(N - k)` bytes
under `kind = 1` and `(N - k)` under `kind = 2` — so this caveat is *materially
worse* on the default build, and is the strongest argument for flipping the gate
before plan-58-D ships to users.

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
  — **including** after the §0(b) move, which must be a pure relocation.

## Phases

Ordered uncertainty-first: Phase 1 exists to falsify the one unproven premise
(§2.4's last row) before anything is built on it.

### Phase 0 — resolve the constructor (§0)

Not code in this sub-plan's own scope, but nothing here compiles without it.

- [ ] Decide §0 (a) or (b). Record the decision in §Corrections.
- [ ] If (b): move `emit_alloc_byte_list` (`audio/mod.rs:135`) to a shared module,
      make it `pub(crate)`, update `audio/`'s call sites, and add a `pub(crate)`
      data-base helper alongside it that is correct under **both** `kind` values.
      Record the work against plan-57-B track 2.

Acceptance: a `pub(crate)` byte-list constructor and data-base helper exist and
are callable from `link_thunk.rs`; `scripts/artifact-gate.sh` shows every emitted
byte unchanged (a pure relocation changes no codegen).
Commit: —

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
- [ ] `CBUFFER_MAX_BYTES` (8 MiB) and the `buffer_size_fail` block raising
      `ErrInvalidArgument`, commented with the `MFB_KIND2` relationship.
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

### Phase 4 — cross-representation proof (largest blast radius last)

- [ ] Run the whole Phase 1–3 test set with `MFB_KIND2=1` as well as unset.

Acceptance: identical observable results under both gate states. A divergence
here means §4.2 step 5 or §4.4 open-coded a representation assumption — find it
before plan-58-C/D build on it.
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

1. **§0 (a) wait vs (b) promote.** Recommended **(b)** unless plan-57-B track 2 is
   already in flight. Must be settled before Phase 1. (§0)
2. **`CBUFFER_MAX_BYTES` value.** Recommended 8 MiB while `MFB_KIND2` is off; the
   41× multiplier makes 8 MiB cost 344 MB of arena already. Revisit to 64 MiB when
   the gate flips. (§4.2)
3. **Whether Phase 4 should block the feature or merely report.** Recommended
   block — a representation assumption that leaks into the thunk is exactly the
   class of bug that surfaces as heap corruption later. (§Phase 4)

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **`emit_alloc_list` and `emit_collection_data_pointer_into` do not
  exist.** The 2026-07-19 draft named both as landed plan-57-B deliverables and
  built its §3 on them. Verified absent. Restructured as the blocking
  prerequisite in §0, with a decision required before Phase 1.
- 2026-07-20 — **`kind = 2` is not live**; it is behind `MFB_KIND2`
  (`builder_collection_layout.rs:2191`). The draft's §2.1 hedged correctly but
  §4.2 step 5 and §4.4 were written only for `kind = 2`
  (`dataBase = block + 40`, "a constant offset"). Contradiction resolved in §2.3
  in favour of the hedge; §4.2 now takes the data base from a helper, and Phase 4
  proves both representations.
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
ctype's marshaling, and every byte-list consumer.

The blocker is not in this document's own scope: `link_thunk.rs` has no callable
byte-list constructor today (§0), and that must be resolved before Phase 1.
