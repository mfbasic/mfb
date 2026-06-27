# MFB Memory Layouts

This document specifies runtime memory layouts used by the compiler and native
runtime. These layouts are implementation contracts, not source-level syntax.

## Goals

- Owned values have a clear, copyable memory representation.
- Collections are represented as one contiguous allocation.
- Copying a collection snapshot can be implemented as one contiguous memory copy.
- Collection mutation can minimize payload copying by moving lookup metadata
  instead of moving packed item bytes.
- The collection layout favors one uniform representation over collection-kind
  specialization.

## Reading order

The topics below build up from the smallest units to the largest. `scalar-storage`
fixes the payload sizes; `fallible-call-abi` is the **single source of truth for the
four-register fallible-call result ABI** (other specs summarize and link here);
`heap-values` specifies the compact object bodies (strings, records,
errors, results, unions); `arenas` is where every heap value lives and how it is
allocated, freed, filled, and reclaimed; and `collections` specifies the one
uniform `List`/`Map` layout, its examples, operations, and compaction. The
remaining topics specify the native ABI: `native-calling-convention` (the custom
non-AAPCS64 register/stack-frame model), `runtime-helper-abi` (the
`_mfb_rt_*` helper signatures), `program-startup` (the generated entry/teardown
sequence), and `closures` (the function-value/closure object layout).

## See Also

* ./mfb spec architecture — where memory lowering sits in the pipeline
* ./mfb spec language memory-semantics — the source-level ownership model
* ./mfb spec package — on-disk value and type encoding
* ./mfb spec threading — per-arena thread isolation
* ./mfb spec linker — native emission of these layouts
