### 1. Overview hides the package’s Set API
UNIT:      man-pkg:collections  
PAGE:      package-wide  
CATEGORY:  overview-mismatch  
CLAIM:     “Sequence and map helper functions” and “The collections package provides package-qualified helpers for List and Map values”  
VERDICT:   misleading  
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man collections` prints those claims, but its function table includes the Set-focused `add`, `remove`, `toList`, `toSet`, `union`, `intersection`, `difference`, `symmetricDifference`, `isSubset`, `isSuperset`, and `isDisjoint`. `src/codegen/builtins/collections/mod.rs::register` registers that Set API; `scripts/man-run-examples.sh collections --run toSet union isSubset toList` built and ran all 8 examples successfully.  
SUGGESTED:  Describe the package as helpers for `List`, `Map`, and `Set` values, and add a Set category to the overview, for example: “Set conversion and algebra (`toSet`, `toList`, `add`, `remove`, `union`, `intersection`, `difference`, `symmetricDifference`, `isSubset`, `isSuperset`, `isDisjoint`).”