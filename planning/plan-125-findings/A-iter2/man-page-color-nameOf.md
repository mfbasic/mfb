### 1. Nonexistent “different function”
UNIT:      man-page:color/nameOf  
CLAIM:     `"Closest named colour" is a different function, with a contestable metric and a much higher cost; it is deliberately not this one.`  
VERDICT:   wrong  
EVIDENCE:  `src/codegen/builtins/color/mod.rs:register` registers every public `color` member and has no nearest/closest-colour function; `rg -n -i 'closest named|nearest.colou?r|nearest' src/codegen/builtins` finds no such member. The release-binary probe confirms `nameOf` rejects `#ff0001` with `77050004`, rather than approximating it.  
SUGGESTED: `nameOf never searches for a nearest named colour: a colour must match a CSS named colour exactly.`