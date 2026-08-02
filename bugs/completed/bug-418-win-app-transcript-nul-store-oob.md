# bug-418: Windows app-mode transcript `io_write` writes the wide-string NUL past the 64 KiB `wbuf` for any print ≥ 32768 bytes → arena OOB write

Last updated: 2026-07-28
Effort: small (<1h)
Severity: MEDIUM
Class: Memory-safety (arena/heap OOB write; buffer size arithmetic before store)

Status: Open
Regression Test: tests/ — a Windows GUI-app-mode program printing a single string
≥ 32768 bytes must not corrupt arena memory (assert intact adjacent allocation).

In the Windows app-mode transcript write path (`src/target/win_x86_64/app/mod.rs`),
`wbuf` is `arena_alloc("65536")` (65536 bytes = 32768 wchars) and
`MultiByteToWideChar` is called with `cchWideChar = 32767` (:785), so the conversion
output itself stays in-bounds. But the NUL terminator is then written at
`wbuf + str[0]*2` (:798-803), where `str[0]` is the UTF-8 **byte** length. The comment
calls `str[0]*2` a "safe upper bound", but it is bounded only by string content, not
by the 65536-byte buffer.

For a single `io.print` of a string with byte length ≥ 32768,
`wbuf + str[0]*2 ≥ 65536`, so `store_u16(ZERO, wbuf + str[0]*2)` writes 2 bytes at or
past the end of the 64 KiB arena block (e.g. a 40000-byte print writes the NUL at
offset 80000 — ~14 KiB past the block), corrupting adjacent arena data. Not
adversarial — ordinary large program output. (The headless std-handle path and the
TUI grid path don't use `wbuf`.)

References: `src/target/win_x86_64/app/mod.rs:785` (`cchWideChar=32767`), `:798-803`
(the unbounded `wbuf + str[0]*2` NUL store), `arena_alloc("65536")`. Found during
goal-07.

## Failing Reproduction

Windows-only (GUI app mode + transcript EDIT); not reproducible on the macOS host.
Arithmetic: buffer 65536 vs max NUL offset `str[0]*2`; `str[0] ≥ 32768 ⟹ offset ≥
65536` (out of the block).

- Observed: a ≥32 KiB single print writes the NUL past the 64 KiB `wbuf`, corrupting
  adjacent arena memory.
- Expected: the NUL lands within `wbuf` (offset < 65536) for any input.

## Root Cause

The NUL offset uses the untrusted UTF-8 byte length `str[0]*2` (which can exceed the
buffer) instead of the actual `MultiByteToWideChar` output length (already capped at
32767 wchars).

## Goal

- The wide-string NUL is written at the true converted length (the
  `MultiByteToWideChar` return, clamped to ≤ 32767), never past `wbuf`.

### Non-goals (must NOT change)

- The 64 KiB `wbuf` size or the `cchWideChar=32767` cap (both correct). The earlier
  garbage-high-rax SIGSEGV fix that motivated the `str[0]`-based offset — but use the
  clamped converted length instead of the raw byte length.

## Blast Radius

- `src/target/win_x86_64/app/mod.rs:798-803` — the NUL store. Use
  `min(MultiByteToWideChar_ret, 32767)` (validate the `int` return ≥ 0 first) as the
  wchar offset, or chunk prints > 32767 wchars.
