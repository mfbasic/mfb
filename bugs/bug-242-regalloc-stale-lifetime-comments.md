# bug-242: regalloc register-lifetime comments cite a removed arena_alloc survivor contract + liveness field allowlist has no fail-safe

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: docs / footgun

Status: Open

The register allocator core is sound (liveness, active-list, interference,
`(s,v)` tie-break bug-87, clobber masks all verified), but its comments in the
exact area maintainers must reason about are stale, plus one missing fail-safe:

- `src/target/shared/code/regalloc/analysis.rs:82-85` — comment claims
  `_mfb_arena_alloc` "uses callee-saved x20–x28 as scratch (saving only x30)",
  contradicting the current impl (`entry_and_arena.rs:723-727`) and
  `.ai/compiler.md:51` ("PCS-preserves x19–x28; there is no survivor set"). Fix:
  re-word to justify `all_int` by "other `_mfb_*` helpers' clobber sets are
  unknown."
- `src/target/shared/code/regalloc/linear_scan.rs:94-96` — cites arena_alloc's
  `x8/x11/x12/x13/x17` "survivor contract" as the reason for `reserved`, but that
  survivor set no longer exists; and `:78` calls `[s, e]` a "half-open span"
  while `call_clobber_in` treats it inclusive (`idx > e` breaks, so `idx == e` is
  included). Fix: replace the stale example, correct "half-open" → "inclusive".
- `analysis.rs:22-36` (`DEF_FIELDS`/`USE_FIELDS`) — liveness sees registers only
  via a hardcoded field-name allowlist; a future register-valued field name not
  listed would be invisible to liveness and left uncolored (`%vN`) with no
  failure. Fix: add a debug assertion after allocation that no `%v`/`%f` sentinel
  survives, so an uncovered field fails loudly.
