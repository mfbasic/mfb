### 1. Year parameter omits its valid range
UNIT:      man-page:datetime/daysInMonth  
CLAIM:     “The calendar year. It matters: February has 29 days in a leap year and 28 otherwise.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/datetime/func_days_in_month.rs:__datetime_daysInMonth` has no year-range check. A compiled probe printed `negative-years=29,29,28,29` for years `0,-4,-100,-400` with February.  
SUGGESTED: The calendar year. Zero and negative years are valid; February has 29 days in a leap year and 28 otherwise.

### 2. Month parameter falsely implies a restricted domain
UNIT:      man-page:datetime/daysInMonth  
CLAIM:     “The month, 1 through 12.”  
VERDICT:   misleading  
EVIDENCE:  `src/codegen/builtins/datetime/func_days_in_month.rs:__datetime_daysInMonth` returns 31 in its default branch. A compiled probe printed `unbounded-month=31,31,31,31` for months `0,-1,13,14`; it did not raise.  
SUGGESTED: The month number. Values 1 through 12 designate calendar months; every other value returns 31.