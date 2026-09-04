# MEM-70 spike — cross-thread arena allocation race in `thread::send`

audit-3 MEM-70 (`planning/audit-3-codegen-memory.md`), bug-498.

```
mfb build spikes/audit-3/MEM-70 && ./spikes/audit-3/MEM-70/build/mfb_project.out
```

The parent sends 200 000 messages to a worker while the worker allocates in a
tight loop. `thread::send`'s lowering repoints the pinned arena register (`x19`)
at the *worker's* arena and deep-copies the message there — with no lock — while
the worker is allocating from the same arena. The unsynchronized quick-bin
free-list pop hands out the same block twice or overwrites a `next` link.

## Observed (defect present)

```
Segmentation fault: 11   (exit 139), reproducibly — 3/3 and 5/5 runs
```

lldb shows both threads faulting at the same PC in `_mfb_arena_alloc`'s quick-bin
pop on a garbage link. `grep -rn 'mutex|_lock|atomic|ldxr|stxr|compare_exchange'
src/codegen/memory/arena/` returns nothing — the allocator has no synchronization.

## Expected (after fix)

`worker returned <n>` and exit 0 — a cross-thread send must copy into a
process-global / locked region, not allocate from the peer thread's live arena.
