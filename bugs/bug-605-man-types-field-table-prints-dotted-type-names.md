# bug-605: `mfb man <pkg> types` field tables print internal dotted type names (`color.Color`)

Last updated: 2026-09-12
Effort: small (<1h)
Severity: LOW
Class: Documentation (renderer)

Status: Open
Regression Test: none yet — see Phase 1

A record's field table on a types page spells a package type the way the compiler
stores it, with a dot, instead of the way source writes it:

```
│ color  │ color.Color │ The colour at that offset. A color::Color — the …
│ font   │ RES canvas.Font │ The font to draw it in. …
│ from   │ net.Address │ The address the datagram was sent from. …
```

MFBASIC source writes `color::Color`; `color.Color` is not valid syntax, and the
description in the same row says `color::Color`. The Parameters table on every
function page already converts the spelling.

**The single correct behavior a fix produces:** every Type cell in a types-page
field table uses `::` for a package-qualified type, exactly as the Parameters
table does. The six cells below read `color::Color`, `RES canvas::Font`,
`RES canvas::Image` and `net::Address`.

Found by plan-125-B while sweeping the rendered man surface for dotted names.
**Filed, not fixed**, by user instruction during a documentation-only plan
("file all bugs, make no fixes").

## Reproduction

```
mfb man canvas types | grep 'color\.Color\|canvas\.Font\|canvas\.Image'
mfb man udp types | grep 'net\.Address'
```

Observed at `worktree-P-125` HEAD: six cells in total —
`canvas::GradientStop.color`, `canvas::Paint.fill`, `canvas::Paint.stroke`,
`canvas::Text.font`, `canvas::Picture.image`, `udp::Datagram.from`.
(`python3 /tmp/p125-ex/dotted.py` walks `mfb man --all` for every table cell of
the form `pkg.Type`; those six are all it finds.)

Expected: no output from either command.

## Root cause

`src/cli/man.rs:render_types_markdown` writes each field's type with
`prop.ty.name()`, the internal dotted spelling. The function-page Parameters table
goes through `src/cli/man.rs:public_type_name`, which is
`ty.name().replace('.', "::")`. The field table was never routed through it.

## Non-goals

- Changing type IDs (`COLOR_TYPE_ID = "color.Color"` and friends). They are the
  compiler's internal keys and correct as they are.
- Editing descriptor prose to work around the cell.

## Blast-radius audit

- `grep -n '\.name()' src/cli/man.rs`: line 417 (this bug), line 515 (inside
  `public_type_name` itself — correct), line 996 (a diagnostic list in a check,
  not rendered prose — unaffected).
- Union member and resource sections of `render_types_markdown`: no raw `.name()`
  call, so they are unaffected.

## Fix

Phase 1 — a `man.rs` unit test that renders a record with a
`ParameterType::named("color.Color")` field and asserts the cell is
`` `color::Color` `` (RED). Commit:

Phase 2 — use `public_type_name(&prop.ty)` at `render_types_markdown`'s field row
(GREEN); run the `cli::man` tests and `tests/cli/cli_man_summary_plain.rs`.
Commit:
