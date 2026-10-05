# bug-906: runtime grapheme segmentation treats 5,222 unassigned code points as Other — diverges from UAX #29 and from the folded path

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/strings/grapheme-unassigned-props-rt (proposed)

UAX #29 assigns Grapheme_Cluster_Break and Extended_Pictographic to **unassigned** code
points too: 3,769 reserved Default_Ignorable code points are `Control`
(U+2065, U+FFF0..U+FFF8, U+E0000, U+E0002..U+E001F, U+E0080..U+E0FFF) and 1,453 reserved
code points in the emoji blocks are `Extended_Pictographic` (U+1F02C..U+1F02F,
U+1FA58..U+1FA5F, ... U+1FFFD). The emitted runtime segmenter (`strings::graphemes`,
`graphemesCount`, `graphemeAt`, `displayWidth`, `term::` placement) reads its boundclass
from the vendored utf8proc property trie, which has records only for assigned characters
and returns `OTHER` for all of these. So at run time `U+2065 U+0301` is one cluster
(GB9 instead of GB4/GB5) and `U+1F600 U+200D U+1F02C` is two clusters (GB11 missed).
The compile-time path (`unicode-segmentation`) gets both right, so the same literal gives a
different answer depending on whether the call was folded — a divergence the spec does not
list (it claims the only fold/runtime differences are versioned-data ones, and both paths
here are Unicode 17-era data).

Correct behavior: the runtime boundclass of every code point, assigned or not, equals
GraphemeBreakProperty.txt / emoji-data.txt Extended_Pictographic for the vendored Unicode
version, so runtime and folded segmentation agree.

References:

- `src/docs/spec/unicode/01_tables-and-algorithms.md` — "Runtime algorithm: grapheme segmentation ... based on UAX #29"; `spec.md` — fold vs runtime differences are "specified in tables-and-algorithms" (this one is not).
- `src/docs/spec/unicode/02_strings-model.md` — "Grapheme ... extended grapheme cluster".
- UAX #29 §3.1 / GraphemeBreakProperty.txt (`2065 ; Control # Cn <reserved-2065>`), emoji-data.txt (`1F02C..1F02F ; Extended_Pictographic # <reserved>`).

## Failing Reproduction

```
mkdir -p /tmp/g && cd /tmp/g && mfb init . >/dev/null
cat > src/main.mfb <<'EOF'
IMPORT io
IMPORT strings
FUNC rt(s AS String) AS Integer
  RETURN len(strings::graphemes(s))
END FUNC
FUNC main() AS Integer
  io::print("U+2065 U+0301 folded=" & toString(len(strings::graphemes("\u{2065}\u{301}"))) & " runtime=" & toString(rt("\u{2065}\u{301}")))
  io::print("U+E0080 U+0301 folded=" & toString(len(strings::graphemes("\u{E0080}\u{301}"))) & " runtime=" & toString(rt("\u{E0080}\u{301}")))
  io::print("U+1F600 U+200D U+1F02C folded=" & toString(len(strings::graphemes("\u{1F600}\u{200D}\u{1F02C}"))) & " runtime=" & toString(rt("\u{1F600}\u{200D}\u{1F02C}")))
  io::print("U+1F600 U+200D U+1FAFA folded=" & toString(len(strings::graphemes("\u{1F600}\u{200D}\u{1FAFA}"))) & " runtime=" & toString(rt("\u{1F600}\u{200D}\u{1FAFA}")))
  RETURN 0
END FUNC
EOF
mfb build && ./build/*.out
```

- Observed:
```
U+2065 U+0301 folded=2 runtime=1
U+E0080 U+0301 folded=2 runtime=1
U+1F600 U+200D U+1F02C folded=1 runtime=2
U+1F600 U+200D U+1FAFA folded=1 runtime=2
```
- Expected: `folded == runtime` on every line, with the folded (UAX #29) values.

Measured scope: the runtime segmenter passes all 766 cases of Unicode 17.0.0
GraphemeBreakTest.txt (`/tmp/hardening/B10/gtest.py`), and a 331k-case sweep of every
code point with a non-default property in 10 contexts against a Python UAX #29 oracle
(`/tmp/hardening/B10/gsweep.py`, UCD=ucd17) fails only on these unassigned code points.
Parsing `third_party/utf8proc/utf8proc_data.c` directly (`/tmp/hardening/B10/u8p.py`):
3,769 code points expected Control are `UTF8PROC_BOUNDCLASS_OTHER`, 1,453 expected
Extended_Pictographic are `OTHER`; there are no other boundclass mismatches.

## Root Cause

`src/unicode/runtime_tables.rs:parse_tables` takes boundclass verbatim from utf8proc's
`utf8proc_properties`; utf8proc 2.11.3 maps every unassigned code point to the default
record (boundclass OTHER). `src/codegen/string/unicode_props.rs:emit_grapheme_break_branch`
is correct for the classes it is given. The folded path
(`src/unicode/backend.rs:graphemes`, `unicode-segmentation`) uses full UCD ranges.

## Goal

- Runtime boundclass for unassigned code points matches GraphemeBreakProperty /
  Extended_Pictographic (e.g. by overlaying those ranges when building the trie in
  `parse_tables`), and the four lines above print equal counts.

### Non-goals (must NOT change)

- Assigned-code-point behavior (passes GraphemeBreakTest 17.0.0 today), widths, NFC.

## Blast Radius

- `strings::graphemes` / `graphemesCount` / `graphemeAt` (`gen_graphemes.rs`) — fixed by this bug
- `strings::displayWidth` runtime path and `term::` grapheme placement (same boundclass lookups via `emit_grapheme_break_branch_free`) — fixed by this bug
- regex — unaffected (does not segment graphemes)
