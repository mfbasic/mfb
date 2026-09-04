# MEM-11 spike — bounds-check elision reads out of bounds on a stale length

audit-3 MEM-11 (`planning/audit-3-codegen-memory.md`), bug-495.

```
mfb build spikes/audit-3/MEM-11 && ./spikes/audit-3/MEM-11/build/mfb_project.out
```

The inner `FOR i = 0 TO n - 1` has its `collections::get(xs, i)` bounds check
elided because `n = len(xs)` is a proven index bound — but `xs` is reassigned to
a shorter list by the *enclosing* loop's back edge, which the proof does not see.

## Observed (defect present, identical at -O0 and -O3)

```
out=24
v=1 … v=8        # pass 0, correct
v=99 v=0 v=80 v=7099613721752949849 …   # pass 1+: 14 words of adjacent heap
```

24 elements are produced from an 8→1-element list; passes 1 and 2 read past the
entry table and copy arena/heap words into a program-visible list (an info-leak
primitive). With `List OF String` the same shape dereferences the OOB
(offset,length) pair and segfaults.

## Control

Replacing `TO n - 1` with the literal `TO 7` (so the len-fact recognizer does
not fire) raises `7-705-0001 index/range is outside valid bounds` — proving the
difference is the elision, not a missing check in general.

## Expected (after fix)

`ErrIndexOutOfRange` (`7-705-0001`) on the first out-of-range `get` in pass 1.
