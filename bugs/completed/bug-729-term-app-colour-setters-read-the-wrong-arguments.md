# bug-729: in an `--app` build, `term::setForeground`/`setBackground` ignore the `color::Color` and set a garbage colour

Last updated: 2026-09-25
Effort: small
Severity: HIGH
Class: Correctness (wrong colour drawn; the argument is never read)

Status: FIXED BEFORE FILING — by 8030c1e3d (2026-09-21, "app mode: read term::setForeground/setBackground's color::Color record"), four days before this doc; the review ran an older binary. Verified at HEAD 2026-09-27: the repro as a headless macOS --app build prints fg=#0a141e bg=#28323c; regression test tests/codegen/codegen_term_color_record_offsets.rs:app_set_color_reads_the_color_record_fields covers macos-aarch64, linux-aarch64, linux-x86_64 and windows-x86_64. No code change.
Regression Test: tests/codegen/codegen_term_color_record_offsets.rs:app_set_color_reads_the_color_record_fields

`term::setForeground` and `term::setBackground` take one `color::Color`. The console
backend reads the red, green and blue fields off that record. The app backends were
never migrated when the signature changed from three scalars to one record: they
still pack their first three argument registers as `r`, `g`, `b` — the record
pointer's low byte plus two stale registers.

Measured on macOS (`/tmp/p125-ex/termappfg`, `mfb build --app`, run from the bundle):

| Call | `getForeground`/`getBackground` reads back | Expected |
|---|---|---|
| `setForeground(color::rgb(10, 20, 30))` | `#50d8df` | `#0a141e` |
| `setBackground(color::rgb(40, 50, 60))` | `#70f8ff` | `#28323c` |

The same program built for the console reads back the expected values (reviewer
probe `/tmp/plan-125-scratch/D-phase3/man-page-term-setForeground/probe`: `#0a141e`).

**The single correct behavior a fix produces:** on every app backend the two setters
load the channels off the `color::Color` record exactly as the console
`emit_set_color` does, so an `--app` program draws and reads back the colour it set.

Found by plan-125-D Phase 3 (the Fable review of `man-page:term/setForeground`).
**Filed, not fixed**, by user instruction during a documentation-only plan ("file
all bugs, make no fixes").

## Reproduction

```basic
IMPORT term
IMPORT color
IMPORT fs

SUB main()
  term::on()
  term::setForeground(color::rgb(10, 20, 30))
  LET c AS color::Color = term::getForeground()
  term::off()
  fs::writeText(fs::pathJoin([fs::tempDirectory(), "fg.txt"]), color::toHex(c))
END SUB
```

`mfb build --app`, run the bundle: the file holds `#50d8df`, not `#0a141e`.

## Root cause

`src/target/macos_aarch64/app/app_io.rs:emit_app_set_color` packs `mfb_arg(0..2)`
as `r|g<<8|b<<16`. Same three-scalar shape, read not run, in
`src/target/linux_gtk/app_io.rs:emit_app_term_set_color` and
`src/target/win_x86_64/app/mod.rs:emit_term_set_color`. The console
`src/codegen/term/core/term.rs:emit_set_color` was migrated to read the record
pointer in `c_arg(0)`; the three app emitters were not.

## Non-goals

- Alpha: every backend ignores it by design (`mfb man term setForeground`).

## Blast-radius audit

- The `astrings` → `term::drawText` bridge calls both setters per style run, so
  every styled `drawText` in an app build is miscoloured too.
- Any other `term::` member whose signature changed to a record in the same
  commit.

## Fix

Phase 1 — RED test: an `--app` build that sets and reads back both colours on each
app backend that can run headless. Commit:

Phase 2 — load the fields off the record in all three app emitters. Commit:
