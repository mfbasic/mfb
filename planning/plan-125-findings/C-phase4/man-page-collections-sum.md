### 1. Float overflow claim contradicts executable

UNIT:      man-page:collections/sum  
CLAIM:     "The Float overload uses IEEE-754 double addition and never raises — an out-of-range total becomes ±Inf in the usual floating-point way."  
VERDICT:   wrong  
EVIDENCE:  Probe `/tmp/plan-125-scratch/C-phase4/man-page-collections-sum/src/float_overflow.mfb` binds `collections::sum([1.0e308, 1.0e308, 1.0e308])` to a `Float`; the release binary printed `Error: 7-705-0015` and `Floating-point arithmetic overflowed to infinity.`, exiting 255. `src/codegen/builtins/float_result.rs` classifies `collections.sum` as able to return a non-finite result that is checked at an observation boundary.  
SUGGESTED: The Float overload uses IEEE-754 double addition. If its result is infinite when it is bound, returned, passed, or printed, MFBASIC raises `ErrFloatOverflow`.

### 2. Inline-TRAP paragraph is false

UNIT:      man-page:collections/sum  
CLAIM:     "Note a wrinkle worth knowing before writing a handler: the compiler's inline-sum is treated as infallible for the purpose of inline TRAP, so attaching an inline TRAP to a sum call raises the TYPE_INLINE_TRAP_DEAD_HANDLER diagnostic and that handler does not receive the overflow. The overflow is still raised at run time and still propagates out of the enclosing function, where an ordinary function-level TRAP can handle it."  
VERDICT:   wrong  
EVIDENCE:  Probe `/tmp/plan-125-scratch/C-phase4/man-page-collections-sum/src/trap_probe.mfb` compiled with the stated warning, then printed `inline=77050010` and `after=-1`, exiting 0. Its inline handler did receive and recover from the Integer overflow; the enclosing function-level handler did not run.  
SUGGESTED: Remove this paragraph until the diagnostic and runtime behavior agree. It also exposes compiler-specific terminology that does not belong on a developer man page.

### 3. Errors table assigns ErrOverflow to Float

UNIT:      man-page:collections/sum  
CLAIM:     "`ErrOverflow` — Overloads `1, 2, 3`"  
VERDICT:   wrong  
EVIDENCE:  `src/codegen/builtins/collections/func_sum.rs:213-232` emits checked integer addition only for `Integer` and `Fixed`; its Float arm uses `float_add_d`. The Float-overflow probe instead raised `ErrFloatOverflow` (`77050015`), not `ErrOverflow` (`77050010`).  
SUGGESTED: `ErrOverflow` — overloads 1 and 3 only. Document Float non-finite results separately with their actual observation-boundary error.

### 4. “Rejects it a second time” leaks implementation detail

UNIT:      man-page:collections/sum  
CLAIM:     "There is no List OF Byte, no List OF Money, and no general \"any numeric list\" form: any other element type fails to resolve at compile time, and the call rejects it a second time."  
VERDICT:   out-of-scope  
EVIDENCE:  The invalid `List OF String` call is a compile-time error (`tests/syntax/collections/func_collection_sum_invalid/golden/build.log`); whether later implementation validation repeats that rejection is not developer-visible.  
SUGGESTED: `sum` accepts only `List OF Integer`, `List OF Float`, and `List OF Fixed`; calls with other element types are compile-time errors.

### 5. “Shares the Integer path” is compiler-facing

UNIT:      man-page:collections/sum  
CLAIM:     "Fixed shares the Integer path because it is a scaled 64-bit integer."  
VERDICT:   out-of-scope  
EVIDENCE:  `src/codegen/builtins/collections/func_sum.rs:213-232` confirms the implementation detail, but it does not help a terminal user use `sum`; the developer-visible contract is already the checked-overflow behavior.  
SUGGESTED: `Integer` and `Fixed` totals raise `ErrOverflow` instead of wrapping when the running total is out of range.