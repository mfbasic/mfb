# bug-02: stale in-place String append into freed List data (regex::replace UAF)

## Status
**FIXED 2026-07-01, commit 75ddad3c.** NOT a use-after-free after all — a
HEAP OVERFLOW: `emit_flat_block_size` omitted the map hash-bucket region
(capacity<<4 bytes), so record-embedded maps (regex `prog.names`) were
constructed/copied/freed 16*capacity bytes short, and the lazy
`_mfb_rt_map_build_buckets` rebuild wrote its `index+1` bucket markers
past the block into the adjacent free-list node (`next` <- 1). Caught
red-handed by breakpoint-logging every free-list node store site plus a
same-run crash-dump correlation (the pc at `place`'s store with the node
address in the base register). One-line-of-math fix in the single size
primitive covers construction, copies, and frees. Verified: replace 5/5
on the box byte-exact, regex+map+thread batch 12/12, aa 975/975.

Companion fix (commit 72782fd5): thread-regex-rt was a second, separate
issue — musl's 128 KiB default worker stack vs the regex engine's ~230 KiB
frame; thread.start now sets an explicit 8 MiB stack via pthread_attr.

(Original OPEN analysis below, kept for the forensic record.)

## Symptom
`func_regex_replace_valid` on linux-x86_64 segfaults during the 14th
replacement (`show("ab","(a)(b)","$10")` in the full sequence; passes in
isolation — heap-layout dependent). Crash: `arena_insert_free`'s
address-ordered walk dereferences a free-list node whose `next` word holds
`1`. AArch64 passes the test — the same stale write lands on a harmless
address there.

## Evidence chain (Alpine box, lldb; process launch --stop-at-entry FIRST)
1. Crash walk: node chain ... → 0x7f22140 {next=0x7f22450, size=0x30} →
   0x7f22450 {next=0x1, size=0x40} → deref 0x1.
2. Conditional breakpoints on `arena_free` (0x80a0ff) and
   `arena_insert_free` (0x809f68) prove 0x7f22450 was NEVER passed to
   either — it became a node via an alloc-split tail remainder.
3. Auto-logging [0x7f22140] at every arena_free shows the pattern twice:
   poison → {0,...} → {1,...} (a LIVE empty String getting one char
   appended in place: len 0→1) → later inserted as a free node. For
   0x7f22450 the len=1 write happens AFTER the containing block was freed
   → the `1` lands on the node's `next`.
4. The freeing call site (return addr 0x4de0df) is an owned-value
   scope-drop in `_mfb_ifn_regex_compile` freeing a `List` (40-byte
   entries; guard slot 120 x86 / 104 aa — same logical slot, +16 frame
   shift). The drop itself is structurally identical on both ISAs and is
   believed CORRECT (the list is a dead temp).
5. Therefore: some String variable's buffer points INTO that freed list's
   inline string data (a borrow that should have been an owning copy), and
   a later in-place self-append (`s = s & c`, len 0→1) writes through it.

## Session-2 investigation (2026-07-01, after bug-01 landed)

New hard facts, each from a fresh lldb pass on the deterministic binary
(`--stop-at-entry` first; auto-continue breakpoint logging):

1. **Not an allocator unlink bug**: conditional breakpoints on both of
   `arena_alloc`'s return sites (`$rdx == <node>`) never fire before the
   crash — the allocator never hands out the still-linked node.
2. **The corrupting write is an isolated 8-byte store of `1`** at the
   node's `next` word, with **no arena_alloc and no arena_free between the
   last-good and first-bad observations** — it is plain user/builder code
   writing through a stale pointer, not allocator traffic.
3. **Window narrowed to `__regex_lookupRef`'s inter-call stretch** for the
   `${1}0` replacement: `__regex_allDigits("1")` returns (its tail is
   register-only, verified in disasm) → `__regex_parseIntClamp("1")`
   (inline toInt + function-level TRAP; its success path allocates
   nothing) → the write lands → `__regex_lookupNum` entry observes the
   corruption. None of these functions contains a self-append or any
   other static heap store — the writer is NOT visible in their code,
   which means the store is reached indirectly (a helper tail, or a
   register-indirect store whose base is computed elsewhere).
4. **Forcing the self-append regrow path unconditionally makes the test
   pass 3/3** — suggestive of the in-place-append-with-stale-shadow
   theory, but NOT conclusive: any allocation-pattern change shifts the
   heap layout and can mask the corruption rather than fix it.
5. **The corruption is self-feeding**: reads of poisoned freed memory
   feed back into control flow, so heap layout drifts between lldb
   scripts of different weight (only byte-identical scripts reproduce the
   same addresses). This blocks instance-targeted breakpoints
   (`--ignore-count`, memory-value conditions) from converging.
6. Value-shape note: an 8-byte `1` is also the ERR tag of a boxed
   trap-Result (`str tag,[box+0]`) and a collection count — not only a
   String length. The trap-box store with a STALE box pointer (the
   `result_inline_alloc_ok` path skipping the alloc but running the
   stores) fits the window (parseIntClamp's TRAP machinery runs exactly
   there) and deserves the next look.

Also audited clean (static, per-instruction): `allDigits`'s inlined
`collections::get` string copy (the only heap-store site in the window's
functions — its `str len,[rdx]` correctly uses the immediately-preceding
alloc result; single-entry label, no intervening rdx clobber);
`parseIntClamp` (zero heap stores, zero allocs — the RETURN-of-trapped
toInt does not box); `lookupRef` (zero heap stores); `isDigit` (pure);
`allDigits`' return tail (register-only). The 8-byte `1` store therefore
comes from a register-indirect path none of these functions' static code
explains — the leading residual theories are (a) a stale trap-box store
reached via a skipped-alloc path, (b) an in-place append whose site lives
in `expand` but whose store lands during this window via tail/loop timing
my markers misattributed.

Next tools to try: compiler-inserted canary (env-gated) that logs every
trap-box store's target, or qemu-based single-step tracing off-box (the
Alpine VM denies watchpoints; docker/qemu denies ptrace). The
forced-regrow discriminator (pass 3/3) plus the marker misattribution
question make re-running the window bisection with markers INSIDE
`__regex_expand`'s append sites the best next step.

## Suspects
- A `collections::get`/`strings::*` path in the regex engine that still
  returns a borrow into `List OF String` inline data (the scope-drop-frees
  work made get/replace own — check for a remaining seam, e.g. the paths
  __regex_expand → __regex_lookupNum/lookupName/parseIntClamp use, or
  __regex_toScalars interplay).
- Copy-insertion (lower_value_owned) eliding a copy for a value that ends
  up owned by a dropped temp.

## Also affected
`thread-regex-rt` (regex compile/match inside a worker) segfaults on
linux-x86_64 — same engine, same arena-churn pattern; treat as the same bug
until proven otherwise.

## Repro
Full tests/func_regex_replace_valid on linux-x86_64 (box: ssh -p 2227
test@127.0.0.1). 13-call prefixes pass; layout-sensitive.

## Debug method notes
- lldb on musl NEEDS `process launch --stop-at-entry` before breakpoints
  (silently never inserts otherwise — burned again this session).
- Hardware watchpoints HANG the Alpine VM. The working substitute:
  `breakpoint command add` + `--auto-continue` auto-logging memory at
  every arena_free, then diff the state timeline.
