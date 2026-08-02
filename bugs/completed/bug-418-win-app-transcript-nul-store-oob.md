# bug-418: Windows app-mode transcript `io_write` writes the wide-string NUL past the 64 KiB `wbuf` for any print ≥ 32768 bytes → arena OOB write

Last updated: 2026-08-01
Effort: small (<1h)
Severity: MEDIUM
Class: Memory-safety (arena/heap OOB write; buffer size arithmetic before store)

Status: FIXED (73304a222)
Regression Test: `src/target/win_x86_64/app/mod.rs` unit test
`transcript_nul_offset_is_clamped_within_wbuf` — emit-inspection: the transcript
NUL store (`str_u16` of the zero register) must be preceded by a `cmp_imm rhs=32767`
clamp so the offset stays within the 65536-byte `wbuf`.

## STATUS: FIXED (73304a222)

Confirmed the mechanism, then fixed the offset arithmetic:

- **Reproduced (mechanism, on the macOS host):** the RED emit-inspection test found
  the transcript-path NUL `str_u16` but **no** `cmp_imm rhs=32767` before it — the
  offset was the unbounded `str[0]*2`. That is exactly the documented mechanism (the
  UTF-8 byte length driving the wchar offset), not a proxy.
- **Fix:** derive the NUL offset from `MultiByteToWideChar`'s converted wchar count
  in `%ret0`: mask the `int` return's garbage high bits (the same trick the
  `WM_GETTEXTLENGTH` return below uses — this is what the byte-length hack originally
  dodged), clamp to ≤ 32767, then `*2`. Max offset is now `32767*2 = 65534 < 65536`,
  always inside `wbuf`. A failed conversion returns 0 → NUL at `wbuf[0]` (safe).
- **Verified in the real target:** a cross-compiled `windows-app` PE
  (`mfb build -app -target windows-x86_64`) disassembles at the io.print helper to
  `movabsq $0xffffffff,%rdx; andq %rdx,%rax; cmpq $0x7fff,%rax; jle …; movabsq
  $0x7fff,%rax; …; shlq $0x1,%rcx; addq wbuf,%rcx; movw $0x0,(%rcx)` — the NUL is
  now `wbuf + min(convertedLen, 32767)*2`. (No Windows `.ncodesum` golden exists;
  artifact-gate skips Windows codegen — verification is disasm + RED→GREEN emit test.)
- **Gate:** full `cargo test` green (EXIT 0; `mfb` bin 3747 passed, 0 failed),
  including the new regression test.

Deviations from the Blast Radius note: the suggested "validate the `int` return ≥ 0
first" is subsumed by masking to the low 32 bits (`& 0xffffffff` makes the value
non-negative) before the ≤ 32767 clamp, so no separate sign check is needed.
Byte-length `str[0]*2` chunking was not needed — the single clamped store suffices.

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
