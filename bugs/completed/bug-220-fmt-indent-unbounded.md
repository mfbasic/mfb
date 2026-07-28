# bug-220: `mfb fmt --indent` accepts an unbounded value → repeat-overflow panic / multi-GB allocation

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: footgun

Status: Fixed (2026-07-15) — parse_indent now rejects a value > MAX_INDENT (256), mirroring parse_spec_width's clamp, so `mfb fmt --indent` can no longer drive indent_str's " ".repeat(level*width) into an overflow panic / multi-GB allocation.
Regression Test: verified — `mfb fmt --indent=18446744073709551615` prints a clean range error instead of panicking.

`parse_indent` (`src/cli/fmt.rs:69-73`) accepts any `usize` with no upper bound,
unlike the sibling `parse_spec_width` (`src/cli/spec.rs:79`) which clamps to
`20..=1000`. An absurd value drives `indent_str`'s `" ".repeat(level * width)`
(`src/fmt.rs:115`) into a multiply-overflow / capacity-overflow panic or a
multi-GB allocation.

Trigger: `mfb fmt --indent=18446744073709551615 file.mfb` (or a fat-fingered
`--indent=100000000`) on a file with any nesting → panic/OOM instead of a clean
error.

Fix: clamp or range-check the parsed indent (e.g. reject `> ~256`) as spec width
already does.
