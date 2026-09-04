# MEM-40 spike — Win64 entry RNG seed scratch aliases the stdin buffer arena slot

audit-3 MEM-40 (`planning/audit-3-codegen-memory.md`), bug-512. Windows-only.
Verifiable on any host by codegen inspection (no Windows box needed):

```
mfb build -ncode -target windows-x86_64 spikes/audit-3/MEM-40
```

## Observed (defect present) — from the emitted windows-x86_64 ncode

Program entry (arena pinned at `rsp+32` by the Win64 shadow space):
```
add_imm r15, rsp, 32          # r15 = arena = rsp + 32
str_u64 r15, [rsp + 3768]     # seed scratch at rsp+3768 == arena + 3736
bl BCryptGenRandom            # 8 random bytes written at arena + 3736
```
`_mfb_rt_stdin_next_byte`:
```
mov_imm r10, 3736 ; add r10, r15, r10 ; ldr_u64 r11, [r10]   # reads arena + 3736
cmp_imm r11, 0 ; b.ne have_lbuf                               # "NULL => not yet allocated"
```
`3736 == ARENA_STDIN_LOCAL_BUF_OFFSET`. The seed scratch (meant to be one word
*past* the arena state, at `sp + ARENA_STATE_SIZE`) is not shifted by the Win64
shadow, so on Win64 it lands inside the arena at the stdin local-buffer slot. The
8 random bytes become a bogus buffer pointer that the stdin path reads and writes
stdin bytes through → arbitrary pointer read/write.

Preconditions (both ordinary): `math::rand` anywhere in the module (the seed
writer) + any stdin read (the reader).

## Negative control

The same build at `-target linux-x86_64` emits `add_imm r15, rsp, 0`, so
`[rsp+3768]` is `arena+3768` — one word past the arena state, as intended.

## Expected

The seed scratch must be reserved above the arena at `arena + shadow +
ARENA_STATE_SIZE` (or the arena base and the scratch offset must both account for
the shadow), so it never aliases a live arena slot on Win64.
