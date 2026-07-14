# bug-245: x86_64 x86_float_branch panics (compiler ICE) on an unmapped float-compare condition

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: footgun (latent)

Status: Open

`x86_float_branch` (`src/arch/x86_64/select.rs:605`) triggers a `panic!` (compiler
ICE) rather than a recoverable `Err` when a float-compare branch condition is
outside the mapped set (b.gt/ge/mi/lo/ls/eq/ne/hi/lt/le/vs/vc).

Trigger: a fused float compare (`FCmpD`/`FCmpZeroD`) whose branch mnemonic is
outside the mapped set. No current float lowering is known to produce another
condition (latent), but any future/edge fusion (e.g. a `b.pl`/`b.cs` after
`fcmp`) panics the whole compile instead of erroring cleanly like the encoder's
other unsupported arms.

Fix: return `Result`/`Err(...)` from `x86_float_branch` (and its caller) instead
of panicking on an unmapped condition.
