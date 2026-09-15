### 1. Empty fractional part is accepted
UNIT:      man-page:datetime/parseIso
CLAIM:     "`.fraction` — optional fractional second: a `.` followed by decimal digits."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/datetime/func_parse_iso.rs:__datetime_parseIso` does not require any digit after `.`. Probe `datetime::parseIso("2026-06-26T09:30:00.Z")` compiled and printed `2026-06-26T09:30:00.000000000Z`.
SUGGESTED: "`.fraction` — optional fractional second: a `.` optionally followed by decimal digits. The first nine digits are scaled to nanoseconds; additional digits are ignored. A `.` with no following digits represents zero nanoseconds."

### 2. Parameter omits empty-input behavior
UNIT:      man-page:datetime/parseIso
CLAIM:     "The ISO-8601 text to parse."
VERDICT:   incomplete
EVIDENCE:  The rendered parameter description gives no empty-input outcome. `src/codegen/builtins/datetime/func_parse_iso.rs:__datetime_parseIso` calls `__datetime_readNum` first; the probe `datetime::parseIso("")` printed `empty: 77050003` (`ErrInvalidFormat`).
SUGGESTED: "The RFC 3339 / ISO 8601 timestamp to parse. It must include the required UTC offset; an empty or malformed string raises `ErrInvalidFormat`."