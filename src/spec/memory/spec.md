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
fixes the payload sizes; `fallible-call-abi` is the register contract for native
calls; `heap-values` specifies the compact object bodies (strings, records,
errors, results, unions); `arenas` is where every heap value lives and how it is
allocated, freed, filled, and reclaimed; and `collections` specifies the one
uniform `List`/`Map` layout, its examples, operations, and compaction.
