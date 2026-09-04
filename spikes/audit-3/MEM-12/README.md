# MEM-12 spike — operand aliasing UAF: `g = <op>(g, f(…))` where f reassigns g

audit-3 MEM-12 (`planning/audit-3-codegen-memory.md`), bug-496. The general
argument-aliasing half of open bug-487, reached with a plain global and no `RES`.

```
mfb build spikes/audit-3/MEM-12 && ./spikes/audit-3/MEM-12/build/mfb_project.out
```

`GS & other(1)` lowers operand 0 (`GS`) to a pointer into `GS`'s block, then
evaluates operand 1 (`same(1)`), which reassigns `GS` — freeing operand 0's
block. The concat then reads the freed block's `byteLength` at offset 0 (which
`arena_free` has overwritten with the quick-bin link).

## Observed (defect present, identical at -O0 and -O3)

```
other -> [abcdefghtail] len=12     # other() reassigns a DIFFERENT global -> correct
same  -> [tail] len=4              # same() reassigns GS (the left operand) -> UAF, bytes lost
```

Collection variants are louder: `g = collections::append(g, evict(x))` where
`evict` reassigns `g` reads `COUNT`/`DATA_LENGTH` out of recycled free-node words
and aborts with `7-701-0001 Allocation failed`.

## Expected (after fix)

Both lines `abcdefghtail` / `len=12` — operand 0 is `GS` as it was before the
call (correct value semantics per `.ai/collections.md`); losing the *nested
write* is fine, losing operand 0's *bytes* is the bug.

## Control

`other()` (reassigns a different global) prints the correct result — proving the
difference is the aliasing of operand 0, not evaluation order.
