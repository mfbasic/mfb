# Audit 2 — Surface 3: Codegen & runtime memory safety (arena / collections / strings / arithmetic / SIMD / vector)

Last updated: 2026-07-14
Untrusted party: whoever controls runtime inputs (attacker-supplied strings,
collection sizes, thread transfers, SIMD/vector lengths). Must not: reach an OOB
read/write, UAF, double-free, or size-overflow-driven under-allocation in emitted
native code or runtime helpers.

Scope read: `src/target/shared/code/{entry_and_arena, builder_arena_transfer,
builder_strings, builder_strings_builtins, builder_collection_layout,
builder_collection_queries, builder_values, builder_numeric, builder_money_math,
builder_simd_math, builder_simd_float_math, builder_vector_inline,
runtime_helpers, validation}.rs`.

## Verdict on prior audit-1 findings (re-verified)

| ID | Prior sev | Verdict | Evidence |
|----|-----------|---------|----------|
| MEM-01 | CRITICAL | **FIXED** | `strings.repeat` routes `len*times` through `emit_checked_size_multiply(total, len, times_rem, &invalid)` (`builder_strings_builtins.rs:2286`) + `emit_checked_size_add_immediate` for the +9 header (`:2289`), branching to ErrInvalidArgument on wrap. |
| MEM-02 | CRITICAL | **FIXED** | `strings.padLeft/padRight` guards all three size steps: `emit_checked_size_multiply` (`:2529`), `emit_checked_size_add` (`:2530`), `emit_checked_size_add_immediate` (`:2534`). |
| MEM-03 | HIGH | **FIXED (coalescing path); regressed for bin paths → MEM-09** | `arena_insert_free` has the idempotency guard (`entry_and_arena.rs:1535-1536`) and `arena_free` scrubs only `[ptr+16, ptr+size)` (`:1682-1689`). But the guard lives only in `arena_insert_free`; the now-primary quick-bin (`:1655-1662`) and large-bin (`:1673-1681`) park paths push unconditionally. |
| MEM-04 | HIGH | **NOT DEMONSTRATED** (downgraded) | `builder_arena_transfer.rs:474-486` still does raw `multiply_registers(capacity, ENTRY_SIZE)` + unchecked adds, but the same `size_slot` drives both `arena_alloc` (`:488-493`) and `emit_copy_bytes` (`:511-512`), so a wrap cannot desync alloc-size from copy-length; a wrap needs `capacity ≈ 2^59`, impossible in a real header. Folded into MEM-10. |
| MEM-05 | MEDIUM | **FIXED** | allocation-size multiplies guarded: graphemes (`:103`), toBytes (`:309`), split (`:1693`) all `emit_checked_size_multiply` + `emit_write_cursor_assert`; remaining raw multiplies are bounded write-pass offsets. |
| MEM-06 | MEDIUM | **UNCHANGED, not runtime-reachable** | `arena_free`/`arena_insert_free` trust the compiler-emitted `size`; every `ARENA_FREE_SYMBOL` site passes a codegen-computed size — no runtime-input path. Structural. |
| MEM-07 | LOW | **FIXED** | `entry_and_arena.rs:788-794` rejects `raw > u64::MAX - ARENA_MIN_CHUNK` *before* the `+15` round-up (`:799`), so the round-up cannot wrap. |
| MEM-08 | LOW | **FIXED** | `builder_math.rs:476-490` uses an allocated `dst`/`bound` vreg; no hardcoded `x17`; INT64_MIN caught via `emit_overflow_return`. |

The audit-1 dominant class (unchecked size arithmetic) is closed on the
attacker-reachable allocation paths; `emit_checked_size_multiply` /
`emit_checked_size_add*` are wired into repeat/pad/toBytes/graphemes/split and the
collection append/grow paths (`builder_collection_mutate.rs:574-586`).

## New findings

### MEM-09 — LOW — Arena free-list idempotency guard bypassed by the quick-bin & large-bin park paths
- Location: `entry_and_arena.rs:1655-1662` (quick-bin park), `:1673-1681`
  (large-bin park); guard they bypass at `:1532-1536`.
- Threat/impact: defense-in-depth against double-free. NOT reachable from runtime
  inputs alone; requires a codegen/ownership double-free bug — a class this repo
  has hit repeatedly (`trap-cleanup-double-free`, `union-drop-codegen-nondeterminism`).
  The MEM-03 guard was added to neutralize exactly that class, but now covers only
  chunks that reach `arena_insert_free`.
- Mechanism: `arena_free` routes every chunk `≤ ARENA_QUICK_BIN_MAX` (and every
  larger chunk to a hashed large bin) to an unconditional O(1) head-push:
  `bin_head = bin[slot]; ptr.next = bin_head; bin[slot] = ptr` (`:1658-1661` /
  `:1678-1681`). A second free of the same live `ptr` sets `ptr.next = ptr`
  (self-cycle) and leaves `bin[slot] = ptr`; the next two allocations of that
  class both return `ptr` → aliasing / UAF / heap corruption. The
  `insert_already_free` no-op never runs because bin-sized frees never enter the
  coalescing list.
- Reproduction: not demonstrable from `.mfb` source (no runtime-input path forces
  a double-free). Byte argument: two `arena_free(A, 48)` with no intervening alloc
  → size-48 quick bin is a 1-node self-cycle; two subsequent size-≤48 allocs both
  return `A`.
- Best fix: before the bin push, compare `ptr` against `bin[slot]` and branch to a
  no-op on equality (catches the immediate-double-free self-cycle), mirroring
  `insert_already_free`. No language-surface change.
- Non-goals: full free-list membership verification; poison/quarantine; changing
  the O(1) fast path for the non-double-free case. (LOW — no bug doc; document the
  guard-parity gap for the next allocator touch.)

### MEM-10 — Not demonstrated (LOW, defensive) — Copy/transfer/SIMD allocation multiplies diverge from the checked-helper convention
- Location: `builder_arena_transfer.rs:474-486` (collection copy), `:391,:612`
  (union/record copy); `entry_and_arena.rs:1311-1312` (`simd_alloc_list`:
  `count*(ENTRY_SIZE+8)+HEADER`, raw); `builder_collection_layout.rs:353,:418,:1751`.
- Threat/impact: whoever controls collection sizes / transferred values; in
  principle a wrapped size under-allocates — in practice not demonstrated.
- Mechanism / why not reachable: each multiply takes `count`/`capacity` from an
  *existing* header, so wrapping `count*24`/`count*32` needs `count ≈ 2^59`
  (impossible — the source block would already be that large). And in each copy
  path the identical size value drives both `arena_alloc` and `emit_copy_bytes`
  length (e.g. `builder_arena_transfer.rs:488` vs `:511-512`), so even a
  hypothetical wrap cannot desync alloc-size from write-length. SIMD kernels
  enforce input-length equality (`builder_simd_math.rs:500-503`) and size output
  to the shared `count`.
- Best fix (consistency hardening, optional): route these copy/transfer/simd size
  computations through `emit_checked_size_multiply`/`_add*` for uniformity with
  the append/grow paths. Purely defensive.
- Non-goals: treating it as exploitable; adding cost to the already-checked hot
  append path.

## Fresh-audited areas with no finding
- **plan-41 Scalar (4-byte) collection layout** (`builder_collection_layout.rs:60-134`):
  alignment 4 applied consistently by both the alloc-size pass and the writer via
  `emit_align_offset_slot/register`; reader loads per-entry stored offsets — size
  and write passes stay in lockstep. No under-allocation / stride mismatch.
- **Vector inline ops** (`builder_vector_inline.rs`): lane counts are compile-time
  constants (2/3/4 via `VECTOR_SHAPES`); no runtime length math, no OOB surface.
- **SIMD min/max/clamp** (`builder_simd_math.rs`): input-length equality enforced
  (`:500-503`), output sized to shared `count`, vector loop + scalar tail cover
  exactly `[0..count)`.

## Verdict

Surface 3 is **hardened**. All prior CRITICAL/HIGH memory bugs fixed. One new LOW
(MEM-09: bin-park double-free guard parity gap, not runtime-reachable) and one
Not-demonstrated defensive item (MEM-10). No bug docs (neither is attacker-reachable
from runtime inputs alone).
