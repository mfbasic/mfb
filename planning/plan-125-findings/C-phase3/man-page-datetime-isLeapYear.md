### 1. Parameter omits valid range and zero behavior
UNIT:      man-page:datetime/isLeapYear
CLAIM:     "The calendar year to test, as written. Follows the Gregorian rule, so 1900 is not a leap year and 2000 is."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/func_is_leap_year.rs:__datetime_isLeapYear` directly applies `MOD` to its `Integer` parameter and declares no errors. Scratch probe `is_leap_probe.mfb`, built and run with the specified release binary, printed `TRUE` for years `0`, `-4`, `-400`, and `-9223372036854775808`; it printed `FALSE` for `-100` and `9223372036854775807`.
SUGGESTED: Any `Integer` calendar year to test. Year `0` and negative years use the same Gregorian divisibility rule; for example, `-4` is a leap year, while `-100` is not.