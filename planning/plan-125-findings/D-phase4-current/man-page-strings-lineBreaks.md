### 1. Mandatory-break list omits vertical tab and NEL
UNIT:      man-page:strings/lineBreaks
CLAIM:     "`mandatory`, which is `TRUE` where the line **must** break: after a line feed, a carriage return (but not between a CR and the LF that follows it), a form feed, or a line or paragraph separator."
VERDICT:   incomplete
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-lineBreaks/probe/src/main.mfb` (built with `mfb build -q probe`, run `./probe/build/probe.out`) prints `a VT b len=3 : 2! 3!` and `a NEL b len=3 : 2! 3!`: U+000B (vertical tab) and U+0085 (next line) both produce a mandatory break, exactly like LF/FF. In `func_line_breaks.rs:LINE_BREAKS`, VT is class BK (LB4) and NEL is class NL (LB5), so this is by design, not a bug. The page lists five of the seven hard-break characters.
SUGGESTED: "…where the line **must** break: after a line feed, a vertical tab, a form feed, a carriage return (but not between a CR and the LF that follows it), a next-line character (U+0085), or a line or paragraph separator."

### 2. "after a hyphen" is not unconditional
UNIT:      man-page:strings/lineBreaks
CLAIM:     "after a hyphen but not before it"
VERDICT:   misleading
EVIDENCE:  Same probe: `hyphen after sp len=4 : 2 4!` for `"a -b"` (no break after the hyphen; LB20a, a hyphen that starts a word), and `hyphen num len=3 : 3!` for `"x-5"` (no break after the hyphen before a digit; LB25). Only `"ab-cd"` gives `3 5!`, the break after the hyphen. The page states the rule as if it always holds, and a reader wrapping `"-5"` or `"an -ing suffix"` will not get a break there.
SUGGESTED: "after a hyphen between two words (`ab-cd`), but never before one, and not when the hyphen starts a word (` -ing`) or a signed number (`-5`)"

### 3. Spaces before a break belong to the line before it; the page never says so, and its own example wraps early because of it
UNIT:      man-page:strings/lineBreaks
CLAIM:     "Wrapping is your loop: walk the breaks, measure the text from the line's start to each one, and break at the last one that still fits"
VERDICT:   incomplete
EVIDENCE:  Probe prints `a b len=3 : 2 3!` and `a  b len=4 : 3 4!`: the reported index is after every space, so a space run is part of the preceding line and `b.index - start` counts it. The page's example compiled and ran as written (`/tmp/plan-125-scratch/D-phase4-current/man-page-strings-lineBreaks/example`), output:
```
|The price is|
|$(12.35) — about|
|€11 — per box, (tax-|
|free).|
```
`"$(12.35) — about €11"` is exactly 20 scalars (`python3 -c 'print(len("$(12.35) — about €11"))'` → 20) and the width is 20, yet the example pushes `€11` to the next line because it measures the trailing space. A second input, `wrap("abcde fghij klmno", 11)`, gives `|abcde|` / `|fghij klmno|` although `abcde fghij` is 11 wide. Replacing the test with the trimmed length (`fixed/src/main.mfb`, same build) yields `|$(12.35) — about €11|` / `|— per box, (tax-|` / `|free).|` and `|abcde fghij|` / `|klmno|`. This is the sharp edge a wrapping loop hits first, and the page neither states it nor handles it.
SUGGESTED: Add to the Description: "A break comes after any spaces that precede it, so the spaces belong to the line before the break and are part of what `strings::mid(text, start, b.index - start)` returns. Measure a line with its trailing spaces removed (`strings::trimEnd`), or a line that fits exactly will wrap one word early." And in the example change the test to `IF len(strings::trimEnd(strings::mid(text, start, b.index - start))) > width AND last > start THEN`, which makes the shown output `|The price is|`, `|$(12.35) — about €11|`, `|— per box, (tax-|`, `|free).|`.

### 4. Example comment overstates its own output
UNIT:      man-page:strings/lineBreaks
CLAIM:     "' Wrap `text` to lines of at most `width` characters, breaking only where the algorithm allows."
VERDICT:   example-broken
EVIDENCE:  Same run as finding 3. The comment is literally true, but a developer running the example against its own input sees a 16-character line followed by `€11` on the next line when 20 fits, and has no way to know this is the trailing-space edge rather than the algorithm. Resolved by the fix in finding 3; listed separately so the example edit is not skipped if the prose edit lands alone.
SUGGESTED: Apply the measurement change in finding 3; the comment can then stay as written.

Everything else on the page checked out against the probe: empty text returns an empty list (`empty len=0 :`), position 0 is never reported even for a leading LF (`LF a len=2 : 1! 2!`), the end is always reported as mandatory, CR LF is a single break (`a CRLF b : 3! 4!`), CJK breaks between every character (`CJK : 1 2 3 4!`), no break before `)`, `!` or `.` or after `(`, none inside `$(12.35)` or `1,000.00`, none inside a ZWJ emoji sequence or a flag pair or `e + U+0301`, NBSP and word joiner glue (`3!` only), Korean breaks between syllables (`1 2 3!`), Thai breaks only at the space (`7 11!`), soft hyphen is a break opportunity (`3 5!`), and `index` is the scalar position `strings::left`, `strings::mid` and `strings::find` use (`left(6)=|héllo |`, `find w=6` on `"héllo wörld"`). The Errors table has one row, `ErrOutOfMemory`, matching `errors: vec!["ErrOutOfMemory"]` in the descriptor. No banned vocabulary or compiler-model sentences appear in the rendered page. No files in the repository were edited.
