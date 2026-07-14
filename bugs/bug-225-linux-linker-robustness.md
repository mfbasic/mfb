# bug-225: Linux linker robustness — unbounded read_u32/write_u32 slice + vestigial dynamic_prefix_size parameter

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: memory-safety / dead-code

Status: Open

Two low items in the Linux linker:

- `read_u32`/`write_u32` (`src/os/linux/link/mod.rs:551-557`) index
  `bytes[offset..offset+4]` with no bounds check, so a relocation offset landing
  within the last 3 bytes of text panics with an arithmetic slice panic instead
  of returning a linker `Err`. Currently prevented only by the encoder always
  placing reloc fields inside emitted instructions (latent). Fix: bounds-check
  `offset+4` against the slice length and return an `Err`, matching the
  surrounding reloc handlers.
- `dynamic_prefix_size` (`src/os/linux/link/elf.rs:662,699`) accepts
  `text_len_with_stubs` but explicitly discards it (`let _ = ...`); the returned
  GOT offset depends only on the data layout. The parameter (and the caller's
  `stub_count*12` computation) is vestigial and misleads readers. Fix: drop the
  unused parameter.
