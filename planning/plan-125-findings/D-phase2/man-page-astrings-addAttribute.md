### 1. Errors are absent from the rendered page

UNIT:      man-page:astrings/addAttribute  
CLAIM:     “An invalid range raises.”  
VERDICT:   incomplete  
EVIDENCE:  The rendered `mfb man astrings addAttribute` page has no Errors table. `src/codegen/builtins/astrings/func_add_attribute.rs:register` declares `errors: vec![]`, while `__astrings_validateRange` in `helper_validate_range.rs` raises `ErrInvalidArgument` for `start < 0` or `endIndex < start`, and `ErrIndexOutOfRange` for `endIndex >= scalarLen(value)`. Acceptance coverage at `tests/acceptance/src/astrings.mfb` verifies both.  
SUGGESTED: “Raises `ErrInvalidArgument` when `start` is negative or greater than `endIndex`, and `ErrIndexOutOfRange` when `endIndex` is outside the visible text.” Add both errors to the registry so the rendered Errors table is present.

### 2. Tie-breaking for overlapping ranges is omitted

UNIT:      man-page:astrings/addAttribute  
CLAIM:     “Spans are stored as-is and never merged; overlapping same-member spans resolve at read time by higher-start-wins (see getAttributes).”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/astrings/helper_is_winner.rs:__astrings_isWinner` resolves equal-start spans by higher `seq`, i.e. later insertion wins. `tests/acceptance/src/astrings.mfb` explicitly verifies “equal start — later insertion wins.” The page gives no result for this valid overlap case.  
SUGGESTED: “Spans are stored as-is and never merged. For overlapping spans of the same attribute member, the higher start wins; if starts are equal, the later-added span wins.”

### 3. `start` parameter does not state its valid range

UNIT:      man-page:astrings/addAttribute  
CLAIM:     “The first scalar index of the range (0-based).”  
VERDICT:   incomplete  
EVIDENCE:  The descriptor parameter description in `src/codegen/builtins/astrings/func_add_attribute.rs:ranged_attr_params` omits the required non-negative bound and ordering rule. `helper_validate_range.rs:__astrings_validateRange` raises `ErrInvalidArgument` for negative `start` and for `start > endIndex`.  
SUGGESTED: “The first scalar index of the inclusive range (0-based); it must be zero or greater and no greater than `endIndex`.”

### 4. `endIndex` parameter does not state its valid range

UNIT:      man-page:astrings/addAttribute  
CLAIM:     “The last scalar index of the range (inclusive).”  
VERDICT:   incomplete  
EVIDENCE:  The descriptor parameter description in `src/codegen/builtins/astrings/func_add_attribute.rs:ranged_attr_params` omits that the index must exist in the visible text. `helper_validate_range.rs:__astrings_validateRange` raises `ErrIndexOutOfRange` when `endIndex >= astrings::scalarLen(value)`, including `0, 0` on empty text.  
SUGGESTED: “The last scalar index of the inclusive range; it must be at least `start` and within the visible text.”