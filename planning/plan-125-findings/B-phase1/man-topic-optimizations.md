### 1. Pass-stage ordering contradicts the rendered catalog
UNIT:      man-topic:optimizations  
PAGE:      optimizations  
CATEGORY:  consistency  
CLAIM:     “the stages appear in the table from earliest to latest”  
VERDICT:   wrong  
EVIDENCE:  `src/optimizer/catalog.rs:rows` places the MIR “Constant propagation” and “Copy propagation” rows before NIR “Loop-invariant code motion” and later NIR rows; the rendered table has that same order. `src/target/shared/lower.rs:80` runs `optimize_nir` before `src/codegen/engine/regalloc/builder_registers.rs:95` runs `optimize_mir`, so NIR is necessarily earlier than MIR.  
SUGGESTED:  “Passes on the dial, in the order they run. *Stage* names the part of compilation where each pass runs; some rows affect more than one stage.”

### 2. Test-command users cannot discover the matching syntax or help
UNIT:      man-topic:optimizations  
PAGE:      optimizations  
CATEGORY:  discoverability  
CLAIM:     The page says the dial is selected on the “`mfb build` / `mfb test` command line,” but its Synopsis, verbose explanation, and See also link name only `mfb build`.  
VERDICT:   missing  
EVIDENCE:  `src/docs/man/optimizations/package.md` contains only `mfb build` in Synopsis, the `-v` paragraph, and See also. `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb test --help` prints `-O <level>` and `-v` support, including per-pass optimizer fire counts.  
SUGGESTED:  Add `mfb test -O <level> [path]` to Synopsis, say “With `-v`, `mfb build` and `mfb test` print…”, and add `mfb test --help` to See also.