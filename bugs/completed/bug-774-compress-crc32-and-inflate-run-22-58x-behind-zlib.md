# bug-774: `compress::crc32` is 58× and `compress::inflate` 22× slower than zlib — per-access check overhead, per-byte output appends with spilled loop state

Last updated: 2026-10-04
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Other (Performance)

Status: Fixed (2026-10-04, bug-staging-13)
Regression Test: `tests/runtime/rt_compress_throughput.rs` (timing ratios + `.ncode` shape on four
targets); `tests/runtime/rt_self_slice_append.rs`; `tests/runtime/rt_runtime_helper_callee_saved.rs`;
`src/codegen/engine/regalloc/tests.rs:audited_runtime_helpers_clobber_only_caller_saved_registers`;
`src/codegen/engine/util/vreg_frame.rs:tests::required_sp_local_bytes_uses_each_access_width`

The `compress` benchmark group has one F row and one D row (`./benchmark/rank.py` →
`F LIB compress crc32 13.488 0.232 0.241 58.1x 56.0x`, `D LIB compress inflate 18.208
0.841 0.895 21.7x 20.3x`). `deflate` (44.3 ms vs C 13.95) and `gzip` (50.8 vs 14.16) are
3.2–3.6× and are not covered here.

| row (min ms, `benchmark/baseline/*.log`) | mfb -O1 | mfb -O2 | mfb -O3 | C -O0 | C -O2 | Python |
|---|---|---|---|---|---|---|
| `compress crc32` (64 × crc32 of 131,073 B) | 13.488 | 12.310 | 12.313 | 0.232 | 0.232 | 0.241 |
| `compress inflate` (16 × inflate + len + crc32 of the output) | 18.208 | 15.265 | 14.997 | 0.841 | 0.864 | 0.895 |

The C and Python peers call system zlib. Its `crc32` runs 8 MiB in 0.237 ms, or about
35 GB/s. No table-driven software CRC reaches that. The same machine's C slicing-by-8,
the algorithm `compress::crc32` uses, takes 4.35 ms at `-O2` (probe below). zlib is
therefore hardware- or carry-less-multiply-accelerated. Closing all 58× would take a
native CRC path, and plan-137-A rejected that (§ Open Decisions). What this bug fixes
is the part that belongs to MFB: the same algorithm runs **2.5× slower than C `-O0`**
(14.1 vs 5.66 ms). `inflate` also spends most of its time outside the algorithm, in
per-byte output appends whose loop state lives in memory, not in registers.

**The single correct behavior a fix produces:**

1. **crc32.** 64 × `compress::crc32` over the same 131,073 bytes takes at most **8.5 ms
   at `-O1`** on the reference machine (macos-aarch64, the machine that produced the
   baseline). That is ≤ 1.5× the 5.66 ms of C `-O0` running the same slicing-by-8
   algorithm (`/tmp/dbF-crc-c0`). The benchmark row falls to about 8.5 ms, or 37× C
   (D).
2. **inflate.** 16 × `compress::inflate` of the 20,820-byte level-6 stream, without the
   checksum, takes at most **5.0 ms at `-O1`** (15.1 ms today). The benchmark row,
   inflate plus crc32, reaches **≤ 12.6 ms**, which is ≤ 15× C `-O0` and out of D.
   Target derivation (guess): 2.1 M match bytes at ≤ 1 ns each, plus 153 k symbols at
   ≤ 15 ns each, is about 4.4 ms.
3. **Per-call cost.** `compress::inflate` of a 5-byte payload costs at most 5 µs per
   call. Today it is 24–32 µs.

Output bytes, checksums, errors, error codes and `maxBytes` behavior are unchanged.

References:

- `mfb spec stdlib compress` (`src/docs/spec/stdlib/20_compress.md`): "Algorithm:
  slicing-by-8", "Measured throughput". The dated MiB/s and "CRC-32 at ≈86 MiB/s"
  figures there are already stale: the crc32 probe below measures ≈ 590 MiB/s at
  `-O1`. The fix re-measures them.
- `mfb spec memory collections`: "Fixed-Width Lists (kind 2)", "Self-updates" (the
  in-place append path, amortized O(1)) and "Cost". This is the cost contract the fix
  stays inside.
- `planning/completed/plan-137-A-compress-package-crc32.md` §1 Non-goals ("No native
  code") and §3 Rejected alternatives. `plan-137-B` Phase 1 holds the decoder design
  measurements.
- `planning/optimizations.md` "Bounds-check elimination" row (Level 3,
  `src/optimizer/opt2/checks.rs`).
- Found by the benchmark D/F-grade sweep (`benchmark/RANKING.md`).

## Failing Reproduction

All numbers: macos-aarch64, `target/release/mfb` at `7e53c82b9`,
`mfb build -O 1 -q .`, best of 5 in-process rounds.

### A. crc32 (`/tmp/dbF-crc`)

`project.json` is the standard executable manifest (`"entry": "main"`, sources `src/**`).
`src/main.mfb`:

```
IMPORT io
IMPORT compress
IMPORT collections
IMPORT datetime
IMPORT bits

FUNC mkInput(n AS Integer) AS List OF Byte
  MUT out AS List OF Byte = []
  MUT x AS Integer = 12345
  FOR i = 1 TO n
    x = (x * 1103515245 + 12345) MOD 2147483648
    out = collections::append(out, toByte(97 + (x / 65536) MOD 26))
  NEXT
  RETURN out
END FUNC

' contrast: a plain byte-sum loop over the same list (one get per byte)
FUNC byteSum(data AS List OF Byte) AS Integer
  MUT s AS Integer = 0
  MUT i AS Integer = 0
  LET n AS Integer = len(data)
  WHILE i < n
    s = s + toInt(collections::get(data, i))
    i = i + 1
  END WHILE
  RETURN s
END FUNC

' contrast: byte-at-a-time table CRC with the table passed as a LOCAL argument
FUNC crcLocal(data AS List OF Byte, t AS List OF Integer) AS Integer
  MUT crc AS Integer = 4294967295
  MUT i AS Integer = 0
  LET n AS Integer = len(data)
  WHILE i < n
    crc = bits::bxor(bits::sr(crc, 8), collections::get(t, bits::band(bits::bxor(crc, toInt(collections::get(data, i))), 255)))
    i = i + 1
  END WHILE
  RETURN bits::bxor(crc, 4294967295)
END FUNC

FUNC mkTable() AS List OF Integer
  MUT t AS List OF Integer = []
  FOR i = 0 TO 255
    MUT c AS Integer = i
    FOR k = 1 TO 8
      IF bits::band(c, 1) = 1 THEN
        c = bits::bxor(bits::sr(c, 1), 3988292384)
      ELSE
        c = bits::sr(c, 1)
      END IF
    NEXT
    t = collections::append(t, c)
  NEXT
  RETURN t
END FUNC

FUNC main() AS Nothing
  LET x AS List OF Byte = mkInput(131072)
  LET t AS List OF Integer = mkTable()
  MUT best AS Integer = 1000000000000
  MUT acc AS Integer = 0
  FOR r = 1 TO 5
    LET t0 AS Integer = datetime::monotonicNanos()
    acc = 0
    FOR rep = 1 TO 64
      acc = acc + compress::crc32(x)
    NEXT
    LET t1 AS Integer = datetime::monotonicNanos()
    IF t1 - t0 < best THEN
      best = t1 - t0
    END IF
  NEXT
  io::print("crc32 x64 (8 MiB): " & toString(best / 1000) & " us  sum=" & toString(acc))
  ' ... the same best-of-5 harness around crcLocal(x, t) and byteSum(x), 64 reps each
END FUNC
```

Observed (`./build/crcprobe.out`):

```
crc32 x64 (8 MiB): 14146 us  sum=265406097920
user byte-table crc x64: 30224 us  sum=265406097920
byteSum x64: 12656 us  sum=918409216
```

At `-O2`: 12510 / 30117 / 12810 µs.

The same-algorithm C peer is `/tmp/dbF-crc-c.c`: slicing-by-8 with `uint32_t T[8][256]`,
the same bytewise tail, the same input and 64 reps, plus zlib `crc32` for reference.
`cc -O0 … -lz` and `cc -O2 … -lz`:

```
-O0: slice8 x64: 5660 us   bytewise x64: 20832 us   zlib x64: 237 us   (sum=265406097920 for all)
-O2: slice8 x64: 4351 us   bytewise x64: 19831 us   zlib x64: 232 us
```

- Observed: MFB slicing-by-8 takes 14.1 ms, 2.5× C `-O0` running the same algorithm.
  It is no faster than `byteSum`, a loop that does nothing per byte but one
  `collections::get` and one add.
- Expected: ≤ 8.5 ms (≤ 1.5× C `-O0` slicing-by-8).
- Contrast: the checksum is correct (MFB, C and zlib all give 265406097920, and the
  benchmark's `test_compress_*` sums are identical in `benchmark/baseline/{mfb-O1,c-O0,python}.sums`),
  and the
  call passes `data` by pointer with no block copy (`bl _mfb_ifn_compress_5Fcrc32`
  right after loading the list pointer from its slot in `_mfb_fn_main`).

### B. inflate (`/tmp/dbF-inf`)

`src/main.mfb` holds the benchmark's `compressInput` (verbatim from
`benchmark/mfb/src/compress.mfb`) and these functions:

```
' the inflate back-reference copy shape alone: per-byte self-append (distance 8)
FUNC copyLoop(src AS List OF Byte) AS List OF Byte
  MUT out AS List OF Byte = []
  MUT k AS Integer = 0
  LET n AS Integer = len(src)
  WHILE k < 8
    out = collections::append(out, collections::get(src, k))
    k = k + 1
  END WHILE
  WHILE k < n
    out = collections::append(out, collections::get(out, k - 8))
    k = k + 1
  END WHILE
  RETURN out
END FUNC

' the same self-copy through in-place set on a pre-sized list (no grow path)
FUNC copySet(src AS List OF Byte) AS List OF Byte
  MUT out AS List OF Byte = src
  MUT k AS Integer = 8
  LET n AS Integer = len(src)
  WHILE k < n
    out = collections::set(out, k, collections::get(out, k - 8))
    k = k + 1
  END WHILE
  RETURN out
END FUNC

' 14-byte matches at distance 1000: per byte, and via the existing List-append overload
FUNC matchPerByte(src AS List OF Byte) AS List OF Byte
  MUT out AS List OF Byte = collections::take(src, 1000)
  MUT outLen AS Integer = 1000
  LET n AS Integer = len(src)
  WHILE outLen + 14 <= n
    LET from AS Integer = outLen - 1000
    MUT k AS Integer = 0
    WHILE k < 14
      out = collections::append(out, collections::get(out, from + k))
      k = k + 1
    END WHILE
    outLen = outLen + 14
  END WHILE
  RETURN out
END FUNC

FUNC matchBulk(src AS List OF Byte) AS List OF Byte
  MUT out AS List OF Byte = collections::take(src, 1000)
  MUT outLen AS Integer = 1000
  LET n AS Integer = len(src)
  WHILE outLen + 14 <= n
    out = collections::append(out, collections::mid(out, outLen - 1000, 14))
    outLen = outLen + 14
  END WHILE
  RETURN out
END FUNC

' per-call floor: 1000 inflates of a 5-byte payload
FUNC tinyInflate() AS Nothing
  LET d AS List OF Byte = compress::deflate(strings::toBytes("hello"), 6)
  MUT acc AS Integer = 0
  LET t0 AS Integer = datetime::monotonicNanos()
  FOR rep = 1 TO 1000
    acc = acc + len(compress::inflate(d))
  NEXT
  LET t1 AS Integer = datetime::monotonicNanos()
  io::print("tiny inflate x1000: " & toString((t1 - t0) / 1000) & " us  sum=" & toString(acc))
END FUNC
```

`main` wraps each expression below in a best-of-5 × 16-rep harness, with
`d = compress::deflate(compressInput(131072), 6)`.

```
tiny inflate x1000: 25498 us  sum=5000          (runs: 23556, 24278, 25498, 32047)
input 131073 B, deflated 20820 B
inflate only x16: 15701 us  sum=2097168          (runs: 15092, 15144, 15378, 15701)
inflate+crc32 x16 (bench row shape): 18506 us
crc32 only x16 (128 KiB each): 3434 us
per-byte append self-copy (dist 8) x16: 8294 us
per-byte in-place set self-copy x16: 6787 us
14-byte matches, per-byte append x16: 9552 us
14-byte matches, mid+append bulk x16: 6000 us
```

C `-O0` contrast (`/tmp/dbF-copy.c`, a `push()` that `realloc`-doubles, the same
distance-8 self-copy of 131,073 bytes × 16): **4820 µs**.

Symbol statistics of the stream: `/tmp/dbF-stats.py`, a pure-Python inflate with
counters, run on zlib's level-6 stream of the same text (20,796 B; mfb's own is
20,820 B). Result: 1 dynamic block, **331 literals, 9,220 matches, 130,742 match bytes
(99.7% of the output), mean match 14.2**.

- Observed: 15.1–15.7 ms per 16 inflates, 7.2 ns per output byte. The row adds 3.4 ms
  of crc32 over the output (sub-issue A). A 5-byte inflate costs 24–32 µs.
- Expected: ≤ 5.0 ms per 16 inflates, ≤ 12.6 ms for the row, ≤ 5 µs per tiny call.

| Environment | config | Result |
| --- | --- | --- |
| macos-aarch64 | `-O1` | crc32 14.1 ms, inflate 15.1 ms ✗ |
| macos-aarch64 | `-O2` | crc32 12.5 ms ✗ (probe); inflate row 15.3 ms ✗ (baseline) |
| macos-aarch64 | `-O3` | crc32 12.3 ms, inflate row 15.0 ms ✗ (baseline) |
| other targets | — | not measured. The mechanism is target-neutral (the `.ncode` shape comes from shared lowerings), so expect the same. |

## Root Cause

### A. crc32: about 40 instructions per byte, 80% of them checks and address arithmetic

`__compress_crc32` (`src/codegen/builtins/compress/helper_crc32.rs:BODY`) is correct
slicing-by-8 in MFB source. Each 8-byte step does 16 `collections::get`s: 8 on `data`,
8 on the module-level `__COMPRESS_CRC32_TABLES`. The generated code for one step was
counted from `.ncode` (`mfb build -O 1 --ncode --target macos-aarch64`, then
`/tmp/dbF-hot.py crcprobe.ncode '#compress_crc32'`). That script counts the
instructions of the `while_loop_17` region, excluding segments that call
`_mfb_make_error_result`. Result: **326 hot instructions per 8 bytes** at `-O1`, 285 at
`-O3`. C `-O0` needs about 5.66/14.1 of the time. Histogram at `-O1`: `mov_imm` 42,
`ldr_u64` 32, `mov` 25, `add_imm` 24, `cmp_imm` 23, `cmp` 20, `b.lt` 19, `mul` 19,
`b` 18, `adds` 18, `b.vc` 18, `b.ge` 16, `add` 16, `ldr_u8` 8, `eor` 8. The overhead
breaks down as follows (the attribution is from reading the listing):

1. **16 bounds checks per step** (`cmp_imm idx,0; b.lt; ldr len; cmp; b.ge`), from
   `src/codegen/builtins/collections/func_get.rs:lower_get`. Level-3 BCE
   (`src/optimizer/opt2/checks.rs`) keeps 14 of 16. The data reads `i + 1 … i + 7`
   need the relational fact `i <= n - 8`. The table reads index a *module global*
   whose length (2,048) the optimizer cannot see, because
   `__compress_crc32Tables()` builds it at run time
   (`helper_crc32_table.rs:BODY`).
2. **Element addressing by multiply**: `mov_imm w; mul idx,w; add_imm base,40; add`
   (19 `mul`). Scaled addressing does it in one `ldr`.
3. **The global table re-read on every access**: `add_imm x10, x19, 3784; ldr x21,
   [x10]`, then a length reload for the bounds check.
4. **Checked arithmetic that cannot overflow**: 18 `adds`/`b.vc`, e.g.
   `1536 + band(sr(x,8),255)`, plus `smulh`-checked `256 * toInt(byte)` for the
   little-endian fold.
5. **`bits::sr` range-checks a constant count**: `mov_imm 8; cmp_imm 0; b.lt;
   cmp_imm 63; b.le` (`src/codegen/builtins/bits/func_sr.rs:lower_bits_sr`, the
   `bits_shift_out_of_range` label). plan-137-A §2 already recorded this ("`sl`/`sr` add
   two compare-and-branch pairs on the count even when it is a constant").
6. **Every check's raise block sits inline in the hot path.** Each successful check
   jumps over its block with a taken `b …_done` (18 per step).

The contrast rows show the cost is per access, not specific to CRC. `byteSum`, one
`get` plus one add per byte, runs at the same 1.5 ns per byte (12.7 ms per 8 MiB) as
the whole slicing-by-8 loop (14.1 ms). The C `-O0` slicing-by-8 does the same table
work with none of items 1–6.

### B. inflate: the output is written one checked append at a time, and the loop state lives in memory

`__compress_inflateCore` (`src/codegen/builtins/compress/helper_inflate_core.rs:BODY`)
is table-driven. Its cost has four parts:

1. **Match copy, about 55% (measured by contrast).** Each back-reference is
   `WHILE k < length: out = collections::append(out, collections::get(out, from + k))`.
   99.7% of the output comes through it. The contrast `copyLoop` does only this, over
   the same 2 MiB, and takes 8.3 ms (4 ns per byte); inflate takes 15.1 ms. The listing
   of `copyLoop`'s loop (`-O1 .ncode`) shows the reason:
   - The in-place append fast path (`src/codegen/collection/list/list_mutate.rs:lower_list_append_in_place`)
     is about 70 instructions.
   - `k` lives at `[sp+416]` and `out` at `[sp+24]`. Both are reloaded and stored every
     iteration.
   - The list's `count` (`+8`) and `dataLength` (`+24`) are updated through memory.
     Every iteration therefore carries three store→load round trips.

   The frame explains the spills. `copyLoop` uses only callee-saved register `x21`, and
   `__compress_inflateCore` has a **15,792-byte frame** whose only callee-saved register
   is `x21` (its `.ncode` `frame`). The reason is
   `src/codegen/engine/regalloc/analysis.rs:call_clobber_mask`. It treats every
   `bl _mfb_*` runtime-helper call (anything but `_mfb_fn_*`/`_mfb_ifn_*`) as
   clobbering **all** integer registers (`ALL_PHYS`), callee-saved ones included. The
   append's grow path calls `_mfb_arena_alloc` and `_mfb_arena_free`. That path is
   cold, but it rejoins the loop, so every value live across it (the whole loop state)
   can have no register at all and is spilled.

   Contrast: `byteSum`, which has no rejoining runtime call, keeps `s`, `i` and `n` in
   `x9`–`x13`. `copySet` is only slightly faster (6.8 ms). The in-place `set` lowering
   also carries a rejoining rebuild path with `_mfb_arena_alloc` and
   `_mfb_rt_drop_owned_collection`, even for a `List OF Byte`, whose element width can
   never change.

   The clobber model and its own documentation also disagree. `call_clobber_mask`'s
   comment says "`_mfb_arena_alloc` tramples callee-saved `x20`–`x28`". Its doc-comment
   two paragraphs up, `.ai/compiler.md`, and `lower_arena_alloc`
   (`src/codegen/memory/arena/arena.rs`, "callee-saved (`x19`–`x28`) are preserved by
   the PCS frame") all say it preserves them. Whichever is true decides whether the
   narrow fix below is sound (Phase 1 audit).
2. **Symbol decode, about 40% (residual: 15.1 − 8.3 ms over 153 k symbols, about
   44 ns per symbol).** Three causes:
   - The decode reads `symEntry MOD 16` and `symEntry / 16`, and the same for
     `dsymEntry` and the sub-table entries. These lower to `sdiv` + `msub` (30 `sdiv`
     and 17 `msub` in the function, counted in `.ncode`), with the INT_MIN/zero guards
     before each.
   - `bitBuf = bits::sr(bitBuf, symEntry MOD 16)` puts that division on the
     loop-carried `bitBuf` dependency chain.
   - 42 constant/variable `bits::sr`/`sl` range checks and 30 bounds checks remain, and
     `bitBuf`/`bitCount`/`pos` are frame slots (offsets 56/64/48), reloaded on every
     use for the reason in item 1.

   The entries are non-negative wherever those operations run: after the `< 0`
   rejection, or after `>= 1048576`. So `bits::band(e, 15)` and `bits::sr(e, 4)` give
   identical values. `mfb spec stdlib compress` already measured `bits::` shifts
   beating `/`/`MOD` by 7–19% elsewhere in the decoder.
3. **A per-call rebuild of the fixed tables (24–32 µs per call).** Every call runs
   `__compress_buildTable(__compress_fixedLitLens(), 0, 288, 9, 1)` and the 32-entry
   distance table, including calls whose stream uses only dynamic blocks.
   `fixedLitLens` alone is 288 appends. The code reading, and the spec's "about 36 µs
   more per block — the per-block table build", point at this. Hoisting will confirm it
   (hypothesis until then). It is 2.5% of the benchmark row (16 calls) but all of the
   cost for small payloads.
4. **The benchmark row's own crc32 of the output**: 3.4 ms of the 18.2 ms. Sub-issue A
   covers it.

## Goal

- G1: `/tmp/dbF-crc` "crc32 x64" ≤ 8.5 ms at `-O1`, and the benchmark row `compress crc32`
  ≤ 8.5 ms (`benchmark/baseline` regenerated by the normal procedure).
- G2: `/tmp/dbF-inf` "inflate only x16" ≤ 5.0 ms at `-O1`, and the row `compress inflate`
  ≤ 12.6 ms.
- G3: "tiny inflate x1000" ≤ 5,000 µs.
- G4: every output byte, checksum, error and error code is identical. The
  `tests/interop/rt_compress_interop.rs` suite and `tools/oracles/compress` pass
  unchanged.

### Non-goals (must NOT change)

**Owner constraint:** no change to the language specification or the memory model. The
following are untouched:

- value semantics (`get`/`LET`/argument/return behave as copies);
- `collections::*` results, ordering, errors and error codes;
- syntax, keywords, builtins and signatures (no new public `compress::` or
  `collections::` member);
- `mfb man` / `mfb spec language` behavior;
- `mfb spec memory` (copy/mutate/alias rules, the arena allocator and drop contract,
  the in-place self-update cost contract in `mfb spec memory collections`
  "Self-updates"/"Cost", RES handles).

Only lowering, fast paths, optimizer and register-allocator behavior, and the
package's private MFB helper bodies may change.

- `compress` public surface, output format and errors: the same bytes for `crc32`,
  `inflate`, `zlibDecode` and `gzipDecode`. The decoders keep checking `maxBytes`
  before every write and keep every refusal (over-subscribed/incomplete codes, bad
  distances, and so on, per plan-137-B).
- **Do not edit the benchmark row** (`benchmark/mfb/src/compress.mfb`) or its C/Python
  peers. The row's crc32-of-output is part of what it measures.
- **Forbidden wrong fixes:**
  - dropping a bounds, overflow or shift check that is not *proven* dead. Every
    elision must come from a proof (range facts or a known-constant length), never
    from "the helper is trusted";
  - loosening the regalloc clobber mask for a helper without auditing that helper's
    real clobber set;
  - replacing the MFB source with a vendored or `dlopen`ed zlib (plan-137-A rejected
    it: no system zlib on Windows, and output would vary by distro).
- Hardware CRC (AArch64 `crc32x`, x86 PCLMUL folding) is **not** in scope. It
  contradicts plan-137-A's "No native code / same bytes everywhere by construction"
  decision (§ Open Decisions).

## Blast Radius

Found by `grep -rln "collections::get(__" src/codegen/builtins`, by the regalloc
clobber rule (every function with a runtime-helper call that rejoins a loop), and by
`grep -rn "MOD 16\|/ 16" src/codegen/builtins/compress`.

- `helper_crc32.rs:__compress_crc32`: fixed by this bug (A).
- `helper_inflate_core.rs:__compress_inflateCore`: fixed by this bug (B1–B3).
  `zlibDecode` and `gzipDecode` share it, so they inherit the gain. `gzipDecode` also
  runs crc32 over its output (A).
- `helper_adler32.rs` (zlibDecode's checksum): same per-access pattern as A (a `get`
  per byte). Fixed by the same codegen items, with no separate change.
- `helper_deflate_core.rs`, `helper_dynamic_block.rs` (the encoders): latent, same
  per-byte append and spill pattern. `deflate`/`gzip` are 3.2–3.6× C and outside
  D/F. Out of scope, but they benefit from B1's regalloc fix automatically; re-measure
  in Phase 3.
- `src/codegen/engine/regalloc/analysis.rs:call_clobber_mask` (`ALL_PHYS` for every
  `_mfb_*` helper): **the widest site**. It applies to every loop in every program
  that contains an in-place `append`/`set`/`insert` grow path or a
  `_mfb_rt_drop_owned_collection` call that rejoins the loop. Fixing it (Phase 2,
  step 3) moves `.ncode` for most programs. Byte-identity goldens will churn, which is
  expected (`.ai/testing-gates.md`).
- `crypto::` MFB helpers (`SHA-2`/`AES` loops over `bits::` + `collections::get` of
  module tables such as `__CRYPTO_K256`, per plan-137-A §2): latent, same A pattern
  (global table reloads, bounds checks, constant-count shift checks). Not measured
  here. They benefit from the codegen items without change.
- `bits::sl`/`sr`/`sra` constant-count checks (`src/codegen/builtins/bits/func_s*.rs`):
  fixed by this bug (A5) wherever the count is a compile-time constant in 0..63.
  A variable count keeps its check.
- `canvas` PNG decode (`__canvas_pngDecode` → `zlibDecode`): unaffected semantically.
  It gets faster through B.

## Fix Design

Each item stays within `mfb spec memory collections` ("Self-updates": an in-place
append on a uniquely-owned `MUT` list is amortized O(1). Nothing here changes when a
copy happens or who owns a block) and within `mfb spec language` (values, errors,
codes).

**A. crc32 (codegen, no source change needed; source tweaks allowed):**

1. Treat a constant shift count in 0..63 as proven. `bits::sl`/`sr`/`sra` skip the
   range compare when `args[1]` is a known immediate. This is an in-lowering check
   elision with a proof.
2. BCE for the module table. Two options:
   - make `__COMPRESS_CRC32_TABLES`'s length a fact (a `LET` global whose initializer
     provably returns a 2,048-element list), so `1792 + band(x,255)` ∈ [1792, 2047] is
     proven in range by the Level-3 range lattice (`opt2/plans/ranges.rs`);
   - or, equivalently and simpler, have the helper read `len(__COMPRESS_CRC32_TABLES)`
     once into a local and guard once before the loop.

   For `data`, the dominating `i <= n - 8` with `n = len(data)` proves `i + 0 … i + 7`
   (range-check widening, already a Level-3 row). Make it fire at `-O1` for this shape,
   or accept the `-O3`-only gain and document it.
3. Scaled addressing in `lower_get` for fixed-width lists (`ldr xD, [base, idx, lsl #3]`
   once the header offset is folded). This touches every target's selector
   (`.ai/arch-abi.md`).
4. Hoist the global's load out of the loop (LICM on a global that the loop does not
   write, `opt2/licm.rs`).
5. Sink raise blocks out of line (cold-block layout), so the hot path has no taken
   branches over them.

Estimate (guess): items 1–4 remove about 170 of the 326 instructions per step, which
lands near C `-O0`.

**B. inflate:**

1. **Regalloc: a precise clobber set for PCS-framed runtime helpers.** Give
   `call_clobber_mask` a per-helper table. `_mfb_arena_alloc`, `_mfb_arena_free` and
   `_mfb_rt_drop_owned_collection` are candidates, each **only after** its emitted body
   is audited to preserve `x19`–`x28` (and the x86-64/riscv64 equivalents: the table is
   per target, as `caller_saved` already is, bug-350). Such a helper clobbers only
   `caller_saved`. Loop state can then live in callee-saved registers across the cold
   grow path. Resolve the comment contradiction above first. If `_mfb_arena_alloc`
   really tramples `x20`–`x28`, the fix is to make it PCS-clean (save what it uses),
   not to trust it.

   Rejected alternative: live-range splitting around the cold call alone. bug-760
   records that Level-2 splitting is only sound for straight-line intervals
   (`.ai/codegen-invariants.md` "Register pressure"). A loop with a rejoining cold path
   is exactly the unsound shape.
2. **Bulk match copy in the decoder source (private helper body only).** Two cases:
   - when `dist >= length` (non-overlapping), use the existing public overload
     `out = collections::append(out, collections::mid(out, from, length))`;
   - when `dist < length`, append `mid(out, from, dist)` repeatedly, doubling as the
     window allows.

   This is the same bytes by construction. The contrast measured 9.55 → 6.00 ms at
   mean length 14. `mid`'s result is a copy (value semantics). A later lowering fast
   path may elide that temporary *only* because it is provably dead after the append.
   That is allowed copy-elision, not a semantic change. Keep the per-byte loop for
   `length <= 2` if the bulk form measures slower there.
3. **Decode arithmetic.** Replace `e MOD 16` / `e / 16` with `bits::band(e, 15)` /
   `bits::sr(e, 4)` on the non-negative entries. It is the same value, with no division
   on the `bitBuf` chain.
4. **Fixed tables once per program.** Move `fixedLit`/`fixedDist` to module-level
   `LET __COMPRESS_FIXED_LIT/…_DIST = …` globals, the `__COMPRESS_CRC32_TABLES`
   pattern, gated `WhenUsed` on the decoders. The block reads them by value (`litT =
   fixedLit` is a copy today too). Measure that the copy is cheaper than the build;
   if not, index the global directly for fixed blocks.

Expected output shifts: `.ncode`/byte-identity goldens for every compress-importing
fixture (B2–B4, A). B1 and A1–A5 move `.ncode` across the tree. Every shift must be
code only: no `.out`/`build.log` runtime output may change.

## Phases

### Phase 1: failing tests + audit (no behavior change)

- [x] `tests/runtime/rt_compress_throughput.rs` (new, modeled on
      `tests/runtime/rt_compress_bounds.rs::decode_time_is_linear_in_output_size`:
      one release build, `datetime::monotonicNanos` around each measured call,
      interleaved over five rounds, medians):
      - `crc32_runs_near_a_plain_byte_loop`: median crc32(x) ≤ 0.6 × median of a
        same-process `byteSum(x)`. Today it is ≈ 1.1× (14.1 / 12.7 ms). The reference
        loop calibrates for the machine.
      - `inflate_output_costs_less_than_a_per_byte_self_copy`: median `inflate(d)` ≤
        0.5 × median `copyLoop(x)` over the same 128 KiB. Today it is ≈ 1.8×
        (15.1 / 8.3 ms).
      - `tiny_inflate_per_call_cost`: median of 1,000 inflates of `"hello"` ≤ 0.2 ×
        median of 1,000 `collections::mid(x, 1, 131000)` copies of a 128 KiB list.
        That is a native word-copy loop, which none of the fixes touch. Today it is
        ≈ 0.62: 24.8–25.7 ms vs 40.2–43.4 ms (`/tmp/dbF-ref`, three rounds).

      Thresholds are placeholders from this doc's numbers. Phase 2 re-derives them
      from the fixed build, keeping a ≥ 1.5× margin to the pre-fix value.
- [x] `.ncode` structure tests (modeled on `tests/runtime/rt_bounds_elim_backedge.rs::get_guard_branches`
      via `common::build_ncode`):
      - `#compress_crc32`'s 8-byte loop contains no `bits_shift_out_of_range` label and
        no `list_get_invalid` guard on the table reads;
      - `copyLoop`'s loop-carried counter is not reloaded from a stack slot (no
        `ldr_u64 … sp …` of `k` inside the loop).
- [x] Audit `_mfb_arena_alloc`, `_mfb_arena_free` and `_mfb_rt_drop_owned_collection`
      emitted bodies on all five targets for callee-saved preservation. Resolve the
      `call_clobber_mask` comment contradiction and write the verdict per helper into
      this doc.
- [x] Complete the Blast Radius verdicts for `crypto::` by measuring one SHA-256 loop
      before and after.

Acceptance: the new tests fail for the documented reasons; the audit table is filled in.
Commit: `1d3a2fbbe` (RED test), `9a2a09933` (audit + test). See *Resolution*.

### Phase 2: the fix

- [x] A1 constant-count `bits::` shift elision (`src/codegen/builtins/bits/func_sl.rs`,
      `func_sr.rs`, `func_sra.rs`).
- [x] A2/A4 table-length fact or single guard, and LICM of the global load
      (`src/optimizer/opt2/checks.rs`, `licm.rs`, or `helper_crc32.rs` source).
- [x] A3 scaled addressing in `lower_get` for fixed-width lists (all targets).
- [x] A5 raise blocks out of line.
- [x] B1 per-helper clobber table in `src/codegen/engine/regalloc/analysis.rs:call_clobber_mask`
      (per target, after the Phase 1 audit), with a unit test beside
      `call_clobber_masks_match_each_targets_abi` in `regalloc/tests.rs`.
- [x] B2–B4 in `helper_inflate_core.rs` / `helper_huffman_table.rs` (private bodies).

Acceptance: the Phase 1 tests pass; G1–G4 hold on the probes; interop/oracle
unchanged; nothing in Non-goals changed.
Commit: `9a2a09933` B1, `ac558a36f` A3, `35cbc8c7f` A1, `a0567278c` (compare fusion, added),
`207a6ed31` A5, `cd55d5cf5` (self-slice copy, added), `27b8bf00c` A2/A4 (source) + B2–B4.

### Phase 3: regenerate expected outputs + full validation

- [ ] Run the full suite first, then `scripts/sync-goldens.sh` for the byte-identity
      fixtures. Diff them: `.ncode` only, with no runtime-output change.
- [ ] Re-run `benchmark` to regenerate `benchmark/baseline` by the normal procedure,
      and confirm `compress crc32` ≤ 8.5 ms and `compress inflate` ≤ 12.6 ms.
      Re-measure `deflate`/`gzip` and record them.
- [x] Update the dated tables in `src/docs/spec/stdlib/20_compress.md` "Measured
      throughput" and the "≈86 MiB/s" CRC figure, via `tools/compress-bench/run.sh`.
- [x] `tests/interop/rt_compress_interop.rs` and `tools/oracles/compress` green.

Acceptance: full suite green; golden deltas are code-only; the reproductions meet G1–G3.
Commit: — (golden regeneration and the benchmark baseline are left to the batch merge; see
*Resolution*, "Not done here").

## Validation Plan

- Regression tests: `tests/runtime/rt_compress_throughput.rs` (timing ratios) and the
  `.ncode` structure asserts. Also the `call_clobber_mask` unit test in
  `src/codegen/engine/regalloc/tests.rs`.
- Runtime proof: `/tmp/dbF-crc` and `/tmp/dbF-inf` probes (sources above) meet G1–G3;
  the benchmark rows move as stated.
- Doc sync: `mfb spec stdlib compress` measured-throughput tables. `mfb spec memory`
  needs no change (no contract moves). `.ai/codegen-invariants.md`: replace the "every
  `_mfb_*` call destroys all integer registers" rule with the per-helper table. `mfb man`:
  none (no observable change).
- Full suite: `cargo test` (full), `scripts/artifact-gate.sh target/release/mfb all`.

## Open Decisions

- **Hardware CRC.** Recommended: keep it out (plan-137-A §1/§3: no native code, the
  same bytes everywhere by construction), and accept a D grade for `crc32`, which is
  bounded by the software algorithm (C `-O2` slicing-by-8 is 4.35 ms = 18.8× zlib).
  Alternative: an internal native CRC fast path (AArch64 `crc32b/x` compute this exact
  CRC-32/ISO-HDLC polynomial; x86-64 SSE4.2 `crc32` is CRC-32C and would need PCLMUL
  folding). Same results, spec-compliant (internal only), but it reverses a recorded
  user decision, so it needs the owner. This is the only route to a C/B grade on
  this row.
- **B1 scope.** Recommended: a per-helper table for the three audited helpers. The
  alternative, making every `_mfb_*` helper PCS-clean and dropping `ALL_PHYS`, is a
  larger audit across hand-written helpers.

## Summary

Both rows are MFB's own overhead on a correct algorithm, not a wrong algorithm. For
crc32 the overhead is per-access checks and address arithmetic, 2.5× C `-O0` on the
same slicing-by-8. For inflate it is per-byte appends whose loop state the allocator
spills, because every runtime-helper call is modeled as clobbering all registers.
Around that sit a division on the bit-buffer chain and a per-call table build. The
risk is in B1 (the regalloc clobber model is tree-wide and soundness-critical: audit
before loosening) and in the golden churn it causes. The decoder source changes
(B2–B4) are local and output-identical by construction. The spec, the memory model,
the public API and the output bytes are untouched. The last ~19× to zlib on crc32 is
hardware acceleration, left as an owner decision.

## Resolution (2026-10-04)

All four goals hold. Every number below is `macos-aarch64`, a release `mfb` built from the commit
named, `mfb build -O 1`, measured with the 1-minute load average stated (`uptime`, 12-core host).

| goal | probe (best of N, in-process) | before (`e089c1c85`) | after (`27b8bf00c`) | target |
|---|---|---|---|---|
| G1 | `/tmp/bs13-probe` "crc32 x64 (8 MiB)" (best of 7, load 4.5) | 13,650 µs | 7,208 µs | ≤ 8,500 |
| G1 | benchmark row `compress crc32`, min of 10 (best of 2 runs, load 5.4) | 13.754 ms | 7.081 ms | ≤ 8.5 |
| G2 | `/tmp/bs13-probe` "inflate only x16" (best of 7, load 4.5) | 14,192 µs | 4,614 µs | ≤ 5,000 |
| G2 | benchmark row `compress inflate`, min of 10 (best of 2 runs, load 5.4) | 17.708 ms | 6.386 ms | ≤ 12.6 |
| G3 | `/tmp/bs13-probe` "tiny inflate x1000" (best of 7, load 4.5) | 20,452 µs | 974 µs | ≤ 5,000 |
| G4 | outputs, checksums, errors | — | identical (below) | identical |

`/tmp/bs13-probe` is this doc's reproduction B harness plus reproduction A's `crc32 x64` loop; the
benchmark rows come from `benchmark/mfb` with `main` cut down to the four `test_compress_*` calls
(`--run 10`; checksums `test_compress_*` identical to `benchmark/baseline/mfb-O1.sums`). Against the
committed C `-O0`/zlib baseline (crc32 0.232 ms, inflate 0.841 ms) the rows are now **30.5×** (was
58.1×, F) and **7.6×** (was 21.7×, D). Throughput: CRC-32 0.86 ns/byte (≈1,160 MB/s; zlib ≈36 GB/s,
hardware-assisted), inflate 2.2 ns/output byte (≈455 MB/s). At `-O2`/`-O3` (same harness, load 3.6):
crc32 12.51/12.06 → 5.69/6.43 ms, inflate 14.72/14.54 → 5.72/5.73 ms. The other two rows moved too
(-O1, same run): `deflate` 43.86 → 32.34 ms, `gzip` 49.64 → 34.43 ms.

`tools/compress-bench/run.sh <mfb> inflate zlibDecode gzipDecode crc32` (load 3.6–5.7, 16 MiB, `-O1`):
`inflate` 81.4/49.9/69.5 → 319.5/825.3/1,788.5 MiB/s (random/text/zero), CRC-32 ≈86 → ≈1,100 MiB/s;
`deflate6` text 13.1 → 64.1 MiB/s. The spec's dated tables are re-measured with these figures.

### Root causes, and what fixed each

| # | cause | fix | commit |
|---|---|---|---|
| A3 | a fixed-width `get` was `cmp_imm; b.lt; ldr; cmp; b.ge; mov_imm; mul; add_imm; add; ldr` | one unsigned `cmp count, idx; b.ls`, `[lsl_imm]; add; ldr [x, #40]` | `ac558a36f` |
| A1 | a literal shift count was range-checked at run time | literal (or `band` with a literal ≤ 63) count: immediate shift / no check | `35cbc8c7f` |
| — | every IF/WHILE materialized a Boolean (7 instructions, 2 branches) | fuse into `cmp; b.!cc` | `a0567278c` |
| A5 | every check's raise block sat inline, jumped over by the passing path | raise blocks after the body (`emit_cold_block`) | `207a6ed31` |
| A2/A4 | the CRC table was re-loaded from its module global on every lookup | the loop takes the table as a parameter; indexes by `bits::bor` | `27b8bf00c` |
| B1 | `_mfb_arena_*`/drop calls modeled as clobbering every register, so a loop's state across a rejoining grow path lived in memory | audited `PCS_CLEAN_RUNTIME_HELPERS` clobber only caller-saved | `9a2a09933` |
| B2 | 99.7% of the output went through a per-byte checked append + get | `append(out, mid(out, from, chunk))`, lowered as an in-place copy with no temporary | `cd55d5cf5`, `27b8bf00c` |
| B3 | `MOD 16`/`/ 16` on the bit-buffer chain | `bits::band(e, 15)` / `bits::sr(e, 4)` on non-negative entries | `27b8bf00c` |
| B4 | the fixed tables were rebuilt on every call (the whole cost of a small inflate) | module globals built once | `27b8bf00c` |

A2's "BCE for the module table" was not implemented: the table reads keep their (now one-compare)
bounds check; the parameter and A3/A5 made it cheap enough. Hardware CRC stays out (Open Decisions).

### Phase 1 audit: callee-saved preservation (B1)

Read from each target's `-ncode` (`tests/runtime/rt_runtime_helper_callee_saved.rs`, which walks every
`bl` into another `_mfb_*` body):

| helper (and callees) | macos/linux aarch64 | linux x86_64 | windows x86_64 | linux riscv64 |
|---|---|---|---|---|
| `_mfb_arena_alloc` (+ `_fill_random`, `_flush_coalesce`, `_insert_free`) | saves x21–x27 it uses; reads x19 | saves r12/r14; reads r15 | saves r12/r14; reads r15; `VirtualAlloc` (Win64 ABI) | saves s1–s9 it uses; reads s11 |
| `_mfb_arena_free` | no callee-saved write; reads x19 | saves r12/r14 | saves r12/r14 | saves s1 |
| `_mfb_rt_drop_owned_collection` | saves x21 | saves r12/r14 | saves r12/r14 | saves s1 |

Verdict: all three are PCS-clean on every target. The comment that `_mfb_arena_alloc` "tramples
x20–x28" was wrong and is gone; `.ai/compiler.md` was right.

Blast radius, `crypto::`: `crypto::hash(SHA2_256, 1 MiB)` best of 5 went 35.3 → 27.6 ms with no
change to its source (`/tmp/bs13-sha`, load 20 — indicative only).

### Equivalence evidence (G4)

- A 293-line verification program (`/tmp/bs13-xv`: round trips of 10 payloads at every level in all three formats,
  every `maxBytes`/truncation/garbage/stored-block error, a 10-variable loop with grows, every new
  fast path's raise — overflow, index, shift count, `toByte` — and integer/float comparisons) prints
  byte-identical output under the base and fixed compilers at `-O0`..`-O3`, and on
  linux-aarch64 glibc (2226), linux-riscv64 musl (2229) and windows-x86_64 (206).
- 150,000 seeded 1–3-edit mutations of 48 deflate/zlib/gzip streams (with a small `maxBytes` on every
  fifth): identical verdict hash (1476612783: 23,590 ok, 119,673 ErrInvalidFormat, 6,737 ErrTooLarge,
  0 other) under the base and fixed compilers locally and on 2226, 2229 and 206.
- `tools/oracles/compress/run.sh` all modes (1,057 cases) and `mutate` (600): 0 failures.
- `tests/interop/rt_compress_interop.rs`, `rt_compress_bounds`, `rt_compress_deflate_o3`: green.
- `scripts/linux-runtime-proof.sh` `FILTER=compress` on 2226 and 2229: 9/9; `FILTER=rt-error/` on
  2229 154/154 and on 2226 153/154 (the failure, `func_net_ping_range_invalid`, fails identically with
  the base compiler: the box cannot send ICMP); `FILTER=rt-behavior/collections` on 2229: 66/66.
- `cargo test --release --bin mfb` (all 4,738 unit tests) green; 37 targeted integration test
  binaries green.

### Expected golden moves (not regenerated here)

- `.ir` of every fixture that imports `compress` and reaches the changed helper bodies (the injected
  source changed): the 10 under `rt-behavior/compress`, `rt-error/compress` and
  `byte-identity/compress` in the targeted `scripts/test-accept.sh` run.
- `.ncode`/`.mir` of codegen fixtures: tree-wide, since `get`, the shifts, IF/WHILE conditions, the
  raise layout and the arena-call clobber model all change emitted code. Seen in the targeted run:
  `collections/func_map_getor_hash_probe` and `collections/list-ops-codegen-rt` (`.ncode`),
  `control-flow/control-flow-if` (`.ncode`, `.mir`).
- Every `tests/byte-identity/**` `.ncodesum` on every target (artifact-gate).
- No runtime output (`build.log`, `.out`) changed in any targeted run; the one `build.log` mismatch
  (`rt-error/native/native-cbuffer-overrun-rt`) is environmental — it writes under `./target`, absent
  in this worktree, fails identically with the base compiler, and passes once `target/` exists.

### Not done here

- `benchmark/baseline` is not regenerated (a whole-suite, all-language run); the rows above were
  measured with the compress-only harness.
- The golden regeneration and the full suite / artifact gate are left to the batch merge, per the
  batch instructions.
