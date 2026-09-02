# Compile-speed report — where `mfb` spends its time

**Status:** report only. Nothing here is a plan, a schedule, or a commitment; the
recommendations at the bottom are a ranked list of what the measurements
justify looking at, not a design for any of them.

**Date:** 2026-08-30
**Host:** macOS 24.6.0, arm64 (`macos-aarch64`)
**Subject:** `tests/acceptance` — 22 source files, 28,982 NIR statements
**Instrument:** `mfb test -vv` (the compile profiler added alongside this
report; see `mfb spec tooling cli-reference` and `src/trace.rs`) plus a
`/usr/bin/sample` capture of a debug build for leaf-level attribution.

Every number below is from one of:

```sh
# debug compiler   (what a `cargo build` produces, and what the report was opened against)
cd tests/acceptance && ../../target/debug/mfb   test -vv
# release compiler
cd tests/acceptance && ../../target/release/mfb test -vv
# leaf profile of the debug run
sample <pid> 240 1 -file /tmp/mfb-sample.txt
```

---

## 1. Headline

A release build of `tests/acceptance` takes **68.8 s**. A debug build takes
**448 s**. The 6.5× is real and worth taking, but it is a constant factor on top
of a structural problem, and the structural problem is this pair of counters:

```
NIR statements                  28982
machine instructions         16957344
```

**585 machine instructions per source statement; ~10,500 per function.**

Every stage after NIR — register allocation, both peepholes, FMA fusion, plan
validation, branch relaxation, image encoding, linking — is linear or worse in
that 17 M. The back end is not slow *per instruction*. It is being handed an
instruction stream an order of magnitude larger than the program.

## 2. Phase totals

| phase | debug | release | speedup |
|---|---:|---:|---:|
| parse | 0.06 s | 0.02 s | — |
| resolve | 31.2 s | 14.6 s | 2.1× |
| verify | 1.9 s | 0.5 s | 3.5× |
| codegen+link | 414.9 s | 53.8 s | 7.7× |
| **total** | **448 s** | **68.8 s** | **6.5×** |

## 3. Is it just the debug build?

No. If debug overhead were the whole story every row would speed up by roughly
the same factor. The spread is 2.1× to 10.9×:

| row | debug | release | speedup |
|---|---:|---:|---:|
| **monomorphize** | 30.5 s | **14.4 s** | **2.1×** |
| peephole: store-to-load | 5.0 s | 1.5 s | 3.4× |
| verify rules | 1.1 s | 0.3 s | 3.5× |
| merge packages | 1.8 s | 0.4 s | 4.0× |
| emit ops | 10.8 s | 2.3 s | 4.6× |
| linking executable | 30.1 s | 6.3 s | 4.8× |
| route helpers through MIR | 8.6 s | 1.6 s | 5.3× |
| fma fusion | 13.0 s | 2.0 s | 6.4× |
| encoding image | 113.8 s | 13.7 s | 8.3× |
| lower_function (total) | 160.7 s | 19.0 s | 8.4× |
| peephole: fp shuttles | 16.4 s | 1.8 s | 9.3× |
| planning + regalloc | 18.1 s | 1.9 s | 9.7× |
| finalize frame | 7.5 s | 0.75 s | 10.0× |
| **coloring** | 86.4 s | 7.9 s | 10.9× |

Two readings:

* The **8–11× rows** (coloring, encoding, regalloc planning, `finalize_frame`)
  are doing cheap work in a tight loop. `-O0` Rust makes each iteration
  expensive and release genuinely fixes them. They are *volume* problems, not
  algorithm problems.
* **`monomorphize` at 2.1× is the outlier**, and it moves the wrong way in
  relative terms: 7 % of a debug build, **21 % of a release build**. A row that
  barely benefits from optimized Rust is not bottlenecked on per-operation
  overhead — it is bottlenecked on allocation, hashing, or an algorithm.

Caveat: `relax branches`, `validate code plan`, and the module-level rows in
§4 were instrumented after the debug capture, so they have no debug column.

## 4. Release build, by row (68.8 s total)

```
resolve                                       14583.0ms  21.2%
  monomorphize                                14353.4ms  20.9%
  resolve_project                               124.4ms
  augment_project                                85.0ms
verify                                          466.4ms   0.7%
codegen+link                                  53766.2ms  78.1%
  emitting native code                        31414.6ms  45.6%   (self 76ms)
    lower_function                            19024.6ms  27.6%   1616 calls
      register allocation                     10538.6ms  15.3%
        coloring                               7897.3ms  11.5%
        constant folding                        766.7ms
        instruction selection                   649.0ms
        physical-operand scan                   612.5ms
        (22 other Opt2 rows)                      2.1ms   <-- see §6
      emit ops                                 2343.8ms   3.4%
      fma fusion                               2029.2ms   2.9%
      peephole: fp shuttles                    1759.0ms   2.6%
      peephole: store-to-load                  1456.3ms   2.1%
      finalize frame                            748.4ms   1.1%
    relax branches                             5745.1ms   8.3%
    validate code plan                         4692.7ms   6.8%   2 calls
    route helpers through MIR                  1640.4ms   2.4%
    string symbols                              220.5ms
  encoding image                              13717.4ms  19.9%   (no sub-spans)
  linking executable                           6253.5ms   9.1%
  planning + regalloc                          1870.8ms   2.7%
  lowering module                               509.6ms   0.7%
    merge packages                              444.6ms
```

## 5. Findings

### 5.1 The 585:1 instruction expansion is the ceiling on everything else

28,982 NIR statements become 16,957,344 machine instructions. Until that ratio
comes down, every row in §4 below `lowering module` is paying for volume. This
is the single number that, if halved, halves most of the build.

**Correction and root cause (2026-09-01, plan-118-A).** The 585:1 headline is
partly a measurement artifact, and the expansion is concentrated rather than
uniform.

*The ratio.* `NIR statements` is a FLAT counter — `sum(function.body.len())`, so
a loop, `IF` tree or `MATCH` counts as one statement no matter how much is
nested inside it (`src/target/shared/lower.rs`). It undercounts the tree by
1.8x. plan-118-A added `NIR ops (recursive)` beside it. Over the same corpus:

```
NIR statements                  29088
NIR ops (recursive)             52548
machine instructions         17079160
```

so the honest expansion is **325:1**, not 585:1. Still the ceiling; still worth
attacking. The flat counter is kept unrenamed so the numbers above this line
keep meaning what they said.

*Where it goes.* plan-118-A also added a `costliest expansion` tally: each
builder-emitted instruction is attributed to the NIR op / value / call target
that emitted it, exclusive of its children. Five categories are 67.9 % of all
13,175,351 attributed instructions:

```
--- trace: costliest expansion (40 of 1821 keys, 13175351 total, exclusive) ---
     2907604     17221x  binop:Concat        (169 per site)
     2173050      7876x  val:Constructor     (276)
     2007382     11432x  op:Return           (176)
     1030128      5826x  call:toString       (177)
      826446      3193x  rtcall:io.print     (259)
      551939     12407x  op:Bind
      319358      4540x  op:Assign
      255872      3609x  op:Fail
      245967      1828x  binop:Add
```

Each of those is an inline lowering emitted afresh at every site — an inline
arena allocation, an inline ~45-instruction allocation-failure error block, and
(for concat) two byte-at-a-time copy loops. A `RETURN a & b` function is 300
machine instructions. That is what recommendation 3 turns out to mean, and
plan-118-C/D/E out-of-line the top five.

The attributed total (13.18 M) is less than the module total (17.08 M): frame
prologues, regalloc spill code and slot zeroing are emitted outside any
construct's frame, and the peepholes delete instructions after attribution.

Separately measured and deliberately NOT in plan-118's scope: roughly half of
every function's instructions are stack round-trips (in the 300-instruction
`RETURN a & b`, 83 `ldr_u64` + 68 `str_u64` — every intermediate value stored
and immediately reloaded). Fixing that means a values-in-registers builder, a
different investigation.

### 5.2 `monomorphize` is 21 % of a release build and produces almost nothing

The counters straddling it:

```
HIR functions (generic)          1618
HIR functions (concrete)         1609
```

Fourteen seconds to turn 1,618 generic functions into 1,609 concrete ones —
*nine fewer than it started with*. Combined with the 2.1× release speedup (§3),
this is the one row in the profile whose shape suggests a defect rather than a
scaling problem. Not yet root-caused.

### 5.3 Three generated functions are 18 % of all function lowering

```
--- trace: slowest lower_function (20 of 1616, 19024.5ms total) ---
  1435.1ms  #regex_genCat
  1383.6ms  #strings_genCat
   618.5ms  #regex_scriptOf
   256.8ms  __mfb_test_case_94
    89.4ms  __mfb_test_chunk_24
    ...      (the remaining ~1600 sit at 20–90 ms each)
```

Three functions out of 1,616 account for 3.44 s of the 19.0 s spent lowering
all of them. They are generated Unicode category/script tables. The tail is
flat and unremarkable, which is what says the problem is these three
specifically and not a general per-function cost.

### 5.4 Register colouring is dominated by string comparison

The `sample` capture of the debug run puts these at the top of self-time:

```
core::str::...::eq                          8217
alloc::vec::partial_eq::...::eq             7177
_platform_memmove                           6569
core::str::...::eq                          6516
_platform_memcmp                            4803
slice::Iter::any                            3320
```

and the mfb-side call tree underneath `coloring`:

```
regalloc::allocate
  linear_scan::run
    analysis::analyze
    analysis::effect -> analysis::classify_ref
      analysis::int_physical_index
        analysis::int_concrete_physical_index
          analysis::riscv_int_index      <-- on a macos-aarch64 build
      analysis::fp_physical_index
        analysis::riscv_fp_index
```

`int_concrete_physical_index` (`src/codegen/engine/regalloc/analysis.rs:291`)
decides register identity by string inspection: a `%`-prefix fast-reject
(added by plan-78-C for exactly this reason), then an `xN` parse, then a linear
`position` scan over the 16-entry `X86_GPRS` table, then `riscv_int_index`'s
32-entry scan. Any name that is not `%`-prefixed and not `xN` — `sp`, `lr`,
`w0`, the FP names — walks up to 48 string comparisons, on every operand, on
every instruction, on a host that is none of those architectures.

This is the micro-level reason colouring is the largest single codegen row. It
is also the row with the largest debug penalty (10.9×), so release already
takes most of it back; the remaining 7.9 s is the honest cost.

### 5.5 `relax branches` scans 17 M instructions to do nothing

5.7 s (8.3 %) in release. `relax_conditional_branches`
(`src/arch/aarch64/encode/relax.rs:68`) relaxes every function to a fixpoint so
that an out-of-`imm19`-range conditional branch becomes a trampoline hop. For
every program that compiled before bug-445 — i.e. essentially all of them — it
rewrites nothing and the entire cost is the scan.

### 5.6 The whole code plan is validated twice

`validate code plan` reports **count 2**: `NativeCodePlan::validate` runs once
as the tail of `lower_module_for_platform` and again on the returned plan in
each backend's `write_executable`. 4.7 s across both calls in release. One of
the two is redundant by construction.

### 5.7 `encoding image` is 20 % of the build and completely opaque

13.7 s in release, second-largest row after `lower_function`, and it currently
has no sub-spans at all — the profiler can say only that the time is in there.
Nothing is known about its internal shape.

### 5.8 The optimizer is not the problem

All 25 gated Opt2 rows together cost **2.1 ms** across the whole build. They are
off at `-O1`. The `Peephole optimization (store-to-load forwarding): 485647`
line in a `-v` build is the *post-regalloc* machine peephole, which is a
different pass at a different seam and costs 1.5 s — not one of the dial rows.
Any speed work aimed at the `-O` pipeline would be aimed at 0.003 % of the
build.

### 5.9 First `strings::upper` costs a fixed 1.63 s (debug)

`UnicodeCaseMap::entry_count` calls `unicode::runtime_tables::tables()`, a
`OnceLock` over `parse_tables()`. The whole cost lands on whichever call site
touches it first. Measured on a two-statement program:

| program | `strings.upper` tally | codegen+link |
|---|---|---|
| 1 call | 1634.3 ms, 1× | 1726 ms |
| 4 calls | 1657.2 ms, 4× | 1847 ms |

One-time, not per-call. 238 ms in release across `tests/acceptance`'s 6 call
sites. A fixed tax on any program that touches case mapping; small in absolute
terms, but it is ~95 % of `codegen+link` for a *small* program, which is what
makes trivial builds feel slow.

### 5.10 Front end is not a concern

`parse` is 0.02 s and `verify` is 0.5 s in release — 0.8 % of the build
combined. Nothing to do here.

---

## Recommendations

Ranked by (measured cost) × (confidence the fix is real). Nothing here is
scoped or designed; each is a place the numbers say is worth opening.

1. **Ship/use the release compiler for anything time-sensitive.** 6.5× for
   zero engineering. If the debug binary is what developers and CI actually
   run, that alone is the largest single win available, and it needs no change
   to the compiler at all. *(Note: CI runs a DEBUG `mfb` on Linux — so CI is
   paying the full 6.5× today.)*

2. **Root-cause `monomorphize`.** 21 % of a release build, 14.4 s, to convert
   1,618 generic functions into 1,609 concrete ones, and it barely responds to
   optimized Rust. Everything about its shape says defect rather than volume.
   Highest-value single row that is *not* explained by §5.1.

3. **Attack the NIR→machine expansion.** It is the ceiling on §4's entire
   lower half; halving it roughly halves the back end. Broadest and hardest —
   worth understanding before committing to anything, which is why it sits
   below the two cheap items above rather than at the top. *(Root-caused
   2026-09-01 — see the correction in §5.1: the honest ratio is 325:1, and five
   inline lowerings are 68 % of it. plan-118-A landed the instrument; -B..-E
   attack the categories.)*

4. **Instrument `encoding image`.** 13.7 s / 20 % with zero visibility. Not a
   fix — a prerequisite for knowing whether there is one. Cheap.

5. **Early-out `relax branches`.** 5.7 s / 8.3 % spent proving there is nothing
   to do. A pre-scan for any branch within range of the limit should collapse
   this to near zero for every normal program.

6. **Drop one of the two `NativeCodePlan::validate` calls.** ~2.3 s, and the
   duplication is structural rather than incidental. Needs a decision about
   which seam owns the invariant, not new logic.

7. **Look at `#regex_genCat` / `#strings_genCat` / `#regex_scriptOf`.** 3.4 s in
   three functions. Generated Unicode tables emitted as code; if they can be
   emitted as data objects instead, this row disappears and §5.1 improves with
   it. *(Sized 2026-09-01: 2,556,471 machine instructions, **15.0 % of the whole
   module** — the top three rows of the new `largest lower_function`
   leaderboard. plan-118-B.)*

8. **Type the register operands.** §5.4 — deciding register identity by walking
   x86 and RISC-V name tables with `str::eq`, on an AArch64 host, is the leaf
   cost under the largest codegen row. This is a deep change to how operands are
   represented, so it is listed for the record rather than proposed; note that
   plan-78-C already added one fast-reject here for the same reason, which
   suggests the representation is the recurring issue.

9. **Leave the `-O` pipeline alone.** §5.8 — 2.1 ms. Explicitly listed so that
   the eye-catching `485647` fire count in `-v` output does not send anyone
   here.

### How to reproduce any of this

```sh
cargo build --release
cd tests/acceptance && ../../target/release/mfb test -vv
```

`-vv` prints the span tree, the slowest-function and costliest-builtin
leaderboards, the largest-function leaderboard, the `costliest expansion`
attribution tally, and the size counters. It is print-only: `artifact-gate.sh all`
is 0 diffs over 1780 goldens with it, and
`artifact_bytes_identical_across_verbosity_levels` pins that a `-vv` build
emits the same bytes as a default one.
