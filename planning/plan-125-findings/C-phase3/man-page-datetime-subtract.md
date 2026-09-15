### 1. Signed direction is obscured in the intro
UNIT:      man-page:datetime/subtract
CLAIM:     "Shift a `datetime::Instant` backward along the UTC timeline by a `datetime::Duration`."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/datetime/func_subtract.rs:BODY` computes `at.seconds - by.seconds`; the published negative-duration example compiled and ran with `datetime::duration(-3600)`, which shifts later.
SUGGESTED: "Shift a `datetime::Instant` by a signed `datetime::Duration`: positive spans move it backward and negative spans move it forward."

### 2. The description repeats the unqualified direction claim
UNIT:      man-page:datetime/subtract
CLAIM:     "`datetime::subtract` returns the `datetime::Instant` reached by moving `at` backward along the UTC timeline by the span `by`."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/datetime/func_subtract.rs:BODY` subtracts each signed field; with `by = datetime::duration(-3600)`, the result is one hour later, as the page’s second example itself demonstrates.
SUGGESTED: "`datetime::subtract` returns the instant reached by applying the opposite of signed span `by` to `at`: positive spans move earlier and negative spans move later."

### 3. `add`-with-negation is not equivalent at the minimum duration
UNIT:      man-page:datetime/subtract
CLAIM:     "How far back to shift. Equivalent to `datetime::add` with the duration negated."
VERDICT:   wrong
EVIDENCE:  Probe using `minSecond = -9223372036854775808` printed `0:0` for `datetime::subtract(datetime::instant(minSecond), datetime::duration(minSecond))`; subsequently `datetime::negate(datetime::duration(minSecond))` printed `Error: 7-705-0010`. `func_subtract.rs:BODY` subtracts directly, while `datetime::negate` overflows for that duration.
SUGGESTED: "The signed span to subtract: positive moves earlier, negative moves later, and zero leaves the instant unchanged. For durations that can be negated, this is equivalent to `datetime::add(at, datetime::negate(by))`."

### 4. “Always yield” contradicts the documented overflow path
UNIT:      man-page:datetime/subtract
CLAIM:     "`subtract` is pure: the same `datetime::Instant` and `datetime::Duration` always yield the same `datetime::Instant`, and it has no side effects."
VERDICT:   misleading
EVIDENCE:  The scratch probe `datetime::subtract(datetime::instant(9223372036854775807), datetime::duration(-1))` compiled, then printed `Error: 7-705-0010 Arithmetic overflow or numeric conversion outside the destination range.` `func_subtract.rs:register` declares `ErrOverflow`.
SUGGESTED: "For inputs whose result fits in an `Integer`, `subtract` returns the same result for the same inputs and has no side effects; otherwise it raises `ErrOverflow`."