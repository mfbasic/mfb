### 1. Range bounds and failure behavior omitted
UNIT:      man-page:astrings/italic  
CLAIM:     “Pass it to `astrings::addAttribute` to mark a scalar range italic.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/astrings/helper_validate_range.rs:__astrings_validateRange` makes the range inclusive, rejects negative `start` or `endIndex < start`, and raises out-of-bounds for empty text. Probe `invalid_range_project` called `addAttribute(fromString(""), 0, 0, italic())`; it printed `Error: 7-705-0001` / `attribute range out of bounds`.  
SUGGESTED: Pass it to `astrings::addAttribute` over an inclusive, zero-based scalar range. The range must identify visible text: a negative start or end before start raises `ErrInvalidArgument`, and bounds outside the text raise `ErrIndexOutOfRange`; empty text has no valid range.