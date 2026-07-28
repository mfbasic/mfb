# bug-228: Mach-O dysymtab/symtab u32 fields use bare `as u32` casts, bypassing the overflow guard

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: footgun (latent)

Status: Fixed (2026-07-15) — the dysymtab nundefsym / indirectsymoff / nindirectsyms fields and the symtab symbol string offset now route through the u32_field() overflow guard instead of a bare `as u32`, matching every other offset/size field in commands.rs (bug-88/bug-168). Latent (>4 GiB regime only), now uniform.

A handful of Mach-O u32 fields in `src/os/macos/link/commands.rs` (nundefsym,
indirectsymoff, nindirectsyms, symbol string offsets — `:258, 265, 266, 452`) are
written with a bare `as u32` cast, bypassing the `u32_field()` overflow guard
that bug-88/bug-168 added to every other offset/size field in this file.

Trigger: only in the >4 GiB output-image regime (which the linker does not
otherwise support) — an `indirect_symbol_offset` past 4 GiB would silently
truncate to a wrong linkedit offset instead of panicking with a clear message
like the neighboring fields. Unreachable in practice (LOW/latent), but
inconsistent with the hardening intent.

Fix: route these casts through the existing `u32_field("...", value)` helper for
uniform fail-fast behavior.
