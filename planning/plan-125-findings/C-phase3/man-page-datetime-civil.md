### 1. Identical-offset zones can name the same instant
UNIT:      man-page:datetime/civil
CLAIM:     "The zone the wall-clock time is read in. This is what decides which instant the pair names; the same date and time in two zones are two different instants."
VERDICT:   wrong
EVIDENCE:  Scratch probe resolved `civil(2026-06-26 09:30, utc())` and `civil(2026-06-26 09:30, fixedOffset(0))`; it printed `1782466200=1782466200`. Command: `TZ=UTC /tmp/plan-125-scratch/C-phase3/man-page-datetime-civil/build/man_page_datetime_civil.out`.
SUGGESTED: The zone decides which instant the pair names; using a zone with a different offset can name a different instant.

### 2. `civil` is not pure for a local zone
UNIT:      man-page:datetime/civil
CLAIM:     "`civil` is pure: beyond what `zone` itself resolves it reads no host state and has no side effects."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/datetime/func_civil.rs:__datetime_civil` calls `__datetime_resolveLocal`; `src/codegen/builtins/datetime/helper_resolve_local.rs:__datetime_resolveLocal` calls `__datetime_offsetAt`; `src/codegen/builtins/datetime/func_offset_at.rs:__datetime_offsetAt` calls `datetime::localOffset` for a local zone. The identical probe printed `2026-06-26 09:30:00 +00:00` with `TZ=UTC` and `2026-06-26 09:30:00 -04:00` with `TZ=America/New_York`.
SUGGESTED: With `datetime::utc` or `datetime::fixedOffset`, `civil` depends only on its arguments. With `datetime::local`, it reads the host’s configured time zone, so the same date and time can produce a different result on another host or under different `TZ` settings.

### 3. DST probing algorithm is compiler-facing detail
UNIT:      man-page:datetime/civil
CLAIM:     "It probes the zone's offset one day before and one day after the named local time to bracket any single nearby transition."
VERDICT:   out-of-scope
EVIDENCE:  This describes the implementation algorithm in `src/codegen/builtins/datetime/helper_resolve_local.rs:__datetime_resolveLocal` (`localSeconds - 86400` and `localSeconds + 86400`), rather than a developer-visible contract.
SUGGESTED: For a daylight-saving transition, `civil` resolves an ambiguous or skipped local time deterministically.