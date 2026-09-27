### 1. Range behavior omitted

UNIT:      man-page:astrings/underline  
CLAIM:     “Pass it to `astrings::addAttribute` to mark a scalar range underlined.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/astrings/func_add_attribute.rs:DESC` and `helper_validate_range.rs:__astrings_validateRange` show that the range is inclusive, zero-based, and raises for negative, reversed, or out-of-bounds endpoints; empty text has no valid range. The exact page example compiled and ran with the specified release binary.  
SUGGESTED: Pass it to `astrings::addAttribute` to underline an inclusive, zero-based scalar range. The range must be non-empty and within the visible text; invalid endpoints raise.