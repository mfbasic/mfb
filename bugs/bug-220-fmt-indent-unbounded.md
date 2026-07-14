# bug-220: `mfb fmt --indent` accepts an unbounded value → repeat-overflow panic / multi-GB allocation

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: footgun

Status: Open

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
