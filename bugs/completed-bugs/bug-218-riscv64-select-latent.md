# bug-218: riscv64 select latent defects — d16→ft2 scratch aliasing and standalone unsigned-branch gap

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: correctness (latent)

Status: Partially fixed (2026-07-15) — item 1: map_fp_register now maps a residual physical d16..d24 to ft3..ft11 (was d16..d25 -> ft2.., aliasing the reserved ft2 lowering scratch); comment/bound corrected. Item 2 is not applicable as written: CodeOp has no BranchHs/BranchCs variants (b.hs/b.cs exist only as string mnemonics in the fused int_branch path), so they can never reach the standalone CodeOp match — there is no reachable gap to close.

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
