# bug-218: riscv64 select latent defects — d16→ft2 scratch aliasing and standalone unsigned-branch gap

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: correctness (latent)

Status: Open

Two latent (currently-unreached) correctness issues in
`src/arch/riscv64/select.rs`:

- `map_fp_register` (`:536`): the arm `16..=25 => format!("ft{}", n - 16 + 2)`
  maps a residual physical `d16` to `ft2`, but `ft2` is a reserved
  lowering-scratch register (regmodel excludes ft0/ft1/ft2 for float-compare
  staging and scalarized-v128 FMA lanes); the mapping aliases live scratch, and
  the arm comment wrongly claims it only skips ft0/ft1. No current stream carries
  a physical `d16`–`d25` (kernels use FP virtuals). Fix: start the range at `ft3`
  (`n - 16 + 3`) and correct the comment/bound.
- Standalone flag-branch match (`:348-358`): enumerates
  BranchEq/Ne/Ge/Lt/Gt/Le/Hi/Lo/Ls but omits BranchHs/BranchCs (unsigned `>=`),
  which the fused `int_branch` path (`:89`) does map. A deferred bare compare
  consumed by a standalone `b.hs`/`b.cs` reaches the generic push and fails-loud
  ("rv64 encoder does not yet support 'b.hs'"). No current stream produces it.
  Fix: add `CodeOp::BranchHs | CodeOp::BranchCs` to the standalone arm.
