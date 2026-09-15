### 1. Internal zone representation exposed
UNIT:      man-page:datetime/offsetAt
CLAIM:     “For a local zone (`datetime::ZoneKind::Local`, built with `datetime::local`, internally zone kind `2`) the offset is resolved against the host's configured time zone for the specific instant `at`”
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/datetime/func_offset_at.rs:__datetime_offsetAt` tests `z.kind = 2`; that representation is not useful to an MFBASIC developer.
SUGGESTED: “For a local zone (`datetime::ZoneKind::Local`, built with `datetime::local`), the offset is resolved from the host's configured time zone for the specific instant `at`.”

### 2. Zone storage details exposed
UNIT:      man-page:datetime/offsetAt
CLAIM:     “For a UTC zone (`datetime::ZoneKind::Utc`) and a fixed-offset zone (`datetime::ZoneKind::FixedOffset`, built with `datetime::fixedOffset`) the function returns the zone's stored constant offset directly and does not consult `at` — the UTC zone stores zero, and a fixed zone stores its single configured offset.”
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/datetime/func_offset_at.rs:__datetime_offsetAt` returns `z.offsetSeconds` for non-local zones, but how a `Zone` stores this is an implementation detail.
SUGGESTED: “For UTC and fixed-offset zones, the result is constant: UTC returns zero, and a fixed-offset zone returns its configured offset regardless of `at`.”

### 3. OS-intrinsic terminology exposed
UNIT:      man-page:datetime/offsetAt
CLAIM:     “The function reads no host state for UTC and fixed zones (those are pure); for a local zone it reads the host's time-zone configuration through the `datetime::localOffset` OS intrinsic.”
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/datetime/func_offset_at.rs:__datetime_offsetAt` delegates local zones to `datetime::localOffset(at.seconds)`; calling that mechanism an “OS intrinsic” is compiler-facing terminology. Probe `/tmp/plan-125-scratch/C-phase3/man-page-datetime-offsetAt/src/main.mfb` printed `utc=0`, `fixed-a=-19800`, `fixed-b=-19800`, and equal local offsets for nanos `0` and `999999999`.
SUGGESTED: “UTC and fixed-offset zones give results determined only by the zone. A local zone uses the host's configured time zone.”