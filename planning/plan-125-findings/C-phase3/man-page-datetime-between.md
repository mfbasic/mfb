### 1. Overflow condition omits legal inputs

UNIT:      man-page:datetime/between  
CLAIM:     "The subtraction and the normalizing carry are ordinary signed Integer arithmetic, so two instants far enough apart that their second difference falls outside the signed Integer range overflow and trap."  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/datetime/func_between.rs:BODY` subtracts both public fields before `__datetime_normDuration`; `helper_norm_duration.rs:BODY` does not validate inputs. The probe at `/tmp/plan-125-scratch/C-phase3/man-page-datetime-between/raw-instant/src/main.mfb` called `between(Instant[0, minI()], Instant[0, maxI()])`; it compiled and printed `rawNanosOverflow=TRUE`. The seconds difference is zero, but the nanos subtraction overflows and raises `ErrOverflow`.  
SUGGESTED:  "All intermediate Integer calculations can overflow and raise ErrOverflow, including either field subtraction and normalization; this can occur even when the two seconds fields are equal if an Instant was written with an extreme nanos field."

### 2. Parameter descriptions omit accepted non-normalized instants

UNIT:      man-page:datetime/between  
CLAIM:     "The earlier instant. If it is actually the later one the result is negative."  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/datetime/func_between.rs:register` accepts two `Instant` records without validation, and its BODY forwards their fields directly to normalization. The same raw-record probe compiled `datetime::Instant[0, minI()]` and `datetime::Instant[0, maxI()]`, proving callers can supply non-normalized `Instant` field values.  
SUGGESTED:  "The starting instant. Any datetime::Instant value is accepted; between normalizes the returned duration. If start is later than endTime, the result is negative."