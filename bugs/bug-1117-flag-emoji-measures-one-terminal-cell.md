# bug-1117: a flag emoji measures and draws as one terminal cell; terminals show it as two

Last updated: 2026-10-26
Effort: medium (1h–2h)
Severity: LOW
Class: Runtime (terminal layout)

Status: Open
Regression Test: —

`term::measureText("🇯🇵")` returns `1`, and `term::drawText(0, 0, "🇯🇵|")` puts
`|` in the very next cell. Terminals draw a regional-indicator pair as a
two-column glyph, so the `|` lands on the flag's second half and the rest of the
row shifts. `😀` measures `2` correctly.

**The correct behavior a fix produces:** a cluster of two regional indicators
measures `2` and is stamped as a wide cell (with the following text after its
second column) by `measureText`, `drawText` and `io::write` on the console and
all app backends; a lone regional indicator keeps its current width.

References:

- Found by plan-125-D Phase 6, finding `term-measureText` 5.
- Width comes from the base scalar alone
  (`src/codegen/builtins/term/func_measure_text.rs:measure_text_of` and the
  shared charwidth table, where regional indicators are width 1); the console
  grid writer and the three app backends compute it the same way, so the fix
  touches each.

## Failing Reproduction

Under a pty, after `term::on()`:

```
LET flag AS Integer = term::measureText("🇯🇵")
```

- Observed: `flag=1`.
- Expected: `flag=2`.
