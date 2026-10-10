I've placed all 157 open bug documents in `bugs/` into 11 phases (counted with `python3 -c 'from pathlib import Path; print(len(list(Path("bugs").glob("bug-*.md"))))'`). The phase lists were compared against the open bug documents, with no bug missing or listed twice. I worked from each bug's header (title, severity, effort, class, owner decisions) and the references between bugs. I did not re-run each repro, so some of these may already be fixed. `/fix-bug` re-runs the repro first, so that will show up when each one is picked up.

## How I ordered them
1. **Crashes and security holes first**, when the fix is small and doesn't sit inside a planned redesign.
2. **Language redesigns come before the bugs around them.** bug-976 (remove the optional `RES` keyword on parameters), bug-975 (thread worker handles as resources) and bug-977/978 (Byte typing) change the rules that neighbouring bugs would otherwise be fixed against.
3. **Group by subsystem**, so related fixes land together and the regression test suites stabilise once per area.
4. **The big `Operand::Raw` refactor (bug-946) goes after the behaviour fixes**, so its output diffs are measured against a stable tree.
5. **Spec-drift bugs close last in their area.** The project rule already requires spec updates in the same change. The leftover drift bugs are sweeps, and several cite code that earlier phases change.

## Phases

Legend: `[x]` in main · `[~]` fixed and merged into the staging branch, not yet in main · `[ ]` open · `[-]` waiting on dependencies.


**P1: Crashes and security fixes (independent)**
[x] Memory safety: 841 + 898 (both `EXIT PROGRAM`; fix together), 897, 895, 962, 967
[x] Security: 859, 981, 885, 862, 902, 868
[x] High-severity correctness: 844 is deferred to P2. This phase has 858, 861, 963, and 957 (needs the riscv64 box).

**P2: Resource and thread model** (batch 17; in main at 42f635656)
[x] 976 → 975 (owner-confirmed language changes; 975 depends on 976's rule): fixed, reviewed, merged into staging
[x] 843, 842, 852, 854, 899: fixed, reviewed, merged into staging (842 scoped out inline aggregates → bug-999)
[x] 844, 933, 935, 851: fixed, reviewed, merged into staging
[x] integration fix: a closure may not capture a ThreadWorker (975 × 843 interaction; capture check now shares the List-element predicate)
[ ] 856 moved to P2b (owner re-decided: remove the cleanup table and its format fields, version bump; derive the audit)
[x] Golden proofs done (131 goldens, each attributed), committed in staging
[x] Full gate #1: 2 real failures (bug-852 over-carried a consuming call's take-over round the loop) → fixed by agent F, reviewed, merged; + 2 pre-existing flakes filed (1003, 1004)
[x] Full gate #2 green; merged to main. Paused, then resumed by the owner (2026-10-07).

**P3: Networking and HTTP**
[ ] 979 first: it takes over the validation half of 872 and shares an oracle with 868 (if 868 didn't land in P1).
[ ] Then 872 (the connect half), 980 (builds on 859's rule), 864, 865, 866, 867, 876
[ ] Close with 873 (the spec drift for this area)

**P4: Modules, visibility and templates**
[ ] 912 → 881, 848, 849, 911 (large), 913, 914, 833, 924, 903, 920

**P5: Inline TRAP, internal compiler errors, diagnostics**
[ ] 860 (large; compile memory blows up with many inline TRAP handlers), 835 → 874, 870, 900, 901, 915, 846, 875, 877, 882, 832, 847, 880
[ ] Message-quality only: 836, 837, 838, 887, 936, 937

**P6: Language changes (owner-confirmed)**
[ ] 977 → 878, then 978 + 839, then 945 (large; `Result`/`Ok` reserved in user code)

**P7: Standard library correctness**
[ ] Collections: 1006 (`collections::window` overflows its next-window bound for a valid large stride), 1007 (fixed-width `chunks` fast path overallocates short chunks), 1009 (fixed-width `toSet` reserves by input length), 1010 (native `merge` reserves for overlapping input keys), 1011 (reserved collection results retain source-sized capacity when outputs are smaller)
[ ] Numbers: 831, 931, 932, 934, 845
[ ] Text and formats: 1005 (`strings::repeat` on an empty string still loops `times` times), 827, 828, 829, 869, 871, 863, 905, 906, 879, 888
[ ] I/O and OS: 891, 892, 893, 886
[ ] Collections/strings capacity (filed on main 2026-10-07): 1005, 1006, 1007, 1008, 1009, 1010, 1011

**P8: App, terminal and canvas**
[ ] 966 → 965, 964, 969, 970, 971, 968
[ ] Owner-decided: 972 (new `app::shouldClose`), 973
[ ] 960 (signing `--app` bundles)
[ ] Last, both huge performance items: 688 (its remaining Linux work is blocked on a real-GPU Linux box), then 765 (cites 688)

**P9: Package registry and tooling**
[ ] Registry: 954 (also covers 947), 948, 949, 950 → 956, 951, 952, 953, 955, 940
[ ] CLI: 850, 919, 918, 925 → 941, 916, 943, 853, 907, 908, 909, 923, 921, 922, 926, 939, 942
[ ] Platform and linker: 958, 959

**P10: Codegen internals**
[ ] 946 (huge refactor) together with 930
[ ] 855 (quadratic copying when walking recursive unions)
[ ] 1008 (large valid constant `chunks`/`window` sizes panic in native code generation)

**P11: Spec-drift sweep**
[ ] Any order: 830, 840, 883, 889, 890, 894, 896, 904, 910, 927, 944, 961, 974
[-] These must wait for their dependencies:
  [ ] 857 needs 975.
  [ ] 884 needs 840, 857 and 873.
  [ ] 938 needs 977 and 978.
  [ ] 917 needs 930, 945 and 946, so it is the very last bug.

## New bugs filed since the plan

**P2b: Memory safety, ABI and silent wrong values** (batch 18, `bug-staging-18`; in progress)
Round 1: agent I done (982+993, 994, 983 merged into staging); agent J on 989 and 988's reduce part. Round 2: agent K on the leaks; then 856 and 1012. Round 2: leaks (992, 984, 999, 1000, 1001) and 856.
[~] 982 + 993: mouse callbacks (and the graphics-thread entry, thread trampoline, OpenSSL verify callback) keep callee-saved registers; reviewed, merged into staging (2226: click/move/scroll rc 139 → 0)
[~] 994: Win64 frames save the callee-saved xmm registers they write; reviewed, merged into staging (206: `20.00` no longer becomes `-0.09` / `4294967296.00`)
[~] 990: fixed, reviewed, merged into staging (a variant from a call/field/get/WITH is now wrapped everywhere)
[~] 988: `append`/`insert`/`set`/… accept a variant (merged into staging); `reduce` with a variant initial value: owner decided it works → agent J
[ ] 989: owner decided CONVERT (a `List OF Square` given as `List OF Shape` is copied and each element tagged; same for Set/Map values/nested) → agent J
[~] 1016 (was 1013): a user `TYPE Circle`/`Rectangle` broke the canvas helpers; found and fixed by agent J, merged into staging
[~] 983: `fs::pathJoin` heap overrun after an absolute component; reviewed, merged into staging (rc 138 → `403` at -O0..-O3)
[ ] (agent K, in progress) 992, 984, 1000, 1001: leaks (`graphemesCount` list; macOS TLS listener 2.8 KB; owned-list drain I/O buffer; union drop leaks read and write buffers, up to 16 KiB per drop)
[ ] (agent K, in progress) 999: an inline aggregate of resources is never closed (`len(mkList())`)
[ ] 1013 (HIGH, filed by the plan-172 session, in its worktree, not on main yet): a `STATE` type that is not a record is accepted, then fails to build (scalar) or crashes (`String`). Owner ruling: **a STATE must always be a record**; anything else is a compile error (`TYPE_STATE_INVALID`) at every position a `STATE` clause can appear. Pairs with 998 (STATE on a non-resource parameter → `TYPE_STATE_REQUIRES_RESOURCE`).
[ ] 1012 (HIGH): a resource-union box has no owner outside a binding (record-field use-after-free, exit 139; 16 B leak); found by agent J, crash reproduced by the coordinator
[ ] 856: remove the `.mfp` cleanup table + `FUNCTION_TABLE` cleanup fields (format bump); derive the secondary-close audit

Into existing phases:
[ ] P3: 986 (HEAD waits for EOF; owner decided: `method` field on `http::PendingState`)
[ ] P3: 1017 (was 1014) (Windows `respondPath`: `..\` traversal still 403-vs-404 by existence; reproduced on 206)
[ ] Flakes: 1003 (http::pump example hangs on a stalled remote), 1004 (cli_dbg threaded-arena test timing)
[ ] P5: 1002 (TYPE_COLLECTION_OWNERSHIP_VIOLATION reported twice)
[ ] P5: 998 (STATE on a non-resource parameter; owner decided: new rule `TYPE_STATE_REQUIRES_RESOURCE`)
[ ] P8: 1015 (GTK `pollMouse` sees nothing without a stdin read; owner decided: events come from the window like macOS/Windows) → then 985 (setMode window has no mouse controllers)
[ ] P8: 991, 995, 996, 997
[ ] Standalone: 987 (Windows test scripts → box 206)

Owner decisions recorded: 986 (method field), 902 (keep `abort()`), 987 (repoint to 206), 998 (dedicated rule), 856 (remove the cleanup table, derive the audit), 989 (convert, never reject; one direction only), 988 reduce (works), 1015 (GTK mouse events come from the window), 1013 (a STATE must always be a record).
Not run (boxes down): 962 on x86_64 GTK / musl GTK; 902 on x86_64 glibc.

Numbering note: other sessions file bugs too (the plan-172 session used 1013 and 1014 the same day the coordinator did). The coordinator renumbers its own unmerged docs on a collision; check `ls bugs` in every worktree before taking a number.
