# bug-926: `mfb fmt` turns each leading tab in a DOC `EXAMPLE` into one space, changing the rendered example

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other

Status: Open
Regression Test: src/fmt.rs unit test for `flush_example` (new)

**Owner decision (2026-10-05):** neither (a) nor (b). `mfb fmt` formats the code inside a DOC `EXAMPLE` exactly as
it formats real source (indentation, keyword capitalization, the same rules as top-level code),
then prefixes every example line with 2 extra spaces of indentation relative to the `EXAMPLE`
block. Tabs therefore never survive. They are normalized the way real code's leading
whitespace is. Formatting an example twice is idempotent, and the rendered `mfb doc` HTML
shows the properly indented example. The tooling fmt spec gains this `EXAMPLE` rule in place of
the current verbatim-prefix behaviour.

`flush_example` re-anchors `EXAMPLE` source "preserving its indentation relative
to the least-indented line". It measures indentation in **bytes**
(`line.len() - line.trim_start().len()`) and re-emits the difference as that many
**spaces**. A tab-indented example line therefore loses all but one column per
tab. Example: an example whose lines are `\tIF x THEN` / `\t\tio::print(..)` /
`    io::print(..)` has minimum lead 1, so the second line keeps one relative
column (it had one extra tab) and the third keeps three (four spaces minus one).
Before fmt, the third line was visually *left* of the second (on an 8-column tab
stop). After fmt it is to the right. The `mfb doc` HTML (and the `.mfp` doc
section) changes after formatting.

**Correct behavior (needs owner decision):** either (a) expand tabs to a fixed
tab stop before measuring and emitting, so visual relative indentation survives,
or (b) keep each line's original leading whitespace after the common prefix
verbatim (tabs included). The spec's "the formatter never *emits* a tab" claim
would then need an `EXAMPLE` carve-out like the one it already has for
continuation lines.

References:

- `src/docs/spec/tooling/05_fmt.md` *DOC blocks*: "`EXAMPLE` source … preserving its indentation relative to the least-indented line in the example, so the code's own nesting survives".

## Failing Reproduction

```
mfb init-pkg /tmp/dt && cd /tmp/dt
printf 'DOC\n  FUNC answer\n  DESC   spaced    prose   kept\n  EXAMPLE\n\tIF x THEN\n\t\tio::print("tab")\n    io::print("four spaces")\n  END EXAMPLE\nEND DOC\nEXPORT FUNC answer() AS Integer\n  RETURN 42\nEND FUNC\n' > src/lib.mfb
mfb doc --out a.html; mfb fmt; mfb doc --out b.html; diff a.html b.html
```

- Observed:
  ```
  32c32
  < 	io::print(&quot;tab&quot;)
  ---
  >  io::print(&quot;tab&quot;)
  ```
  The formatted source is `    IF x THEN` / `     io::print("tab")` / `       io::print("four spaces")`.
- Expected: the rendered example keeps the same visual nesting before and after `mfb fmt`.

Contrast: space-only examples render identically before and after fmt (verified
on a 4-line example with mixed relative indents).

## Root Cause

`src/fmt.rs:flush_example`: `lead`/`min` are byte counts of leading whitespace,
and `" ".repeat(prefix + rel)` replaces whatever the whitespace was.

## Open Decisions

- (a) tab-expand, or (b) preserve the original whitespace after the common prefix.
