# bug-133 — `find(String, String, start)` returns wrong index when char at `start-1` is multibyte

**Status:** FIXED (commit e0fa88b8, 2026-07-11).
**Severity:** HIGH — wrong results from a core string builtin over any non-ASCII
text with a start offset.
**Class:** correctness (UTF-8 boundary).

## Finding

`src/target/shared/code/builder_search.rs:168-181` (`lower_find` locate_start
loop). The start-offset walk increments `scalar_index` upon seeing a *lead* byte
and re-checks `scalar_index == start` before consuming that character's
continuation bytes, so the search cursor lands on a continuation byte whenever
character `start-1` is multibyte. The candidate loop then re-counts the leftover
continuation bytes as a character advance, inflating every returned index by 1
(and wasting comparisons mid-char). Contrast `lower_mid` (:672-691), which
correctly skips continuations before re-checking equality.

## Trigger

`find("éb", "b", 1)` → 2 (expected 1). Any `find(s, n, i)` where `s` has a
non-ASCII char at scalar `i-1` — e.g. an iterate-all-matches loop `i = find(s,
n, i+1)` over Unicode text yields wrong indices that then feed `mid()` etc.

## Fix

Mirror `lower_mid`'s structure: consume all continuation bytes of a character
before re-checking `scalar_index == start`, so the cursor lands on a character
boundary.

## Prior art

audit-unicode #5 (retracted fold-parity) and #6 (negative-start) don't cover
this; the only start-offset runtime tests (unicode-05/06 fixtures) are ASCII.

## Resolution

FIXED in commit e0fa88b8. lower_find consumes a whole character before re-checking the start offset (mirrors lower_mid).

Regression test: `tests/rt-behavior/general/bug133_find_multibyte_start` (fails on the unfixed compiler). Full
acceptance (871) and `cargo test` pass.
