# bug-512: Win64 program-entry RNG seed scratch aliases `arena[ARENA_STDIN_LOCAL_BUF_OFFSET]` → arbitrary pointer read/write via the stdin path

Last updated: 2026-09-03
Effort: small (<1h)
Severity: HIGH
Class: security (memory safety — arbitrary pointer read/write, Windows)

Status: Open (found in audit-3, Surface 3 MEM-40; codegen-verified by the lead from the emitted windows-x86_64 ncode)

Regression Test: a codegen-inspection test asserting the entry seed write offset does not equal any live arena-slot offset on Win64 (or that arena base and scratch both include the shadow).

## Summary

On the Windows x86-64 target, the program-entry code seeds the RNG by writing 8
`BCryptGenRandom` bytes to a scratch word it believes is one word *past* the arena
state. But the scratch offset (`ENTRY_SEED_SCRATCH_OFFSET = ARENA_STATE_SIZE`) is
not shifted by the Win64 32-byte shadow space, while the arena base *is* pinned at
`rsp + shadow`. So the write lands inside the arena at
`ARENA_STDIN_LOCAL_BUF_OFFSET`. The stdin path then reads that word as an
already-allocated 4 KiB local buffer pointer ("NULL ⇒ not yet allocated") and
copies stdin bytes through it — an arbitrary pointer read/write driven by random
seed bytes. Reachable by any Windows program that uses `math::rand` (the writer)
and reads stdin (the reader).

## Mechanism

`ENTRY_SEED_SCRATCH_OFFSET = ARENA_STATE_SIZE` (`error_constants.rs:169`), and the
entry writes the seed there (`entry.rs:396-408`). On Win64 the arena register is
`rsp + shadow_space_bytes()` = `rsp + 32` (`x86_64/backend.rs:87`), so
`rsp + ARENA_STATE_SIZE` = `arena + (ARENA_STATE_SIZE − 32)` =
`arena + 3736` = `arena + ARENA_STDIN_LOCAL_BUF_OFFSET`.

Lead-verified in the emitted `windows-x86_64` ncode
(`spikes/audit-3/MEM-40/`):

```
add_imm r15, rsp, 32          # arena = rsp + 32
str_u64 r15, [rsp + 3768]     # seed scratch == arena + 3736
bl BCryptGenRandom            # 8 random bytes at arena + 3736
...
_mfb_rt_stdin_next_byte:
  mov_imm r10, 3736 ; add r10, r15, r10 ; ldr_u64 r11, [r10]   # reads arena + 3736
  cmp_imm r11, 0 ; b.ne have_lbuf                              # nonzero => "already allocated" buffer
```

The linux-x86_64 build emits `add_imm r15, rsp, 0`, so `[rsp+3768]` is
`arena+3768` — one word past the arena, as intended. The bug is Win64-specific.

## Reproduction

Codegen-verified on any host: `mfb build -ncode -target windows-x86_64
spikes/audit-3/MEM-40` shows the seed write and the stdin read at the same
arena-relative offset (3736). On box 2230, `echo hello | MEM-40.exe` would read
back through the corrupted pointer (execution not run by the lead).

## Best fix

Reserve the seed scratch *above* the arena including the shadow — write it at
`rsp + shadow_space_bytes() + ARENA_STATE_SIZE` (i.e. `arena + ARENA_STATE_SIZE`),
not `rsp + ARENA_STATE_SIZE`; and grow `ENTRY_STACK_SIZE` by the shadow on Win64.
Equivalently, make `ENTRY_SEED_SCRATCH_OFFSET` a function of the arena base the
same way the arena register is. Add a codegen guard asserting the seed offset does
not collide with any `ARENA_*_OFFSET`.

## Non-goals

No MFBASIC surface change; the SysV/macOS layouts (shadow = 0) must stay
byte-identical; keep the seed a real `BCryptGenRandom` read.

## Prior art

None (searched `ENTRY_SEED_SCRATCH`, `shadow`, `ARENA_STDIN_LOCAL_BUF`, `seed`
across `bugs/`, `bugs/completed/`, `audit-1-*`, `audit-2-*`). The Win64 target had
no prior security audit.
