# bug-921: `mfb fmt` deletes a comma-only line inside a bracket, shifting every later source line (changes `e.source.line` and test output)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: src/fmt.rs unit test + a CLI fmt golden (new; names suggested)

The leading-comma rewrite (bug-821) moves a line-leading `,` inside an open bracket
to the end of the previous code line. When the line held **only** the comma, that
physical line is deleted. Every later line then moves up by one, and line numbers
are observable at runtime:

- A trapped error's `e.source.line` changes value (measured: `7` becomes `6`).
- `mfb test` failure locations change (`(src/main.mfb:11)` becomes `(src/main.mfb:10)`).
- Every later diagnostic, debugger line and the IR's `line` fields shift.

So formatting changes program output. The fmt spec also says the opposite in its
first paragraph: "String contents, comments, blank lines, **line breaks** … are
preserved". It then says, in *Leading commas*, "A line that held nothing but the
comma is dropped". The two sentences contradict each other.

**Correct behavior:** fmt never changes the number of physical lines. A
comma-only line whose comma moves is kept as an empty line, which the bracket
continuation rules already accept (blank lines inside a bracket are kept). The
spec sentence is corrected to say so.

References:

- `src/docs/spec/tooling/05_fmt.md` intro ("line breaks … are preserved") vs. *Leading commas* ("A line that held nothing but the comma is dropped").
- bug-821 (introduced the leading-comma normalization).

## Failing Reproduction

```
mkdir -p /tmp/f4/src && cd /tmp/f4
printf '{"name":"f4","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src"}]}' > project.json
cat > src/main.mfb <<'EOF'
IMPORT io

FUNC main() AS Integer
  LET xs = [ 1
           ,
             2 ]
  LET n = toInt("bad")
  RETURN n + len(xs)
TRAP(e)
  io::print("line=" & toString(e.source.line))
  RETURN 0
END TRAP
END FUNC
EOF
mfb build && ./build/f4.out      # line=7
mfb fmt   && mfb build && ./build/f4.out   # line=6
```

- Observed: `line=7` before fmt, `line=6` after. The `--ir` output differs only in `line` fields.
- Expected: `line=7` both times.

The same thing happens in a `TESTING` block: `X expected 3, got 2  (src/main.mfb:11)` becomes `(src/main.mfb:10)`.

Contrast: a leading comma followed by an element (`, "AFN"`) keeps its line, and
the whole 1,049-project buildable test corpus (mangled: indentation stripped and
keywords lowercased, then formatted) produces identical IR, because no fixture
has a comma-only line.

## Root Cause

`src/fmt.rs:format_source` gathers a logical line and pushes each scanned
physical line unless
`scanned.leading_comma.is_some() && scanned.cased.trim().is_empty()` ("A line left
holding nothing but the comma it gave up is dropped"). Removing that physical
line is what shifts the numbering. `normalize_leading_comma` itself is fine.

## Goal

- `mfb fmt` output has exactly as many lines as its input (bar the final-newline rule), and the repro prints `line=7` after formatting.

### Non-goals (must NOT change)

- The comma move itself, and its idempotence.
- Do not "fix" by declining to move the comma.

## Blast Radius

- `src/fmt.rs:format_source`, the drop branch: fixed.
- `src/fmt.rs` unit tests that assert the dropped-line output (search for the bug-821 tests): they must be re-baselined to the blank-line form, with this report as the proof.

## Validation Plan

- Unit test: `"[ 1\n,\n2 ]"` formats to three lines.
- Runtime proof: the repro above.
- Doc sync: `05_fmt.md` *Leading commas*: "a line that held nothing but the comma is left empty".
