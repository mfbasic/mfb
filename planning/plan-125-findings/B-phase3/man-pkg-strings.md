### 1. Terminal-width tools are omitted from the overview’s function map
UNIT:      man-pkg:strings  
PAGE:      package-wide  
CATEGORY:  discoverability  
CLAIM:     “The `strings` package provides package-qualified helpers for `String` values: … slicing and reshaping (`left`, `right`, `mid`, `stripPrefix`, `stripSuffix`, `split`, `join`, `replace`, `repeat`, `padLeft`, `padRight`), length and byte queries (`byteLen`, `toBytes`) …”  
VERDICT:   missing  
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man strings` renders that catalog without `displayWidth`, `padLeftToWidth`, or `padRightToWidth`; `src/codegen/builtins/strings/func_display_width.rs:register`, `func_pad_left_to_width.rs:register`, and `func_pad_right_to_width.rs:register` register all three. The probe `mfb build /tmp/plan-125-scratch/B-phase3/man-pkg-strings/width-probe && …/width_probe.out` printed `6` for scalar-padded `"日本"` and `8` for column-padded `"日本"`, confirming these are a distinct developer-facing capability rather than variants of the existing catalog entry.  
SUGGESTED: Add a terminal-width group to the first inventory, for example: “terminal display measurement and padding (`displayWidth`, `padLeftToWidth`, `padRightToWidth`)”; retain the later sharp-edge explanation.