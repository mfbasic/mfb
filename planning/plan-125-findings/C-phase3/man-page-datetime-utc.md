### 1. Compiler-model enum tag
UNIT:      man-page:datetime/utc
CLAIM:     "The returned datetime::Zone carries a zone kind of datetime::ZoneKind::Utc (the first datetime::ZoneKind variant, tag 0), marking it as the canonical UTC zone rather than an arbitrary fixed offset built with datetime::fixedOffset (kind datetime::ZoneKind::FixedOffset)."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/datetime/func_utc.rs:BODY` returns `Zone[0, 0, "UTC"]`; the scratch probe printed `zone=0,0,UTC`. The numeric enum ordering and tag are implementation representation details, prohibited on developer pages by `.ai/man-content.md` §3.
SUGGESTED: "The returned `datetime::Zone` has kind `datetime::ZoneKind::Utc`, distinguishing the canonical UTC zone from a fixed-offset zone built with `datetime::fixedOffset`."

### 2. “Match the seconds-since-epoch” conflates fields with an instant count
UNIT:      man-page:datetime/utc
CLAIM:     "Because the offset is always zero, the civil fields of a datetime::DateTime in this zone match the seconds-since-epoch of the originating datetime::Instant directly, with no offset adjustment."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/datetime/func_in_zone.rs:BODY` converts seconds into calendar fields using day division and remainder. The scratch probe printed `before=1969-12-31T23:59:59Z` for `Instant[-1, 0]` and `after=1970-01-02T00:00:00Z` for `Instant[86400, 0]`: the fields represent the UTC instant, but do not “match” its integer seconds count.
SUGGESTED: "Because the offset is always zero, the civil fields are calculated from the originating `datetime::Instant` without an offset adjustment."