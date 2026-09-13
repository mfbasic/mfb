### 1. `Rounding` qualification contradicts the usable syntax
UNIT:      man-pkg:money  
PAGE:      getRounding and setRounding  
CATEGORY:  consistency  
CLAIM:     “The `money::Rounding` enum is referenced bare, like every other builtin type: write `money::Rounding.Banker`, not `money::Rounding.Banker`.”  
VERDICT:   wrong  
EVIDENCE:  `src/codegen/builtins/money/func_get_rounding.rs:28` and `func_set_rounding.rs:34` render the identical, self-contradictory sentence. Probe `/tmp/plan-125-scratch/B-phase3/man-pkg-money/probe/src/main.mfb` with `LET bare AS Rounding = money::Rounding.Banker`; `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb build /tmp/plan-125-scratch/B-phase3/man-pkg-money/probe` printed `SYMBOL_UNKNOWN_TYPE: Type 'Rounding' is not a built-in or top-level project type.` The same probe compiled after changing the declaration to `LET mode AS money::Rounding = money::Rounding.Banker`.  
SUGGESTED: Replace both occurrences with: “Use the package-qualified enum name: write `money::Rounding.Banker`.”