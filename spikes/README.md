# plan-121 spikes

Minimal MFBASIC programs that establish the root causes behind plan-121. Each
holds the amount of work constant and varies N, so it reports a *complexity*, not
a single number — a scaling curve is what distinguishes "slow constant" from
"wrong algorithm", and the plan's sub-plan split rests on that distinction.

Build and run any one with:

    mfb build spikes/sN && ./spikes/sN/build/mfb_project.out

| spike | question | answer |
|---|---|---|
| `s1` | what does `collections::set` cost, and does it scale with N? | plain Integer local flat at ~50 ns; the same update through a record field rises 951 → 12844 ns/set over N = 50…1600 |
| `s2` | why is String `set` sometimes O(1) and sometimes not? | same-length write is flat (~40 ns); any length change is O(N^1.6) — 173 → 155037 ns/set over N = 50…3200. Shorter is as bad as longer |
| `s3` | is the String fold O(N²), and do insert/removeAt/prepend have an in-place path? | fold is O(N²) for String, O(N) for Integer; `append` is flat at 12–26 ns while `prepend`/`insert`/`removeAt` all rise linearly with N |
| `s4` | is the O(N²) fold inherent, or is the fast mechanism unreachable? | unreachable — the hand loop is exactly linear and `reduce` is quadratic; **790× apart at N=8000** for the identical fold |
| `s5` | where does `isSuperset`'s 410× come from? | **neither hypothesis held.** `contains` is a hash probe (flat 60→81 ns over a 64× size range) and the hand-written loop costs the same as the builtin (1936 vs 1915 µs at N=1600). The gap is early-exit iteration order — see plan-121-E |

`s5` is the reason these exist: it refuted the sub-plan that had already been
drafted for it, before any code was written.
