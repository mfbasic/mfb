### 1. Negative sub-millisecond spans do affect output
UNIT:      man-page:datetime/formatDuration
CLAIM:     “The span is reduced to whole milliseconds before formatting: the value used is d.seconds * 1000 + d.nanos / 1000000, so any sub-millisecond remainder in the nanos field is truncated and does not appear in the output.”
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/datetime/func_format_duration.rs:__datetime_formatDuration` uses that arithmetic after `datetime::duration` normalizes negative nanoseconds. Probe `datetime::formatDuration(datetime::duration(0, -999_999))` printed `-00:00:00.001`; `datetime::formatDuration(datetime::duration(0, -1))` also printed `-00:00:00.001`. Thus a negative span smaller than one millisecond is displayed as negative one millisecond, rather than disappearing.
SUGGESTED: “Formatting uses whole milliseconds. Positive sub-millisecond spans render as `00:00:00.000`; a negative span with any nonzero sub-millisecond part renders as `-00:00:00.001`.”

### 2. Overflow row omits its triggering condition
UNIT:      man-page:datetime/formatDuration
CLAIM:     “ErrOverflow — Arithmetic overflow or numeric conversion outside the destination range.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/func_format_duration.rs:__datetime_formatDuration` computes `d.seconds * 1000 + d.nanos / 1000000` and negates a negative total. Probe `datetime::formatDuration(datetime::duration(9_223_372_036_854_776))` printed `Error: 7-705-0010` followed by the rendered error message. The page’s prose mentions overflow but does not connect the Errors row to the formatting calculation.
SUGGESTED: “Raises `ErrOverflow` when converting the duration to milliseconds, or when taking the magnitude of that millisecond total, exceeds the `Integer` range.”