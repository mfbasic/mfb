# Audit 1 — Codegen & Runtime Memory Safety

Scope: the machine-code emission and runtime helpers that back arrays/collections,
strings/bytes, checked arithmetic, and the arena allocator — `src/target/shared/code/`
(`entry_and_arena.rs`, `builder_arena_transfer.rs`, `builder_collection_*.rs`,
`builder_strings*.rs`, `builder_value_semantics.rs`, `builder_values.rs`,
`builder_numeric.rs`, `builder_fixed_math.rs`, `builder_math.rs`,
`builder_conversions.rs`, `builder_bits.rs`, `builder_control.rs`,
`builder_search.rs`, `builder_inplace_assign.rs`, `codegen_utils.rs`,
`error_constants.rs`, plus the map-probe/arena runtime helpers in `mod.rs`).

Every finding below is grounded in the emitted-instruction logic (not the Rust host
code) and traced through its call path. Bounds checks that ARE present were confirmed
and are listed under "Checked and OK" rather than reported.

Convention verified up front: `abi::branch_lt`/`branch_ge` lower to `b.lt`/`b.ge`
(**signed** conditions — `src/arch/aarch64/abi.rs:386-391`). So the list index checks
of the form `index < 0` (`branch_lt`) + `index >= count` (`branch_ge`) correctly treat a
negative index as negative, not as a huge unsigned value. The "negative-index-as-huge-
unsigned" class does **not** apply to `get`/`set`/`removeAt`/`insert`/`mid`.

Root amplifier (referenced by several findings): `_mfb_arena_alloc` normalizes the
request with `add_immediate(size, size, 15)` then `AND ~15` and has **no upper-bound /
overflow guard** (`entry_and_arena.rs:630-632`). A size within 15 of `u64::MAX` wraps to
a tiny rounded value and the allocation *succeeds small*. This turns every unchecked
size computation below from a benign OOM into an under-allocation → out-of-bounds write.

---

## MEM-01 — CRITICAL: `strings.repeat` result-size overflow → under-allocation → heap OOB write
**Location:** `src/target/shared/code/builder_strings_builtins.rs:1713-1716`
**Issue:** The output length is a plain 64-bit multiply of the source byte length by the
user-supplied repeat count, with no overflow check:
```rust
self.emit(abi::multiply_registers(total, len, times_rem)); // total = len * times, wraps mod 2^64
self.emit(abi::store_u64(total, abi::stack_pointer(), total_slot));
self.emit(abi::add_immediate(abi::return_register(), total, 9)); // alloc total + 9
```
`times_rem` is a signed `Integer`, checked only `>= 0` (`:1709-1710`). The copy loop
(`:1746-1763`) then writes `times` copies of `len` bytes — i.e. the *true*, un-wrapped
`len*times` bytes — into a block sized from the *wrapped* `total + 9`. With `len=16`,
`times=0x1000000000000001`, `total` wraps to `16`, arena_alloc grants ~25 bytes, and the
loop writes ~2^64 bytes: immediate heap corruption.
**Trigger:** `LET s = strings.repeat("ab", 4611686018427387905)`
**Fix:** After the multiply, compute the high 64 bits (`umulh total_hi, len, times_rem`)
and branch to `emit_invalid_argument_return` / `emit_allocation_error_return` when
`total_hi != 0`, before the `+9` and the `branch_link(ARENA_ALLOC_SYMBOL)`. (The
`umulh`/`emit_checked_integer_multiply` machinery already exists in `builder_numeric.rs`.)

## MEM-02 — CRITICAL: `strings.padLeft`/`padRight` result-size overflow → under-allocation → heap OOB write
**Location:** `src/target/shared/code/builder_strings_builtins.rs:1907-1909`
**Issue:** `pad_count = max(0, width - scalarLen)` where `width` is a user `Integer`
checked only `>= 0` (`:1889-1901`). The total size is then computed with an unchecked
multiply and add:
```rust
self.emit(abi::multiply_registers("x12", "x10", "x11")); // pad_count * padLen, wraps
self.emit(abi::add_registers("x11", "x9", "x12"));       // total = valueLen + that, wraps
// ... allocate total + 9  (:1912)
```
The pad-write loop stores `pad_count * padLen` bytes into the wrapped-small allocation →
heap OOB write.
**Trigger:** `LET s = strings.padLeft("x", 9223372036854775807)` (default padChar is one
space, `padLen = 1`, so `pad_count` ≈ i64::MAX and `total` wraps).
**Fix:** Bound `width` to a sane maximum, or `umulh`-check `pad_count * padLen` and a
carry-check the `valueLen + product` add, routing to `emit_invalid_argument_return`
before alloc. Apply to both `padLeft` and `padRight` (shared `lower_strings_pad`).

## MEM-03 — HIGH: double-free corrupts the arena free-list (`arena_free`/`arena_insert_free` have no idempotency guard)
**Location:** `src/target/shared/code/entry_and_arena.rs:1076-1083` (walk) and
`:1180-1186` (scrub-then-insert in `arena_free`)
**Issue:** `arena_insert_free` walks the address-ordered list stopping only when
`cur > ptr` (`branch_hi`, `:1080`); an equal node (`cur == ptr`, an already-free chunk)
does **not** stop the walk — it advances `prev = cur = ptr` (`:1081`). There is no
"already free" check anywhere. Worse, `arena_free` entropy-scrubs the chunk *before*
inserting it:
```rust
abi::branch_link(ARENA_FILL_RANDOM_SYMBOL), // scrub — overwrites the chunk's {next,size} words
...
abi::branch_link(ARENA_INSERT_FREE_SYMBOL), // then coalesce, reading those now-garbage words
```
So on a second free of a block, coalescing reads `prev.size` (`:1089`) from PRNG bytes
and computes `prev_end = prev + garbage`, then `store_u64` writes an attacker-influenced
size into a free node — producing an overlapping/self-referential free-list, which the
next `arena_alloc` hands out as live memory (use-after-free / heap aliasing).
**Trigger:** any codegen path that frees one block twice. The runtime is built on
scope-drop `OwnedValue` frees plus resource/thread/union-drop cleanups; the memory notes
already document a resource-union drop-order non-determinism bug and a TRAP-cleanup
double-free class. When any such slip frees a block twice, this is the corruption sink.
**Fix:** In `arena_insert_free`, after `compare_registers(&cur, "x0")` add
`branch_eq("insert_already_free")` where `insert_already_free` is a bare `return_()`
(idempotent no-op). Two instructions make a double-free safe rather than corrupting.
Independently, `arena_free` should scrub *after* the insert decision (or `insert_free`
should snapshot neighbor size/next before the caller scrubs) so coalescing never reads
poisoned metadata.

## MEM-04 — HIGH: thread-transfer collection copy computes allocation size with unchecked `capacity * ENTRY_SIZE`
**Location:** `src/target/shared/code/builder_arena_transfer.rs:387-398`
(`copy_collection_to_current_arena`)
**Issue:** The destination allocation size is
`capacity*ENTRY_SIZE + HEADER + data_capacity`, read from the *source* collection header,
with a plain multiply and adds:
```rust
self.emit(abi::load_u64("x9", source, COLLECTION_OFFSET_CAPACITY));
self.emit(abi::move_immediate("x10", "Integer", &COLLECTION_ENTRY_SIZE.to_string()));
self.emit(abi::multiply_registers("x9", "x9", "x10"));   // capacity * ENTRY_SIZE — unchecked
self.emit(abi::add_immediate("x9", "x9", COLLECTION_HEADER_SIZE));
... self.emit(abi::add_registers("x9", "x9", "x10"));     // + data_capacity — no carry check
```
A corrupted/oversized `capacity` or `data_capacity` header word (which MEM-03 can
produce, or a genuinely enormous collection) wraps the size; `arena_alloc`'s own
unguarded normalization (root note) then rounds it tiny, and the downstream entry/data
copy walks up to `capacity` entries into the undersized buffer → OOB write in the
*receiving* thread's arena.
**Trigger:** transferring a collection between threads (`thread::transfer*`) whose header
`capacity`/`dataCapacity` is large. Contrast: `arena_alloc`'s grow path *does* guard its
adds (`entry_and_arena.rs` walk overflow guards at `:648`, `:653`); this copy path does
not.
**Fix:** After the multiply emit `umulh`/compare-and-`branch_lo` overflow checks (mirror
the `arena_alloc_*` overflow idiom), and carry-check the `data_capacity` add, routing to
`emit_allocation_error_return`. The same guard belongs in `copy_flat_block` /
`copy_collection_tight` (`builder_collection_layout.rs:263-283`) which use the identical
`count * ENTRY_SIZE + HEADER + dataLength` formula.

## MEM-05 — MEDIUM: `strings.toBytes` / `graphemes` / `split` collection-size multiply unchecked
**Location:** `src/target/shared/code/builder_strings_builtins.rs:198`
(`lower_strings_to_bytes`), `:76` (`lower_strings_graphemes`), `:1209`
(`lower_strings_split`)
**Issue:** Each computes the result-collection allocation size as
`count * (ENTRY_SIZE[+1]) + HEADER` with a plain `multiply_registers` and no overflow
guard, e.g. toBytes:
```rust
self.emit(abi::multiply_registers("x13", "x9", "x13"));   // count * (ENTRY_SIZE+1)
self.emit(abi::add_immediate(return_register, "x13", COLLECTION_HEADER_SIZE));
```
`count` is a string byte/grapheme/segment count. Because `ENTRY_SIZE ≈ 40`, the product
overflows for a source of byte-length ≥ ~`2^64/41`, and the write loop still stores
`count` entries + payload into the wrapped-small allocation → OOB write. Practically the
source string would have to be multi-exabyte (OOM-first), which is why this is MEDIUM
rather than CRITICAL, but the codegen omits the guard uniformly.
**Fix:** Same `umulh` overflow check on the entry-size product (and the header add)
before alloc, in all three lowerings.

## MEM-06 — MEDIUM: arena coalescing cannot detect an over-sized free extent (blind adjacency merge)
**Location:** `src/target/shared/code/entry_and_arena.rs:1089-1128` (`arena_insert_free`
prev-side and next-side merges); `arena_free` size derivation `:1173-1179`
**Issue:** `arena_free` derives the freed extent purely from the *caller-passed* size
argument (`round_up(size,16)`) — it is never read back from an authoritative block
header. Coalescing then extends a neighbor node from bare adjacency
(`prev_end == ptr` / `ptr+size == cur`) with no check that `[ptr, ptr+size)` does not
overlap a live allocation. If any `arena_free` call site passes a size that disagrees
with what the matching `arena_alloc` granted (e.g. a data-union whose runtime `size@8`
word exceeds its block, or a collection whose alloc/free size formulas diverge), the free
chunk overlaps the following live allocation; the next `arena_alloc` first-fit walk hands
it out → use-after-free / heap overlap.
**Trigger:** a size mismatch between an alloc site and its scope-drop free site. All
frees are generated from static type-size helpers (`emit_flat_block_size`, union
`size@8`, `capacity*ENTRY+dataCapacity`); any drift between an alloc formula and its free
formula lands here silently.
**Fix:** Give `arena_alloc` a one-word size redzone/header on the granted block and have
`arena_free` read the granted size back rather than trusting the caller's size argument,
so alloc/free extents can never disagree. (A cheaper partial guard — clamping a merge to
the containing mmap block bounds — does not catch intra-block overlap.) Design-level;
flag for the memory-layout owner.

## MEM-07 — LOW: `_mfb_arena_alloc` size normalization has no overflow guard (root amplifier)
**Location:** `src/target/shared/code/entry_and_arena.rs:630-632`
**Issue:**
```rust
abi::add_immediate(&size, &size, (ARENA_MIN_CHUNK - 1) as usize), // size += 15, no guard
abi::move_immediate(&not15, "Integer", &not_15),
abi::and_registers(&size, &size, &not15),                         // & ~15
```
For a request `> u64::MAX - 15` this wraps to a tiny normalized size. The free-list
*walk* itself is overflow-safe (`:648`, `:653`), so a wrapped-tiny size does not corrupt
the walk — it just returns a chunk far smaller than the caller believes, and the caller's
fill/copy (sized from the original intent) overflows it. No caller clamps the request
before the call (traced: transfer copies, string builders, simd_alloc_list, entry-args
materialization). This is the single choke point that turns MEM-01/02/04/05 from
"allocation fails" into "allocation succeeds small → OOB".
**Fix:** One guard after `move_register(&size, "x0")`: compare the raw request against
`u64::MAX - ARENA_MIN_CHUNK` and `branch_hi("arena_alloc_invalid")`. Two instructions
close the wrap for every caller at once; still keep the per-site MEM-01/02 checks so the
program gets `ErrInvalidArgument` rather than an allocation error where appropriate.

## MEM-08 — LOW: `math.abs` uses a hardcoded physical `x17` scratch (allocator-fragility class)
**Location:** `src/target/shared/code/builder_math.rs:419`, `:439`
**Issue:** `lower_math_abs` writes the INT_MIN sentinel / sign mask into a literal
`"x17"` while `value.location` is live:
```rust
self.emit(abi::move_immediate("x17", "Integer", "9223372036854775808"));
self.emit(abi::compare_registers(&value.location, "x17"));
```
`x16`/`x17` are in the allocatable pool (`arch/aarch64/regmodel.rs`), so a live operand
could in principle be colored `x17`. Under the default LinearScan allocator this is safe
(it tracks physical-operand interference), and the code is byte-identical under the bump
oracle today, so this is **not** a live miscompile — but it is the same fragility class
that already forced the integer-negation path off `x17` (see the comment at
`builder_numeric.rs:318-327`). Reported as hardening only.
**Fix:** Replace the literal `"x17"` in `lower_math_abs` with an `allocate_register()`
temporary, as the negation path was fixed.

---

## Checked and OK (verified present — not reported as findings)

- **List `get` / `getOr` bounds** (`builder_collection_query.rs:28-32`, `:314-318`):
  `index < 0` (`branch_lt`) + `index >= count` (`branch_ge`, signed) → `list_get_invalid`
  → `emit_index_out_of_range_return`. Correct.
- **`removeAt` bounds** (`builder_collection_mutate.rs:1753-1758`): `0 <= index < count`
  signed. Correct.
- **`insert`/`append`/`prepend` bounds** (`builder_collection_mutate.rs:403-408`):
  `0 <= index <= count` signed. Correct.
- **`set` (list)** routes through `removeAt(index)` then `insert(index)`
  (`:248-262`), so an out-of-range set traps at `removeAt`. Correct.
- **`lower_list_set_in_place`** (`:1073-1082`): `0 <= index < count` signed. Correct.
- **In-place append room check** (`:606-618`): `count < capacity` AND
  `dataLength + need <= dataCapacity` before write, geometric grow otherwise. No
  unbounded write. (Only used on private accumulators.)
- **`substring`-family char-offset ops** — `mid` (`builder_search.rs:577-639`):
  `start<0`/`count<0` rejected, `end=start+count` overflow-guarded (`branch_lo`), locate
  loops bounded by byte length via `remaining==0 → invalid`. `left`/`right`
  (`builder_strings_builtins.rs:1557-1642`), `graphemeAt` (`:2039-2103`), byte-list→string
  continuation-byte availability (`:835-1068`) all bounds-checked. Correct.
- **Checked integer arithmetic**: `+`/`-` via flags + `emit_overflow_return`
  (`builder_numeric.rs:829-862`); `*` via `smulh` vs sign-extension
  (`emit_checked_integer_multiply`, `:1164-1181`); `/`/`DIV`/`MOD` via zero-check
  (`emit_nonzero_or_invalid`) + INT_MIN/-1 check
  (`emit_integer_division_overflow_check`, `:870-894`, `:1183-1215`); unary neg/abs
  INT_MIN-guarded (`:294-305`, `:1217-1230`, `builder_math.rs:417-423`). Complete.
- **Fixed-point `*`/`/`** high-word range checks (`builder_numeric.rs:1262-1370`);
  scale-by-2^n headroom check (`builder_fixed_math.rs:566-610`). Present.
- **Float→Int / Float→Fixed** reject NaN/Inf (`exponent==2047 → ErrInvalidFormat`) and
  range-check magnitude before `fcvtzs` (`builder_conversions.rs:86-140`, `:725-774`).
  Guarded.
- **Bit shifts** validate `0 <= count <= 63` → `ErrInvalidArgument`
  (`builder_bits.rs:110-116`); rotates intentionally unchecked (hardware reduces mod
  width — total for any count, semantically sound).
- **Map probe helper** (`mod.rs:2232-2352`): `count==0` early-out, `hash mod bucketCount`
  via `unsigned_divide` + `msub`, probe wraps at `bucketCount` (`:2322-2326`). Bucket
  index stays in range. Correct.
- **Register-across-`bl` clobber class**: the hand-rolled string/collection routines
  consistently spill pointers/lengths to stack slots before `arena_alloc` and reload
  after (e.g. concat `builder_value_semantics.rs:395-402`; repeat/pad carry the result
  pointer in a fresh vreg specifically because a held physical result register is fragile
  cross-ISA). `arena_free` spills ptr/size across both helper calls
  (`entry_and_arena.rs:1163-1186`). No held-live-across-call physical-address bug found.
- **Uninitialized memory**: `arena_alloc` poisons freshly-mapped usable regions with the
  fill-RNG before insert; `Bind` zeroes an owned freeable slot before a fallible
  initializer (`builder_control.rs:69-72`) so scope-drop can't free an uninitialized
  pointer. No uninitialized-observable window found.
