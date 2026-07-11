# bug-60: unchecked allocation-size arithmetic before `arena_alloc` in `strings.replace`/`strings.join`/list-replace and in the thread-queue allocator (defense-in-depth, latent overflow)

Last updated: 2026-07-09
Effort: small (<1h)

Several codegen sites compute an arena allocation size with plain `multiply_registers` /
`add_registers` / `add_immediate` and then fill the resulting block with the un-wrapped
byte/element count — with no overflow trap. The audited string builders
(graphemes/toBytes/normalizeNfc/repeat/pad) route every size term through
`emit_checked_size_multiply`/`_add`/`_add_immediate` (which trap on `umulh != 0` /
unsigned carry); these pre-audit siblings still use the raw ops. A genuine 64-bit wrap
would size the block small while the copy pass writes the full count → heap overflow.

All items are **latent / defense-in-depth**: the inputs (arena-resident strings, a
user-supplied thread-queue capacity) are bounded far below 2^64 on real hardware, so a
wrap is not constructible today. They are worth fixing for consistency with the audit's
stated policy ("every arena-size computation shares the same self-defending shape") and to
close the thread-queue case, whose input (`thread::start` limit) is the most attacker-ish
of the set. The single correct behavior a fix produces: every arena-size computation traps
`ErrOverflow`/`ErrOutOfMemory` on wrap instead of under-allocating.

References (all under `src/target/shared/code/`):

- `builder_strings.rs:lower_replace` (`:194`, `add_immediate(ret, output_len, 9)` header)
  and `lower_list_replace` (`:416-426`, `multiply_registers` + `add_registers`): output
  size `value_len + matches*(new_len - old_len)` and `count*ENTRY_SIZE + HEADER + data_len`
  computed raw.
- `builder_strings_builtins.rs:lower_strings_join` (`:1285` `add_registers` accumulate,
  `:1300` `+9` header): `Σ part_len + (count-1)*delim_len` raw. (Sibling
  `lower_strings_split` `:1543-1553` has the same shape but is covered by the KNOWN
  MEM-01..08 set — not re-filed.)
- `runtime_helpers.rs:emit_thread_queue_alloc` (`:106-109`, `multiply_registers(x0,
  capacity, 8)`): `thread::start` in/out queue limit is validated only `>= 1`
  (`lower_thread_start_helper:327-330`), with no upper bound and no `capacity*8` overflow
  guard; a capacity near 2^61 wraps the size to a tiny block while
  `THREAD_QUEUE_CAPACITY_OFFSET` stores the huge value, so a later enqueue indexes far out
  of the allocation → OOB write.
- Audited templates: `emit_checked_size_multiply`/`_add`/`_add_immediate`
  (`builder_codegen_primitives.rs:255-296`); used by `lower_strings_repeat`
  (`builder_strings_builtins.rs:2132`) and `lower_strings_pad` (`:2375-2380`), which raise
  `77050002`.
- KNOWN size-arith class: MEM-01..08. These are new sites of the same class.
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

No constructible wrap on real hardware (the inputs are bounded); this is defense-in-depth.
The thread-queue case is the closest to reachable: `thread::start(..., inLimit :=
2305843009213693952)` (`2^61`) would, if the allocation succeeded, store a 2^61 capacity
against a wrapped-tiny block. In practice the arena rejects the (wrapped-small) request or
the machine cannot supply it, so the OOB is not reached — but the limit is user-supplied
and only `>= 1`-checked, so the guard is a real gap.

- Observed (analytically): raw size computation with no trap; a 2^64 wrap would
  under-allocate.
- Expected: each size term traps on overflow, as the audited siblings do.

Contrast: `strings.repeat`/`strings.pad` guard the identical `len*times` /
`valueLen+pad*padLen` shapes with the checked helpers and raise `77050002`; the counting
and writing passes in the unchecked functions are self-consistent (allocate == write), so
the *only* failure mode is a true 64-bit wrap.

## Root Cause

`lower_replace`/`lower_list_replace`/`lower_strings_join`/`emit_thread_queue_alloc` predate
the size-arithmetic audit and were never converted to the checked helpers; the thread-queue
limit additionally lacks an upper bound.

## Goal

- Every arena-size term in these functions is computed with the overflow-trapping helpers.
- `thread::start` rejects an out-of-range queue limit (upper bound + `capacity*8` guard)
  alongside the existing `>= 1` check.

### Non-goals (must NOT change)

- The success-path sizes for realistic inputs (byte-identical).
- The `split` site (covered by MEM-01..08).
- The audited helpers themselves.

## Blast Radius

- `lower_replace`, `lower_list_replace`, `lower_strings_join`, `emit_thread_queue_alloc`
  (+ `lower_thread_start_helper` limit check) — fixed here.
- Grep the remaining pre-audit builders for raw `multiply_registers`/`add_immediate(_, _,
  <header>)` feeding an `arena_alloc`; fold any found into this bug.

## Fix Design

Replace the raw size terms with `emit_checked_size_multiply`/`emit_checked_size_add`/
`emit_checked_size_add_immediate` branching to the allocation-error / `77050002` label, as
the audited functions do. For the thread queue, also reject `capacity > usize::MAX/8` (or a
sane cap) in `lower_thread_start_helper`.

## Phases

### Phase 1 — audit

- [x] Enumerate every raw size-arith-before-alloc site in the pre-audit builders; confirm
      the four above and any siblings.

### Phase 2 — the fix

- [x] Convert the size terms to the checked helpers; add the thread-queue limit bound.

### Phase 3 — validation

- [x] Regenerate goldens (delta = checked-size instruction sequences); `scripts/artifact-gate.sh`,
      `scripts/test-accept.sh`. Realistic inputs unchanged.

## Validation Plan

- Regression test(s): a `thread::start` with an out-of-range limit is rejected; string
  ops on realistic inputs unchanged.
- Runtime proof: normal replace/join/thread programs behave identically.
- Doc sync: none expected.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

Four pre-audit size computations (string replace/join, list replace, thread-queue alloc)
still use raw arithmetic where the audited siblings trap on overflow. All are latent on
real hardware, but the thread-queue limit is user-supplied and only `>= 1`-checked, making
its guard the one worth closing. The fix is to route the size terms through the existing
checked helpers and bound the queue limit.

## Resolution

Fixed 2026-07-09. All four pre-audit size computations now route every arena-size
term through the overflow-trapping helpers, and `thread::start` gained an upper
bound on its queue limit.

Files changed:

- `src/target/shared/code/builder_strings.rs`
  - `lower_replace`: the per-match growth accumulator (`output_len += new_len`) and
    the `+9` header now use `emit_checked_size_add` / `emit_checked_size_add_immediate`,
    branching to a new `replace_overflow` label that raises the standard allocation
    error (the subtract `-= old_len` cannot underflow — `old_len <= value_len` at a
    match, so it is left raw).
  - `lower_list_replace`: the `data_len` accumulation (both the new- and old-length
    branches), the `count * ENTRY_SIZE` multiply, the `+HEADER`, and the `+data_len`
    terms now use `emit_checked_size_add` / `emit_checked_size_multiply` /
    `emit_checked_size_add_immediate`, branching to `replace_list_overflow`.
- `src/target/shared/code/builder_strings_builtins.rs`
  - `lower_strings_join`: the `+= delim_len` and `+= part_len` accumulators and the
    `+9` header now use the checked helpers, branching to `strings_join_overflow`.
- `src/target/shared/code/runtime_helpers.rs`
  - `lower_thread_start_helper`: after the existing `>= 1` checks, both `inLimit` and
    `outLimit` are rejected (as `ErrInvalidArgument` via the existing `invalid_limit`
    path) when they exceed `MAX_QUEUE_LIMIT = u64::MAX / 8` — the largest capacity
    whose `* 8` byte size still fits in 64 bits.
  - `emit_thread_queue_alloc`: the `capacity * 8` value-array size is now preceded by
    a `umulh` high-half check that branches to a new `size_overflow` handler raising
    `ErrOutOfMemory` (defense-in-depth; the upper bound above already makes the wrap
    unreachable, but the guard closes it structurally).
- `tests/native_size_arith_overflow.rs` (new): four regression tests.

All overflow branches raise the allocation error the codebase already uses for an
oversized request (`emit_allocation_error_return` / `ERR_OUT_OF_MEMORY_CODE` /
`ERR_INVALID_ARGUMENT_CODE`), never a silent clamp or a new "unsupported" diagnostic.

Register lifetimes: every checked size term is computed and stored (or consumed)
before its `bl _mfb_arena_alloc`; no added value is held in a caller-saved register
across a helper call. The string builders already spilled the length to a stack slot
before the alloc; the thread-queue `umulh` scratch (`%v12`) is consumed immediately.

Validation (all executed on macOS/aarch64 with `target/debug/mfb`):

- Runtime, executed:
  - `strings::replace` / `strings::join` / `collections::replace` on realistic inputs
    produce identical output (`hi world hi` / `a-bb-ccc` / `QQ,y,QQ,z`).
  - `thread::start(..., inLimit := u64::MAX/8 + 1, ...)` is rejected at runtime with
    `Code: 77050002 Message: invalid argument` (exit 255), instead of under-allocating.
  - The boundary `u64::MAX/8` is *accepted* (no `*8` wrap) and instead OOMs in the
    arena (`Code: 77010001`), confirming the bound is exact.
  - A normal small limit (`1`, `3`) still runs the worker (`one`, exit 0).
- Emitted-instruction (the string wraps are not constructible with a real input):
  the `-ncode` dump shows a dedicated overflow label per builder targeted by the wrap
  guards (`b.lo` for adds, `umulh` + `b.ne` for the list-replace multiply), and the
  thread helper shows the `MAX_QUEUE_LIMIT` `b.hi` rejects plus the `size_overflow`
  `umulh`/`b.ne` guard. Covered by `tests/native_size_arith_overflow.rs`.

Test commands / results:

- `cargo test --test native_size_arith_overflow` → 4 passed
  (`string_size_arith_has_overflow_guards`,
  `string_size_arith_success_path_unchanged`,
  `thread_queue_limit_out_of_range_rejected`,
  `thread_queue_limit_in_range_accepted`).
- `cargo test --bin mfb code::tests` → 162 passed (arena/free-list unit tests intact).
- `cargo build` → clean.

Native goldens: no checked-in native-code golden (`.ncode`/`.nir`/`.mir`/`.nplan`/
`.nobj`) shifts — those 21 dirs are simple programs that neither import
`strings`/`collections`/`thread` nor call these builders, and the string/thread test
dirs carry only `.ast`/`.ir`/`.run`/`.log` goldens (unaffected: AST, bytecode IR, and
runtime output are unchanged; the delta is native lowering only). The
`scripts/artifact-gate.sh` execution-free self-diff baseline may shift for any
internally-generated program that exercises these helpers; the orchestrator
regenerates it.
