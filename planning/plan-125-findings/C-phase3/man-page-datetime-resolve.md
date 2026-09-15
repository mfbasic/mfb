### 1. DST resolution is attributed to the wrong call
UNIT:      man-page:datetime/resolve  
CLAIM:     “The date-time to resolve. This is where an ambiguous or non-existent local time — the two that a daylight-saving transition creates — gets settled.”  
VERDICT:   wrong  
EVIDENCE:  `src/codegen/builtins/datetime/func_resolve.rs:__datetime_resolve` only computes `localSeconds - dt.offset`; it neither reads `dt.zone` nor resolves a transition. DST gap/overlap settlement is in `src/codegen/builtins/datetime/helper_resolve_local.rs:__datetime_resolveLocal`, called by `datetime::civil`.  
SUGGESTED: “The date-time to resolve. `resolve` uses its stored UTC offset directly; it does not look up or settle a zone transition.”

### 2. Manually built records are not validated
UNIT:      man-page:datetime/resolve  
CLAIM:     “A `datetime::DateTime` record you build yourself with a year far enough from the epoch that the second count leaves the `Integer` range raises `ErrOverflow`.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/datetime/func_resolve.rs:__datetime_resolve` has no validation of `Date`, `Time`, `nanos`, or `offset`. Probe printed `invalid=1739421639,-1` for `DateTime[Date[2024,13,40], Time[99,99,99,-1], utc(), 0]`: it returned an instant rather than raising.  
SUGGESTED: “For a record built directly, `resolve` uses the stored fields and offset as supplied; it does not validate them. It raises `ErrOverflow` only if its integer arithmetic overflows.”

### 3. The documented overflow condition omits offset subtraction
UNIT:      man-page:datetime/resolve  
CLAIM:     “A `datetime::DateTime` record you build yourself with a year far enough from the epoch that the second count leaves the `Integer` range raises `ErrOverflow`.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/datetime/func_resolve.rs:__datetime_resolve` returns `Instant[localSeconds - dt.offset, ...]`. A probe using `DateTime[Date[1970,1,1], Time[0,0,0,0], utc(), -9223372036854775808]` printed `offset_overflow=TRUE`; the date’s second count itself is zero.  
SUGGESTED: “`ErrOverflow` is raised if converting the date and time to seconds, or subtracting the stored offset, leaves the `Integer` range.”

### 4. Algorithm-level detail is outside the man-page audience
UNIT:      man-page:datetime/resolve  
CLAIM:     “`resolve` first converts the civil date (`dt.date.year`, `dt.date.month`, `dt.date.day`) to a day count with the proleptic Gregorian calendar, multiplies by `86400` to get seconds, and adds the time-of-day contribution (`dt.time.hour * 3600 + dt.time.minute * 60 + dt.time.second`).”  
VERDICT:   out-of-scope  
EVIDENCE:  This is an implementation walkthrough of `src/codegen/builtins/datetime/func_resolve.rs:__datetime_resolve` and `helper_days_from_civil.rs:__datetime_daysFromCivil`; `.ai/man-content.md` §3 excludes details requiring a compiler/source-reading mental model.  
SUGGESTED: “`resolve` combines the stored civil date, time, and UTC offset into an absolute instant.”