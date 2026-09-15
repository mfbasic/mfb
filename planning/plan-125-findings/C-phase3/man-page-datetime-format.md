### 1. Fractional token widths above nine do not error

UNIT:      man-page:datetime/format  
CLAIM:     “f .. fffffffff — fractional second, fixed to the run length (fff = ms, ffffff = us, fffffffff = ns)”  
VERDICT:   wrong  
EVIDENCE:  Probe `datetime::format(dt, "ffffffffff")`, with `dt.time.nanos = 123456789`, printed `f10=123456789` rather than raising or producing ten digits. `src/codegen/builtins/datetime/helper_format_token.rs:__datetime_formatToken` passes every `f` run length to `strings::left(..., runLen)`, which clamps at the nine available digits.  
SUGGESTED: A run of `f` selects fractional-second digits: one through nine `f` characters produce that many leading nanosecond digits; nine or more produce all nine digits.

### 2. Apostrophes are not escaped inside quoted literals

UNIT:      man-page:datetime/format  
CLAIM:     “to emit a literal apostrophe, write two single quotes ('').”  
VERDICT:   misleading  
EVIDENCE:  Probe `datetime::format(dt, "'it''s'")` printed `quote=its`, not `it's`. `src/codegen/builtins/datetime/func_format.rs:__datetime_format` treats an apostrophe encountered while scanning quoted text as its closing quote; only a doubled quote encountered by the outer scanner emits an apostrophe.  
SUGGESTED: `''` by itself emits one apostrophe. A quoted literal cannot contain an apostrophe: its next apostrophe closes the literal.

### 3. The ErrOverflow row has no condition

UNIT:      man-page:datetime/format  
CLAIM:     “ErrOverflow — Arithmetic overflow or numeric conversion outside the destination range.”  
VERDICT:   incomplete  
EVIDENCE:  The descriptor declares `ErrOverflow` in `src/codegen/builtins/datetime/func_format.rs:register`. Probe `datetime::format(datetime::DateTime[..., offset = -9223372036854775808], "ZZ")` printed `overflow=TRUE` when trapping `errorCode::ErrOverflow`. `src/codegen/builtins/datetime/helper_offset_label_sep.rs:__datetime_offsetLabelSep` negates a negative offset before formatting it, which overflows for the most-negative `Integer`.  
SUGGESTED: State the condition beside the error: “ErrOverflow can occur when formatting date-time fields or an offset requires arithmetic outside the `Integer` range; for example, the most-negative `Integer` offset cannot be converted to a positive magnitude.”