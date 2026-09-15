### 1. `seconds` has the wrong meaning in component overloads
UNIT:      man-page:datetime/instant
CLAIM:     “Whole seconds since the Unix epoch, 1970-01-01T00:00:00Z.”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/datetime/func_instant.rs:BODY_3` computes `mins * 60 + seconds`; thus, in the three- through five-argument forms, `seconds` is a component, not independently an epoch offset. Probe `datetime::instant(1, 2, 3, 4, 0)` printed `93784,0`.
SUGGESTED: “Whole seconds: the epoch offset in the one- and two-argument forms, or the seconds component in a component form. It may be negative; zero contributes no seconds.”

### 2. `nanos` omits its signed, unbounded normalization behavior
UNIT:      man-page:datetime/instant
CLAIM:     “Nanoseconds past the second.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/helper_norm_instant.rs:BODY` divides arbitrary signed `nanos` by `1_000_000_000`, carries whole seconds, and changes a negative remainder into `0 .. 999_999_999`. Probe `datetime::instant(10, 1_500_000_000)` printed `11,500000000`; `datetime::instant(10, -1)` printed `9,999999999`.
SUGGESTED: “Signed nanoseconds to add. Values outside 0 .. 999_999_999 are normalized by carrying or borrowing whole seconds; zero adds nothing.”

### 3. A negative component does not necessarily select a pre-epoch instant
UNIT:      man-page:datetime/instant
CLAIM:     “Every numeric argument may be negative, which selects an instant before the epoch.”
VERDICT:   wrong
EVIDENCE:  Probe `datetime::instant(10, -1)` printed `9,999999999`, which remains after the epoch. `src/codegen/builtins/datetime/helper_norm_instant.rs:BODY` confirms that negative nanoseconds adjust the supplied seconds rather than independently determining the side of the epoch.
SUGGESTED: “Every numeric argument is signed. Negative components subtract from the total, so the resulting instant can fall before or after the epoch.”

### 4. The documented overflow condition omits normalization overflow from `nanos`
UNIT:      man-page:datetime/instant
CLAIM:     “The folding and normalization are ordinary signed Integer arithmetic, so a sufficiently large day, hour, minute, or second magnitude can overflow the Integer range and trap.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/helper_norm_instant.rs:BODY` returns `Instant[seconds + q, r]`; normalization can therefore overflow even when only `nanos` supplies the carry. A program calling `datetime::instant(9_223_372_036_854_775_807, 1_000_000_000)` built successfully and printed `Error: 7-705-0010 Arithmetic overflow or numeric conversion outside the destination range.`
SUGGESTED: “ErrOverflow is raised when folding components or carrying normalized nanoseconds would exceed the Integer range.”

### 5. The component parameter descriptions omit their valid domain and signed behavior
UNIT:      man-page:datetime/instant
CLAIM:     “Whole minutes.”
VERDICT:   incomplete
EVIDENCE:  The same issue applies to “Whole hours.” and “Whole days since the epoch.” `src/codegen/builtins/datetime/func_instant.rs:BODY_3`, `BODY_4`, and `BODY_5` multiply each signed component directly; they impose no 0–59 or 0–23 bounds, and zero contributes nothing. The probe’s `datetime::instant(1, 2, 3, 4, 0)` result, `93784,0`, confirms these are folded quantities.
SUGGESTED: “Signed whole minutes to add; values are not restricted to a clock-minute range, and zero contributes nothing.”