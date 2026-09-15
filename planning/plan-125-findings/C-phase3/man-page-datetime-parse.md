### 1. Empty inputs are undocumented
UNIT:      man-page:datetime/parse
CLAIM:     “The text to parse.”
VERDICT:   incomplete
EVIDENCE:  `/tmp/plan-125-scratch/C-phase3/man-page-datetime-parse/probe/src/main.mfb` printed `emptyempty=1970-01-01 00:00:00.000000000 +0000`; `src/codegen/builtins/datetime/helper_parse_fields.rs:11-138` initializes all fields to epoch defaults and accepts an empty pattern/value.
SUGGESTED: “The text to parse. With an empty pattern, an empty value returns `1970-01-01 00:00:00 UTC`; other unmatched text raises `ErrInvalidFormat`.”

### 2. Documented token set omits accepted token runs
UNIT:      man-page:datetime/parse
CLAIM:     “The recognized tokens are:”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/datetime/helper_parse_fields.rs:40-124` accepts every run length for `y`, `f`, and `Z`, rather than just the listed runs. The probe printed `y=0006-01-01 00:00:00.000000000 +0000` for pattern `y`, and `z4=2026-01-01 00:00:00.000000000 +0000` for pattern `ZZZZ`.
SUGGESTED: “A run of formatting letters is parsed as a token. `yy` adds 2000; other `y` runs provide the year directly. The documented widths are the portable forms.”

### 3. `yy` does not require two digits
UNIT:      man-page:datetime/parse
CLAIM:     “`yyyy` reads up to 4 digits, `yy` reads 2 digits and adds 2000 (so `26` becomes `2026`)”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/datetime/helper_parse_fields.rs:45-51` calls `__datetime_readNum(..., runLen)`, and `helper_read_num.rs:11-18` requires at least one digit, not exactly `maxDigits`. The probe printed `yyone=2006-01-01 00:00:00.000000000 +0000` for `datetime::parse("6", "yy")`.
SUGGESTED: “`yyyy` accepts one to four digits. `yy` accepts one or two digits and adds 2000 (`26` becomes `2026`).”

### 4. Weekday token can be empty and accepts ASCII letters only
UNIT:      man-page:datetime/parse
CLAIM:     “`EEE` / `EEEE` — weekday name; the letters are read but not validated”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/helper_skip_weekday_name.rs:11-19` skips zero or more characters, while `helper_is_letter.rs:11-13` recognizes only `A-Z` and `a-z`. The probe printed `emptyweekday=2026-01-01 00:00:00.000000000 +0000` for value `"2026-01-01 "` and pattern `"yyyy-MM-dd EEE"`.
SUGGESTED: “`EEE` / `EEEE` — skips zero or more ASCII letters without validating a weekday or checking it against the date.”