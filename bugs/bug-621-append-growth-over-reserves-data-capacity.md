# bug-621: in-place append growth reserves ~19 bytes of data per `Byte` element

Last updated: 2026-09-13
Effort: large (3h–1d) — the sizing change is small; the `.ncodesum` regeneration across every fixture that appends is not
Severity: MEDIUM
Class: Other (memory overhead — output is correct)

Status: Open
Regression Test: `tests/runtime/rt_list_append_growth_bounds.rs` (to be written, Phase 1)

Appending to a function-local `List OF Byte` with `data = collections::append(data, b)`
takes the in-place path (amortized O(1), correct output), but every grow sizes the new
block's **data region from its own geometric step, not from the bytes the list needs**.
The data capacity is stepped on *every* grow — including the grows triggered by the
element *count* running out — so it drifts far ahead of the data actually stored. For a
16,777,216-byte list the block reserves roughly 19 data bytes per element; peak live
memory is 544,810,144 bytes and peak RSS 981,975,040 bytes, for 16 MiB of data. The same
16 MiB read by `fs::readBytes` peaks at 17,825,792 bytes RSS.

Nothing reports it: every value is right, the loop is linear, and no test asserts the
memory of an append-built list. It surfaced while measuring for the library-free
`compress::` plan (plan-137), whose decoder builds its output by appending and whose
default output cap is 64 MiB.

**The single correct behavior a fix produces:** an append-built fixed-width list reserves
data capacity proportional to its element capacity (`capacity × payload width`), so a
16,777,216-element `List OF Byte` built by appending has a peak live footprint within a
small constant of 16 MiB × the growth factor — and a `List OF Byte` no longer measures
the same as a `List OF Integer`.

References:

- `.ai/collections.md` — "In-place MUT append", "Headroom" (`emit_geometric_step`
  growth shape), "A fixed-width list is entry-FREE".
- `planning/plan-137-compress.md` — the plan whose measurement found this.
- Commit `ce000592b` (plan-01 Ph3+4, 2026-06-25) introduced the growth constants with the
  comment "Lookup slots and data bytes grow independently"
  (`src/codegen/error/constants/error_constants.rs:COLLECTION_GROW_DATA_INIT`).
- `planning/todo.md` "Memory" section — the separate, already-tracked question of freed
  arena regions not lowering RSS (see Root Cause, second effect).

## Failing Reproduction

`/tmp/p/project.json` (any executable manifest, e.g. copied from
`tests/rt-behavior/arena/construct-helper-loop/project.json`) and `/tmp/p/src/main.mfb`:

```
IMPORT io
IMPORT collections

SUB main()
  LET n AS Integer = 16777216
  MUT data AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < n
    LET b AS Byte = toByte((i * 7 + 3) MOD 256)
    data = collections::append(data, b)
    i = i + 1
  END WHILE
  io::print(toString(len(data)))
END SUB
```

```
target/release/mfb build --debug -q /tmp/p
/tmp/p/build/<name>.out 2>&1 | grep -E 'arena.0.(alloc_calls|alloc_bytes|peak_live_bytes|grow)|peak_rss'
```

- Observed (macos-aarch64, release `mfb` built 2026-09-13 at `2829dde83`):
  `alloc_calls 35`, `alloc_bytes 980594176`, `peak_live_bytes 544810144`, `grow 28`,
  `process.peak_rss_bytes 981975040`.
- Expected: peak live bytes a small multiple of 16,777,216 (the data) — not 32×.

**The discriminating contrast:** change `List OF Byte` to `List OF Integer` (and append
the Integer directly). The report is **identical to the byte** — `alloc_bytes 980594176`,
`peak_live_bytes 544810144` — although each element is 8× wider. Same at 1 MiB: both
report `alloc_bytes 86029216`, `peak_live_bytes 47829776`. The reservation does not
depend on the element's width at all.

| Case | n | alloc_bytes | peak_live_bytes | peak RSS |
| --- | --- | --- | --- | --- |
| `List OF Byte`, append, LET-bound item | 1,048,576 | 86,029,216 | 47,829,776 | 87,375,872 |
| `List OF Integer`, append | 1,048,576 | 86,029,216 | 47,829,776 | 87,343,104 |
| `List OF Byte`, append, `toByte(...)` operand | 4,194,304 | — | — | 291,766,272 |
| `List OF Byte`, append, LET-bound item | 16,777,216 | 980,594,176 | 544,810,144 | 981,975,040 |
| `List OF Integer`, append | 16,777,216 | 980,594,176 | 544,810,144 | 981,975,040 |
| `fs::readBytes` of a 16 MiB file (no append) | 16,777,216 | — | — | 17,825,792 |

Contrast cases that are fine today: `fs::readBytes` (exact-size block); list literals and
known-size builders, which "ignore these (exact alloc)" per the constants' own comment.

The in-place arm is really the one running: `mfb build --ncode` of the probe emits the
`append_inplace_*` / `append_grow_cap_*` / `append_grow_dcap_*` labels and loads the
entry stride as `mov_imm x8, #0` — so this is not a fallback to copying (35 allocations
for 16 M appends confirms that too).

## Root Cause

`src/codegen/collection/list/list_mutate.rs:lower_list_append_in_place`, grow arm
(label `append_inplace_realloc`). A grow fires when **either** `count >= capacity` **or**
`dataLength + need > dataCapacity`. Whichever fired, it computes:

- `newCapacity = step(capacity)` — `emit_geometric_step(.., COLLECTION_GROW_LOOKUP_INIT = 4, COLLECTION_GROW_LOOKUP_TAPER = 1024)`;
- `newDataCapacity = max(step(dataCapacity), dataLength + need)` —
  `emit_geometric_step(.., COLLECTION_GROW_DATA_INIT = 32, COLLECTION_GROW_DATA_TAPER = 65536)`;
- allocation `HEADER + newCapacity × entry_stride + newDataCapacity` (stride 0 for a
  fixed-width list — `builder_collection_layout.rs:list_entry_stride`).

`emit_geometric_step` (`src/codegen/collection/buffer/collection_buffer.rs`) doubles below
its taper and multiplies by 1.5 above it. For a 1-byte element the count is always the
binding limit, so every grow is count-triggered, and the data capacity is stepped
alongside anyway:

1. it starts 8× the element capacity (32 vs 4);
2. the element capacity leaves doubling at 1024, but the data capacity keeps doubling
   until 65,536 bytes — three more doublings against three ×1.5 steps, widening the ratio
   by 8 / 3.375 ≈ 2.37;
3. above both tapers both step ×1.5, so the ratio is frozen at ≈ 8 × 2.37 ≈ 19 data bytes
   reserved per element slot.

The `max(.., dataLength + need)` clamp never engages, because the step is already far
above the need. For an `Integer` element the true need (8 bytes per slot) sits below the
same stepped value, so it reserves the identical block — which is why the two reports
match to the byte. Arithmetic cross-check (guess-grade, not measured): a final block of
~16.8 M slots × 19 B ≈ 320 MB, live alongside the ~11.2 M-slot predecessor during the
copy, ≈ 530 MB — consistent with the measured 544,810,144 peak live.

**Second, separate effect (already tracked, not this bug):** peak RSS (981,975,040) equals
the *sum* of every generation's allocation (`alloc_bytes 980594176`), not the peak live
(544,810,144): each grow maps a fresh region (`maps 28 = grow 28`) and the freed
predecessors do not lower RSS before exit. That is the arena-reuse question in
`planning/todo.md`'s Memory section; fixing this bug shrinks every generation, which
shrinks that sum proportionally, but does not change the mapping policy.

## Goal

- `tests/runtime/rt_list_append_growth_bounds.rs` builds the reproduction at
  n = 16,777,216 and asserts, via `common::run_bounded_with_rss`, that peak RSS of the
  `List OF Byte` program is below a bound set from the fix's own sizing rule (derive the
  number from the final formula and record the derivation in the test, not a tuned
  constant), and that it is strictly below the `List OF Integer` program's peak RSS.
- The `List OF Integer` program's footprint does not grow (it must not become the price of
  shrinking the byte case).

### Non-goals (must NOT change)

- Amortized O(1) append: the number of grows must stay logarithmic (35 allocations today
  for 16 M appends). A fix that sizes the data region *exactly* per grow and so reallocates
  on every data-triggered append is forbidden.
- The collection block layout, header fields, `dataCapacity` meaning, and the "data base
  uses capacity, never count" rule (`.ai/collections.md`).
- Variable-width lists (`List OF String`, records with inline data): their data need is
  independent of the count, so their data capacity must keep growing on its own step when
  data is the binding limit.
- Output of any program. This is memory-only; any behavioural golden (`build.log`, run
  output) that moves is a bug in the fix.
- Tempting wrong fix: lowering `COLLECTION_GROW_DATA_INIT` / `_TAPER` only. It narrows the
  ratio for one element width and leaves the drift mechanism (stepping data capacity on
  count-triggered grows) in place for every other width and for the splice/bulk/inline arms.

## Blast Radius

Every `emit_geometric_step` call (`grep -rn --include='*.rs' "emit_geometric_step(" src` → 15
calls, 9 functions). Verdicts marked UNVERIFIED get a Phase 1 probe.

- `list_mutate.rs:lower_list_append_in_place` (`append_grow_cap` + `append_grow_dcap`) —
  **fixed by this bug**; measured above.
- `list_mutate.rs:lower_inline_list_append_in_place` (`inline_append_grow_cap` +
  `inline_append_grow_dcap`) — same coupled cap+dcap shape for a list inlined in a record /
  `STATE` field — fixed by this bug; UNVERIFIED by measurement.
- `list_mutate.rs:lower_list_bulk_append_in_place` (`bulk_append_grow_cap` +
  `bulk_append_grow_dcap`) and `lower_inline_list_bulk_append_in_place`
  (`inline_bulk_grow_cap` + `inline_bulk_grow_dcap`) — same coupled shape; a bulk append of
  a fixed-width list knows its incoming data width exactly — fixed by this bug; UNVERIFIED.
- `list_mutate.rs:lower_list_splice_in_place` (two steps, `prepend`/`insert`) — same coupled
  shape — fixed by this bug; UNVERIFIED.
- `map/map_mutate.rs:lower_map_set_in_place` capacity grow (`mapset_grow_cap` +
  `mapset_grow_dcap`) — same coupling for a map whose values are fixed-width — latent, same
  hazard; include in the fix if Phase 1's probe (a `Map OF Integer TO Byte` built by `set`)
  shows the width-independent footprint, otherwise record why it is immune.
- `map/map_mutate.rs:lower_map_set_in_place` value regrow (`mapset_vgrow_dcap`) — data-only,
  fires only when the value data is the binding limit — unaffected (no count-triggered step).
- `list_mutate.rs:emit_grow_list_data_capacity` (`set_grow_dcap`) — data-only; capacity is
  unchanged by construction (`.ai/collections.md`, "deliberately simpler than `append`'s
  grow") — unaffected.
- `assign/builder_inplace_assign.rs:lower_string_self_append_one` (`concat_self_step`) —
  a `String`'s bytes, no element capacity to drift against — unaffected.

No test pins the constants or the `*_grow_*` labels
(`grep -rn --include='*.rs' "append_grow\|GROW_DATA\|geometric" tests` → comments only, in
`rt_scope_drop_leaks.rs`, `codegen_inplace_record_field.rs`,
`codegen_inplace_append_call_result.rs`, `rt_res_state_inplace_mutation.rs`); those four are
the nearest behavioural guards and must stay green.

## Fix Design

For a **fixed-width** element (`list_element_is_fixed_width(element_type) = Some(w)`), the
data capacity is a function of the element capacity, not an independent counter: compute
`newCapacity = step(capacity)` exactly as today and set
`newDataCapacity = newCapacity × w`. Then a count-triggered and a data-triggered grow are
the same event, the reservation is exactly `w` bytes per slot, and growth stays geometric
(the count's step governs it). The `max(.., dataLength + need)` clamp becomes unnecessary
for that branch but is harmless to keep as an assertion of the invariant.

For variable-width elements keep today's independent step — their need is not a function
of the count.

Where the risk concentrates: the inline-record and splice arms repoint or shift the data
region in place, and a smaller `dataCapacity` moves the data base
(`header + capacity × stride + ...`) for any code that assumed the old headroom. Every
reader already goes through `emit_collection_data_pointer` / the size authority
`emit_inlined_block_size_from_ptr_slot`, so a site that hand-computes a block size is the
thing to look for (the `.ai/collections.md` "Size a collection block through the authority"
rule records two past offenders).

Expected output shift: `.ncodesum` / `.ncode` goldens of every fixture that emits one of the
fixed sites move (the immediate constants and the removed dcap step). `.ast`, `.ir`,
`build.log` and run output must not move.

Rejected: tuning `COLLECTION_GROW_DATA_INIT` / `_TAPER` (see Non-goals); sizing the data
region exactly per append (breaks amortized O(1)).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] `tests/runtime/rt_list_append_growth_bounds.rs` + `[[test]]` in `Cargo.toml`, modelled
      on `tests/runtime/rt_json_bounds.rs` (`common::temp_project`, `common::build_project`,
      `common::run_bounded_with_rss`): the 16 MiB `List OF Byte` reproduction, the
      `List OF Integer` contrast, and the assertion `byte_rss < integer_rss`. Confirm it fails
      today (the two are equal).
- [ ] Probe each UNVERIFIED site in Blast Radius with a same-shape `--debug` program (Byte vs
      Integer element); write each site's verdict into this file.

Acceptance: `cargo test --test rt_list_append_growth_bounds` fails on the `byte < integer`
assertion; every Blast Radius line carries a measured verdict.
Commit: —

### Phase 2 — the fix

- [ ] `list_mutate.rs:lower_list_append_in_place`: fixed-width branch derives
      `newDataCapacity` from `newCapacity × w`.
- [ ] Same change at every site Phase 1 classified as affected.

Acceptance: `cargo test --test rt_list_append_growth_bounds` passes; the four nearest guards
(`rt_scope_drop_leaks`, `rt_res_state_inplace_mutation`, `codegen_inplace_record_field`,
`codegen_inplace_append_call_result`) pass.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `scripts/regen-native-goldens.sh target/release/mfb` for the drifted fixtures; confirm
      only `.ncodesum`/`.ncode` moved (`git diff --stat -- '*.ast' '*.ir' '*build.log'` empty).
- [ ] `cargo test --no-fail-fast > /tmp/bug621.log 2>&1; echo EXIT=$?` and
      `scripts/test-accept.sh target/release/mfb /tmp/bug621-accept`.
- [ ] Re-run the reproduction on macos-aarch64 and on box 2223 (linux-aarch64 glibc), and
      record the new numbers in this file.

Acceptance: full suite and acceptance green; golden delta is native-dump-only; the
reproduction's peak live bytes fall by the factor the formula predicts.
Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_list_append_growth_bounds.rs`.
- Runtime proof: the `--debug` reproduction above, before/after, on macOS and 2223.
- Doc sync: `.ai/collections.md` "Headroom" line (states the growth shape) — add the
  fixed-width rule; `mfb spec` collections/memory pages if they state the headroom formula
  (`grep -rn "geometric\|headroom" src/docs/spec` before closing).
- Full suite: `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh target/release/mfb all`.

## Open Decisions

- Map capacity grow (`mapset_grow_cap`/`mapset_grow_dcap`) — include if Phase 1's probe
  shows the same width-independence (recommended), vs. leave to a follow-up.

## Summary

The mechanism is one sizing rule applied at five list arms (and possibly one map arm): data
capacity stepped on count-triggered grows. The fix is a few lines per arm; the risk is in the
inline-record and splice arms, which move the data base, and the cost is native-golden
regeneration. Output, layout and amortized complexity are untouched. Found during plan-137;
filed, not fixed, in that session.
