# bug-964: GTK app `term::` treats every width-2 glyph as "at the right edge" — `io::write` wraps it to the next row, `term::drawText` drops it and the rest of the run

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Coordinator note: measured by the review agent only (Linux runs in podman, or a macOS window screenshot); the coordinator did not re-run it.
Regression Test: (proposed) a GTK app-mode grid test next to `tests/runtime/rt_gtk_term_utf8_grid.rs` asserting that `io::write("a漢b")` stamps row 0 as `a`,`漢`,TRAIL,`b` and `term::drawText(0,0,"c漢d")` stamps `c`,`漢`,TRAIL,`d`

On the Linux GTK app backend (linux-aarch64 and linux-x86_64, both measured), any
width-2 cluster (CJK, emoji) written while TUI mode is on is mis-placed regardless of
where the cursor is:

* `io::write` / `io::print` wrap the wide glyph to the start of the **next row**, as if
  it straddled the right edge, even at column 3 of a 69-column grid.
* `term::drawText` treats it as "only one column left": the wide cluster **and every
  cluster after it** are dropped (the run ends).

The single correct behavior: a width-2 cluster wraps (write) / ends the run
(drawText) only when `column + 1 >= cols`; anywhere else it is stamped in place with
its `GTK_WIDE_TRAIL` sentinel and the walk continues — which is what the macOS
backend does with the same program.

References:

- `src/docs/spec/app/04_term-backend.md` "Retained double-buffered surface" — "a
  double-width cluster taking two columns and being dropped rather than split when
  only one column remains"; "A wide (width-2) grapheme ... wraps to the next row
  rather than straddling the right edge"; Linux "Unicode-wide clusters (plan-70-E)".
- Found by hardening batch B17 (app spec review).

## Failing Reproduction

`/tmp/hardening/B17/gt2/src/main.mfb` (trimmed):

```
IMPORT io
IMPORT term
IMPORT os

FUNC main() AS Nothing
  term::on()
  term::drawText(0, 0, "A1 plain")
  term::drawText(1, 0, "A2 a漢b")
  term::moveTo(7, 0)
  io::write("W1 a漢b\n")
  term::sync()
  os::sleep(8000)
  term::off()
END FUNC
```

```
mfb build --app-debug --target linux-aarch64 gt2      # also --target linux-x86_64
# in podman (debian trixie + libgtk-4-1 + Xvfb), DISPLAY=:9:
./p1-glibc.AppDir/AppRun &  sleep 5; import -window root shot.png
```

- Observed (screenshot `/tmp/hardening/B17/gt2_z.png`, x86-64 `/tmp/hardening/B17/gt264_z.png`):
  row 1 shows `A2 a` (the `漢` and the `b` are gone); row 7 shows `W1 a`, row 8 shows
  `漢 b` (wrapped although ~65 columns remained). Same for `👍`/`👍🏽`.
- Expected: row 1 `A2 a漢b`; row 7 `W1 a漢b`. The macOS `--app` build of the same
  program renders exactly that (`/tmp/hardening/B17/at1b_s.png`).

Contrast: ASCII, precomposed `é` and `e`+U+0301 (folded into the pool) render
correctly on GTK; only the width-2 branch is wrong.

| Environment | Result |
| --- | --- |
| linux-aarch64 GTK4 / Xvfb (podman) | fails ✗ |
| linux-x86_64 GTK4 / Xvfb (podman, qemu) | fails ✗ |
| macOS aarch64 `--app` | works ✓ |

## Root Cause

`src/target/linux_gtk/term_draw.rs` (the shared writer / drawText cluster walk,
`emit_term_write_helper` / `TermWriteMode`), wide-at-edge check:

```rust
asm.push(abi::add_immediate(abi::SCRATCH[0], abi::LOCAL[7], 1));   // col + 1
asm.load_state(abi::SCRATCH[2], ST_TERM_COLS);                      // clobbers SCRATCH[0]!
asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[2]));
asm.push(abi::branch_lt("tw_edge_ok"));
```

`Asm::load_state` (`src/target/linux_gtk/mod.rs`) materializes the state address in
`abi::SCRATCH[0]` before the load, so the compare tests `&_mfb_gtkapp_state` (a large
address) against `cols`; `branch_lt` is never taken and every width-2 glyph takes the
edge path. A scan of `src/target/linux_gtk/*.rs` for "write SCRATCH[0], then
`load_state`/`store_state`, then read SCRATCH[0]" finds only this site.

## Goal

- `io::write` / `term::drawText` of `a漢b` on GTK stamp the wide glyph in place at
  column 1 with its trail at column 2; wrap / drop happens only at the true right edge.

### Non-goals (must NOT change)

- The wrap-at-edge and drop-at-edge behavior at the real last column.
- macOS / Windows / console backends.

## Blast Radius

- `term_draw.rs` wide-at-edge check — both `TermWriteMode::Write` and `DrawText`
  specializations (one emitter) — fixed by this bug.
- Other `load_state` users: scanned, none reuse `SCRATCH[0]` across the call.
