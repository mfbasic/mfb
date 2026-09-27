### 1. Structural matching omits color values
UNIT:      man-page:astrings/removeAttribute
CLAIM:     "A stored span is affected only when its attribute structurally matches attr (same member and, for font/size, same value)."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/astrings/helper_attr_equals.rs:__astrings_attrEquals` compares the numeric payload for every `AttrNumber`, including foreground and background colors. Probe output: `different-color=4294901760` and `same-color=-`; a different foreground color does not match, while the identical color does.
SUGGESTED: A stored span is affected only when its attribute structurally matches `attr`: it must have the same member and, for value-carrying attributes such as font, font size, foreground, or background, the same value.

### 2. `start` does not state its valid range
UNIT:      man-page:astrings/removeAttribute
CLAIM:     "The first scalar index of the range (0-based)."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/astrings/helper_validate_range.rs:__astrings_validateRange` raises `ErrInvalidArgument` when `start < 0` or `endIndex < start`; the parameter row itself does not state either bound or the negative-input result.
SUGGESTED: The first scalar index of the inclusive range (0-based). It must be zero or more and no greater than `endIndex`; otherwise the call raises `ErrInvalidArgument`.

### 3. `endIndex` does not state its valid range
UNIT:      man-page:astrings/removeAttribute
CLAIM:     "The last scalar index of the range (inclusive)."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/astrings/helper_validate_range.rs:__astrings_validateRange` raises `ErrInvalidArgument` when `endIndex < start` and `ErrIndexOutOfRange` when `endIndex >= astrings::scalarLen(value)`, including `0` on empty text.
SUGGESTED: The last scalar index of the inclusive range. It must be at least `start` and within the text; a value before `start` raises `ErrInvalidArgument`, and one at or past the end raises `ErrIndexOutOfRange`.