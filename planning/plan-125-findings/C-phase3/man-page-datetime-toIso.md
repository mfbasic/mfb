### 1. Years beyond four digits cannot be read back
UNIT:      man-page:datetime/toIso
CLAIM:     "every form this member emits can be read back."
VERDICT:   wrong
EVIDENCE:  `/tmp/plan-125-scratch/C-phase3/man-page-datetime-toIso/probe-project` built with the specified binary printed `10000-01-01T00:00:00Z`, then `datetime::parseIso(datetime::toIso(distant, 0))` raised `datetime: expected separator`. `func_to_iso.rs:BODY_2` pads but does not cap the year width.
SUGGESTED: `For four-digit years, every form this member emits can be read back by datetime::parseIso. Years outside that width render, but parseIso rejects their ISO text.`

### 2. “Lossless” does not describe a DateTime round-trip
UNIT:      man-page:datetime/toIso
CLAIM:     "A datetime::DateTime carries nanoseconds, so only datetime::toIso(dt, 9) is lossless."
VERDICT:   misleading
EVIDENCE:  The probe printed `FALSE` for `dt = datetime::parseIso(datetime::toIso(dt, 9))` on a UTC `DateTime`: parsing creates a fixed-offset zone rather than restoring the original UTC zone record. The same probe confirmed nanos `123456789` are retained.
SUGGESTED: `To preserve an arbitrary sub-second value through parseIso, use datetime::toIso(dt, 9); it preserves all nine nanos digits.`

### 3. Output is neither universally fixed-length nor instant-sortable
UNIT:      man-page:datetime/toIso
CLAIM:     "The fractional-second field is zero-padded to a fixed width, so the output of a given form is always the same length and sorts correctly as text."
VERDICT:   wrong
EVIDENCE:  The probe printed both `2026-06-26T01:02:03+00:00:01` and `10000-01-01T00:00:00Z`; the optional offset seconds and unrestricted year width make lengths differ. `helper_offset_label_sep.rs:__datetime_offsetLabelSep` appends `:SS` for non-whole-minute offsets, and `func_date.rs:__datetime_date` accepts unrestricted years.
SUGGESTED: `The fractional-second field is zero-padded to the selected width. Do not use these strings to order instants across differing UTC offsets; their year and offset fields can also vary in width.`

### 4. Only the most-negative offset overflows
UNIT:      man-page:datetime/toIso
CLAIM:     "A datetime::DateTime record you build yourself with an offset at the edge of the Integer range raises ErrOverflow."
VERDICT:   wrong
EVIDENCE:  A probe-built `DateTime[..., 9223372036854775807]` printed `2026-01-01T00:00:00.000+2562047788015215:30:07`; it did not raise. The most-negative value does raise overflow because `helper_offset_label_sep.rs:__datetime_offsetLabelSep` negates negative offsets.
SUGGESTED: `A manually built DateTime whose offset is the most-negative Integer raises ErrOverflow while rendering; other unchecked offsets render as supplied.`

### 5. The page claims an invocation that does not occur
UNIT:      man-page:datetime/toIso
CLAIM:     "toIso is the convenience form of datetime::format invoked with a fixed pattern (yyyy-MM-dd'T'HH:mm:ss.fffZ for the default form)."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/datetime/func_to_iso.rs:BODY_1` delegates to `__datetime_toIso2`; `BODY_2` constructs the timestamp directly and never invokes `__datetime_format`.
SUGGESTED: `toIso uses a fixed ISO timestamp layout; the default form uses three fractional-second digits.`