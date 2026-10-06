I've placed all 154 open bugs in `bugs/` into 11 phases. The tally is measured: I compared the phase lists below against `ls bugs/bug-*.md` with a script, and no bug is missing or listed twice. I worked from each bug's header (title, severity, effort, class, owner decisions) and the references between bugs. I did not re-run any repro, so some of these may already be fixed. `/fix-bug` re-runs the repro first, so that will show up when each one is picked up.

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

**P2: Resource and thread model** (batch 17, `bug-staging-17`; not in main yet)
[~] 976 → 975 (owner-confirmed language changes; 975 depends on 976's rule): fixed, reviewed, merged into staging
[~] 843, 842, 852, 854, 899: fixed, reviewed, merged into staging (842 scoped out inline aggregates → bug-999)
[ ] 844, 933, 935, 851: fixed by agent G, awaiting coordinator review · 856: in progress
[ ] Then golden proofs + full gate, then merge to main

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
[ ] Numbers: 831, 931, 932, 934, 845
[ ] Text and formats: 827, 828, 829, 869, 871, 863, 905, 906, 879, 888
[ ] I/O and OS: 891, 892, 893, 886

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

**P11: Spec-drift sweep**
[ ] Any order: 830, 840, 883, 889, 890, 894, 896, 904, 910, 927, 944, 961, 974
[-] These must wait for their dependencies:
  [ ] 857 needs 975.
  [ ] 884 needs 840, 857 and 873.
  [ ] 938 needs 977 and 978.
  [ ] 917 needs 930, 945 and 946, so it is the very last bug.

## New bugs filed since the plan

**P2b: Memory safety, ABI and silent wrong values** (owner-approved 2026-10-06; runs after P2, before P3):
[ ] 982 + 993: mouse callbacks overwrite callee-saved `x19` (GTK aarch64 / macOS app); same mistake, fix together
[ ] 994: Win64 functions overwrite `xmm6`/`xmm7` without saving them (silent wrong Float); prove on 206
[ ] 989 + 990 + 988: union-variant values not union-wrapped (crash / garbage / `MATCH` takes no arm); one agent
[ ] 983: string freed wrongly after an absolute `fs::pathJoin` (`http::respondPath("//etc/hosts")` crashes)
[ ] 992, 984, 1000, 1001: leaks (`graphemesCount` list; macOS TLS listener 2.8 KB; owned-list drain I/O buffer; union drop leaks read and write buffers, up to 16 KiB per drop)
[ ] 999: an inline aggregate of resources is never closed (`len(mkList())`)

Into existing phases:
[ ] P3: 986 (HEAD waits for EOF; owner decided: `method` field on `http::PendingState`)
[ ] P5: 998 (STATE on a non-resource parameter; owner decided: new rule `TYPE_STATE_REQUIRES_RESOURCE`)
[ ] P8: 985 (setMode window has no mouse controllers; after 982), 991, 995, 996, 997
[ ] Standalone: 987 (Windows test scripts → box 206)

Owner decisions recorded: 986 (method field), 902 (keep `abort()`), 987 (repoint to 206), 998 (dedicated rule).
Not run (boxes down): 962 on x86_64 GTK / musl GTK; 902 on x86_64 glibc.
