### 1. Rounding return type is misstated in the overview
UNIT:      man-pkg:math  
PAGE:      package-wide  
CATEGORY:  overview-mismatch  
CLAIM:     "`floor`/`ceil`/`round` always give back an `Integer`."  
VERDICT:   misleading  
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man math` prints that sentence, while `mfb man math floor`, `ceil`, and `round` each list `List OF Float`/`List OF Fixed` overloads returning `List OF Integer`. The probe built and ran with that binary at `/tmp/plan-125-scratch/B-phase3/man-pkg-math/probe`: `math::round([1.2, 2.5])` assigned to `List OF Integer` printed `1:3`.  
SUGGESTED:  "`floor`, `ceil`, and `round` return `Integer` for scalar inputs and `List OF Integer` for their `List OF Float` and `List OF Fixed` forms."

### 2. Initial random-sequence behavior is undocumented
UNIT:      man-pkg:math  
PAGE:      package-wide  
CATEGORY:  coverage  
CLAIM:     No Math page says what sequence `math::rand` uses before a program calls `math::seed`.  
VERDICT:   missing  
EVIDENCE:  `mfb man math --all` mentions only sequences “seeded with `math::seed`” and reproducibility after explicit seeding. `src/codegen/engine/function/entry.rs` initializes the generator from OS entropy before user code. The scratch probe, built with the supplied release binary and run twice, printed different first unseeded draws (`142951`, then `897552`) while its explicitly reseeded draws were identical both times (`158027:158027`).  
SUGGESTED:  Add to `rand` or `seed`: “Before your program calls `seed`, its random sequence starts from a fresh automatically chosen seed. Call `seed` when you need the same sequence again.”