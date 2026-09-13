### 1. Type spelling contradicts the function pages
UNIT:      man-pkg:datetime  
PAGE:      package-wide  
CATEGORY:  consistency  
CLAIM:     “types are referenced bare (`datetime::Instant`, `datetime::Date`, …), not package-qualified.”  
VERDICT:   inconsistent  
EVIDENCE:  `mfb man datetime instant` declares `datetime::instant(...) AS datetime::Instant`; compiling a probe with `LET bare AS Instant = datetime::instant(0)` reports `SYMBOL_UNKNOWN_TYPE: Type 'Instant' is not a built-in or top-level project type.`  
SUGGESTED: “Use the package-qualified names `datetime::Instant`, `datetime::Date`, and so on after `IMPORT datetime`.”

### 2. The overview wrongly makes standalone Date and Time values projections
UNIT:      man-pkg:datetime  
PAGE:      package-wide  
CATEGORY:  overview-mismatch  
CLAIM:     “Everything civil — `datetime::Date`, `datetime::Time`, and `datetime::DateTime` — that this package produces is a projection of an instant through a `datetime::Zone`”  
VERDICT:   misleading  
EVIDENCE:  `src/codegen/builtins/datetime/func_date.rs:BODY` returns `Date[year, month, day]`, and `func_time.rs:BODY` returns `Time[hour, minute, second, nanos]`, neither accepting an instant or zone. A probe built and run with `mfb build /tmp/plan-125-scratch/B-phase2/man-pkg-datetime/type_probe` printed `2026 9` after calling only `datetime::date(2026, 6, 26)` and `datetime::time(9, 30)`.  
SUGGESTED: “`datetime::Date` and `datetime::Time` are standalone civil components. A `datetime::DateTime` produced by `civil`, `inZone`, or the conversion functions pairs those components with a zone and resolved offset.”