# bug-245: x86_64 x86_float_branch panics (compiler ICE) on an unmapped float-compare condition

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: footgun (latent)

Status: Skipped (2026-07-15) — the proposed fix (return `Err` from
`x86_float_branch` instead of `panic!`) contradicts the codebase's deliberate,
cross-architecture codegen-ICE convention: `select_riscv64` and `select_aarch64`
both `panic!("unmapped … compare-branch condition")` on the same class of
unreachable defect (each with a `#[should_panic]` test). Making only x86 recoverable
would be inconsistent, and threading a `Result` out requires changing the shared
`mir::Backend::select` trait signature (`-> Vec<CodeInstruction>`) and its call site
plus every backend impl — disproportionate for a LOW/latent condition that no
current float lowering can produce. Left as an intentional ICE, consistent with the
sibling selectors. (Revisit only if the `Backend::select` trait is made fallible
tree-wide.)

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
