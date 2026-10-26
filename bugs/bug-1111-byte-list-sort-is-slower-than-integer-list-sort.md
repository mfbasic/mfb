# bug-1111: `collections::sort` on a `List OF Byte` is slower than on a `List OF Integer` holding the same values

Last updated: 2026-10-24
Effort: small
Severity: LOW
Class: Other (Performance)

Status: Open
Regression Test: none yet; planned `tests/codegen/codegen_sort_narrow_lanes.rs` (Phase 1)

Found while measuring whether a narrower list stride is faster (`/tmp/list-stride-probe`,
see References). Every other workload was the same speed or faster with the 1-byte
stride. Sort was slower: the Byte/Integer speed ratio was 0.56–0.90 across sizes from
4 Ki to 16 Mi elements (probe run 1, M2 Max, `-O2`). The two lists hold the same values
and sort to the same result. The slowdown is in the Byte sort's comparison code, not the
data. A `Byte` is 1/8 the bytes to move, so it should be at least as fast.

**The single correct behavior a fix produces:** sorting a `List OF Byte` compares two
elements by loading them directly and branching on an unsigned compare. That is the same
shape as the Integer merge's signed-word compare: no multiply, no stack round-trip, and
no Boolean materialization per comparison. Sorting `n` Bytes is no slower than sorting
`n` Integers with the same values.

References:

- `collections::sort`'s descriptor prose (`src/codegen/builtins/collections/func_sort.rs`,
  rendered by `mfb man`): O(n log n) and **stable**. `Byte` orders numerically and
  `Scalar` by code point (`mfb spec language types`, the orderable-types paragraph).
- Found in the list-stride probe, `/tmp/list-stride-probe/README.md` (one-off, not
  committed).
- Sibling performance bug from the same probe: bug-1110 (`FOR` step test).
- History: the copying lowering is bug-767 A (`lower_copying_sort`); the word path is
  plan-86 A1.

## Failing Reproduction

```
mkdir -p /tmp/bug-1111/src && cd /tmp/bug-1111
cat > project.json <<'EOF'
{ "name": "bug1111", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
EOF
cat > src/main.mfb <<'EOF'
IMPORT io
IMPORT collections
IMPORT datetime

FUNC main(args AS List OF String) AS Integer
  MUT ints AS List OF Integer = []
  MUT bytes AS List OF Byte = []
  MUT x AS Integer = 12345
  FOR i = 0 TO 999999
    x = (x * 1103515245 + 12345) MOD 2147483648
    ints = collections::append(ints, (x / 65536) MOD 256)
    bytes = collections::append(bytes, toByte((x / 65536) MOD 256))
  NEXT
  MUT bestInt AS Integer = 9223372036854775807
  MUT bestByte AS Integer = 9223372036854775807
  MUT checkInt AS Integer = 0
  MUT checkByte AS Integer = 0
  FOR r = 0 TO 4
    LET t0 AS Integer = datetime::monotonicNanos()
    LET si AS List OF Integer = collections::sort(ints)
    LET t1 AS Integer = datetime::monotonicNanos()
    LET sb AS List OF Byte = collections::sort(bytes)
    LET t2 AS Integer = datetime::monotonicNanos()
    IF t1 - t0 < bestInt THEN bestInt = t1 - t0
    IF t2 - t1 < bestByte THEN bestByte = t2 - t1
    checkInt = collections::get(si, 333333) * 1000 + collections::get(si, 666666)
    checkByte = toInt(collections::get(sb, 333333)) * 1000 + toInt(collections::get(sb, 666666))
  NEXT
  io::print("sort Integer: " & toString(bestInt / 1000000) & " ms  check " & toString(checkInt))
  io::print("sort Byte:    " & toString(bestByte / 1000000) & " ms  check " & toString(checkByte))
  RETURN 0
END FUNC
EOF
mfb build -O 2 . && ./build/bug1111.out
```

- Observed (macos-aarch64, M2 Max, load average about 11, four runs): Integer 66–67 ms,
  Byte 81–85 ms; both `check 85170`.
- Expected: Byte no slower than Integer.

Structural signal (deterministic; this is what the regression test checks): in the
`-O2` `.ncode` plan of `_mfb_fn_main`, the merge loop from its merge label to its
`take_j` label is:

| path | labels | instructions | ops only on this path |
| --- | --- | --- | --- |
| Integer (word path) | `sort_merge_loop` → `sort_take_j` | 29 | — |
| Byte (copying path) | `inplace_sort_merge` → `inplace_sort_take_j` | 43 | `mul`, `ldr_u8`, `mov_imm`, `cmp_imm`, `b.ne` |

## Root Cause

`src/codegen/builtins/collections/func_sort.rs:sort_fast_path` sends `String`,
`Integer`, `Fixed` and `Money` to the word path
(`builder_inplace_sort.rs:sort_word_path_accepts` → `lower_collection_sort_call`). Every
other ordered element type (`Byte`, `Scalar`, `Float`) goes to
`builder_inplace_sort.rs:lower_copying_sort` → `emit_sort_buffer`. There, the compare
for anything other than String and the signed 8-byte types is
`SortCompare::Operator { values: OperatorValues::Lanes, .. }`. Each comparison in
`emit_sort_compare`'s `Operator` arm does the following, for each of the two sides:

1. loads the index and multiplies it by the payload width (`mul`, with the width
   materialized by `mov_imm`) instead of shifting;
2. reads the element through the generic `emit_load_collection_payload`;
3. stores it to the hidden local `$su_sort_left`/`$su_sort_right` stack slot.

Then it lowers a generic NIR `<` over those two hidden locals. That reloads both slots,
materializes a `Boolean`, and branches with `cmp_imm ; b.ne`. The `SignedWords` arm used
for Integer loads both values with a shift and compares the registers directly
(`cmp ; b.lt`). Nothing in the Byte path needs the generic operator: Byte order is
unsigned integer order.

## Goal

- `collections::sort` of a `List OF Byte` (copying form and `xs = collections::sort(xs)`
  self-update form) compares through a direct-load arm: no `mul` and no `Boolean`
  `mov_imm` inside the merge loop (the regression test checks this in `.ncode`).
- The reproduction prints a Byte time no greater than the Integer time (within noise).

### Non-goals (must NOT change)

- Sort semantics: stable, ascending, the same result for every input. A sort of a list
  with duplicates must keep the original order of equal elements. The Byte values are
  indistinguishable, so stability is unobservable for `Byte` itself, but the shared merge
  must stay stable for every other type.
- `Float`'s comparison semantics (IEEE `<`; finite values only reach a list). If `Float`
  is moved off the `Operator` arm, its signed-zero and NaN behavior must be unchanged.
- The word path for `String`/`Integer`/`Fixed`/`Money`.
- The `.mfb` fallback bodies (`_mfb_ifn_collections_5Fsort_24*`).

## Blast Radius

To audit in Phase 1 (the search: every `SortCompare::Operator` construction in
`src/codegen/collection/assign/builder_inplace_sort.rs`):

- `emit_sort_buffer` + `List OF Byte` (copying and self-update forms): fixed by this bug.
- `emit_sort_buffer` + `List OF Scalar`: same `Operator`/`Lanes` arm (4-byte unsigned
  code points). Not measured, but the same code shape. In scope: the same unsigned-lane
  arm covers it with width 4.
- `emit_sort_buffer` + `List OF Float`: same arm. The compare needs IEEE `<`, not an
  integer compare. Latent; verdict to be recorded (a direct `fcmp` arm is possible, but
  it is a separate change with its own semantic risk).
- `emit_sort_by_buffer` (`sortBy`) with `Byte`/`Scalar`/`Float` keys: verdict to be
  recorded, depending on which compare its key merge uses (`OperatorValues::Words`?).

## Fix Design

Add an unsigned narrow-lane compare to `SortCompare`, e.g.
`UnsignedLanes { values_slot, width }`, built in `emit_sort_buffer` for `Byte` (width 1)
and `Scalar` (width 4) the way `SignedWords` is built (data pointer stored once via
`emit_collection_data_pointer_for`). In `emit_sort_compare`, load each side with
`ldr_u8`/`ldr_u32` at `values + (idx << log2(width))`, then `cmp ; b.lo take_j ; b take_i`
(`CodeOp::BranchLo` exists on all three architectures).

Rejected: routing Byte through the 8-byte word path. It would need a widening copy into
8-byte words and back, which is more memory traffic for a 1-byte type.

No golden shift expected beyond the `.ncode`/`.ncodesum` of programs that sort a
`List OF Byte` or `List OF Scalar`.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/codegen/codegen_sort_narrow_lanes.rs` (pattern:
      `tests/codegen/codegen_place_forward.rs`). For `collections::sort` of a
      `List OF Byte` and of a `List OF Scalar`, copying and self-update forms, at `-O1`,
      the merge segment has no `mul` and no `Boolean` `mov_imm`. Confirm it fails today.
- [ ] Add (or confirm existing) runtime coverage that sorts Byte and Scalar lists with
      duplicates, the extremes (0/255, U+0000/U+10FFFF) and length 0/1/2, and checks
      the result.
- [ ] Complete the blast-radius verdicts above.

Acceptance: the codegen test fails for the documented reason; the runtime sort tests
pass; every audit site has a verdict.
Commit: —

### Phase 2 — the fix

- [ ] `builder_inplace_sort.rs`: the `UnsignedLanes` arm and its construction in
      `emit_sort_buffer` for `Byte` and `Scalar`.

Acceptance: the Phase 1 tests pass; the reproduction's Byte time is no greater than its
Integer time; Non-goals unchanged.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate any shifted `.ncode`/`.ncodesum` goldens. Confirm the delta is only
      Byte/Scalar sort merge loops (AGENTS.md: the full artifact gate and
      `scripts/test-accept.sh` before re-baselining).
- [ ] Landing tier plus `tests/codegen` and the collections rt-behavior fixtures.
- [ ] Re-run the reproduction and the probe's `sort` row; record the numbers here.

Acceptance: gates green; golden delta only the intended change; Byte sort no slower
than Integer sort.
Commit: —

## Validation Plan

- Regression test: `tests/codegen/codegen_sort_narrow_lanes.rs`, plus the runtime sort
  fixtures.
- Runtime proof: the reproduction above, and the probe's `sort` row.
- Doc sync: none expected (no contract change).
- Landing gate: the landing tier plus the named test, `tests/codegen`, and the
  collections fixtures.
- Full tier: the scheduled background run.

## Open Decisions

- Include `Float` in this bug? Recommended: no. Record its verdict and file it
  separately if it is measurably slow, because its compare has IEEE semantics.

## Summary

A contained codegen change in one file. The risk is keeping the shared merge stable and
leaving `Float`'s comparison semantics untouched.
