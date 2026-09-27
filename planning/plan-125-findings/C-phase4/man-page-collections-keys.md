### 1. Map assignment can move an existing key

UNIT:      man-page:collections/keys  
CLAIM:     “The map keeps its entries in insertion order and keys walks them in that order; how a key is looked up is separate and does not reorder it.”  
VERDICT:   incomplete  
EVIDENCE:  Probe `/tmp/plan-125-scratch/C-phase4/man-page-collections-keys/src/main.mfb`, built and run with the specified release binary, printed `afterSet=Ada,Lin,Grace,Mina` after assigning `"Grace"` in an initially `Ada,Grace,Lin` map. `collections::set` implements the map path as `removeKey` followed by concatenation (`src/codegen/builtins/collections/func_set.rs:lower_set`), so assigning an existing key produces a new map with that key later in iteration order. A pure `collections::get` probe printed `read=85:Ada,Grace,Lin`, confirming lookup itself does not reorder.  
SUGGESTED: “`keys` follows the map’s current iteration order, which is implementation-defined but stable while that map is unchanged. `keys` and `collections::values` remain index-aligned. On the current implementation, assigning an existing key with `collections::set` produces a new map where that key appears at the end of the iteration order.”