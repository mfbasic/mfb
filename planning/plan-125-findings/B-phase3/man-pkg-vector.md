### 1. `forward` is not available for every vector type
UNIT:      man-pkg:vector  
PAGE:      package overview  
CATEGORY:  overview-mismatch  
CLAIM:     “a set of record constants (`zero`/`one`/`up`/`right`/`forward` in each type).”  
VERDICT:   wrong  
EVIDENCE:  `src/codegen/builtins/vector/mod.rs:368-432` explicitly says “`forward` (+z) is undefined in 2D” and registers it only for dimensions 3 and 4. A probe built with `mfb build /tmp/plan-125-scratch/B-phase3/man-pkg-vector/probe` rejected `vector::forwardFloat2` with `Built-in package vector does not export vector.forwardFloat2.`  
SUGGESTED:  “The package also provides `zero`, `one`, `up`, and `right` constants for every vector type, plus `forward` for 3D and 4D types.”

### 2. Record constants cannot be discovered from the rendered unit
UNIT:      man-pkg:vector  
PAGE:      package-wide  
CATEGORY:  discoverability  
CLAIM:     The overview promises record constants, but no rendered page states their exported names, spelling convention, or component values.  
VERDICT:   missing  
EVIDENCE:  `rg -n 'zeroFloat|upInteger|forwardFixed' /tmp/plan-125-scratch/B-phase3/man-pkg-vector/rendered-all.txt` printed no matches; the overview only gives generic names, and `mfb man vector types` lists fields only. `src/codegen/builtins/vector/mod.rs:437-457` registers names such as `zeroFloat3`, while the probe compiled and ran `vector::zeroFloat3`, `vector::upInteger2`, and `vector::forwardFixed3`, printing `(0.00, 0.00, 0.00)`, `(0, 1)`, and `(0.00, 0.00, 1.00)`.  
SUGGESTED:  Add a “Constants” section to the overview or types page documenting the exported spelling (`zeroFloat3`, etc.), the basis values, and that `forward` exists only for 3D/4D types.

### 3. `cross` incorrectly claims exclusive non-`ErrInvalidArgument` status
UNIT:      man-pkg:vector  
PAGE:      vector::cross  
CATEGORY:  consistency  
CLAIM:     “`cross` is also the only geometry function here that never raises `ErrInvalidArgument`”  
VERDICT:   inconsistent  
EVIDENCE:  `src/codegen/builtins/vector/func_cross.rs:192` supplies an empty error list, but so do `func_distance.rs:169-173`, `func_dot.rs:142-146`, and `func_scale.rs:162`. The rendered `vector::distance` page also says it “therefore never raises ErrInvalidArgument” at `rendered-all.txt:848`.  
SUGGESTED:  Replace with: “`cross` never raises `ErrInvalidArgument`: parallel operands simply produce the zero vector.”