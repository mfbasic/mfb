### 1. Index parameter omits its valid range
UNIT:      man-page:astrings/getAttributes
CLAIM:     “The scalar index to resolve (0-based).”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/astrings/func_get_attributes.rs:BODY` rejects `index < 0 OR index >= astrings::scalarLen(a)`. The scratch probe’s empty-value call printed `Error: 7-705-0001` / `attribute index out of bounds`.
SUGGESTED: “The zero-based scalar index to resolve. It must be from 0 through the last scalar index; a negative index, an index at or past the scalar count, or any index on empty text raises `ErrIndexOutOfRange`.”

### 2. The no-attribute return case is unstated
UNIT:      man-page:astrings/getAttributes
CLAIM:     “The result carries at most one astrings::Attribute per member — flags are present when any covering span carries them; font/font-size take the winning span's value.”
VERDICT:   incomplete
EVIDENCE:  `__astrings_getAttributes` initializes `result` to `[]` and appends only winning covering spans (`src/codegen/builtins/astrings/func_get_attributes.rs:BODY`). Scratch probe `getAttributes(fromString("x"), 0)` printed `count=0`.
SUGGESTED: “The result is an empty list when no attribute covers the scalar; otherwise it carries at most one `astrings::Attribute` per member.”

### 3. Result ordering is undocumented
UNIT:      man-page:astrings/getAttributes
CLAIM:     “The result carries at most one astrings::Attribute per member — flags are present when any covering span carries them; font/font-size take the winning span's value.”
VERDICT:   incomplete
EVIDENCE:  `__astrings_getAttributes` scans stored spans in insertion order and appends each winner in that order; `__astrings_isWinner` selects by highest `start`, then highest `seq` (`src/codegen/builtins/astrings/func_get_attributes.rs:BODY`, `helper_is_winner.rs:BODY`). Scratch probe printed `at1=Bold Italic Font:Winner` and `at2=Bold Italic Font:First`, preserving insertion order among resolved attributes.
SUGGESTED: “Resolved attributes are returned in their original span-insertion order.”

### 4. The Errors table omits the function’s only runtime error
UNIT:      man-page:astrings/getAttributes
CLAIM:     “An invalid index raises.”
VERDICT:   incomplete
EVIDENCE:  The rendered page has no Errors section because `implementations[0].errors` is `vec![]` in `src/codegen/builtins/astrings/func_get_attributes.rs:register`, while its body explicitly executes `FAIL error(77050001, "attribute index out of bounds")`. The scratch empty-text probe printed `Error: 7-705-0001`.
SUGGESTED: “Errors: `ErrIndexOutOfRange` — `index` is negative or does not identify an existing scalar.”