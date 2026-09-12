# bug-594: macOS `term::drawText` does not advance the column for a control character

Last updated: 2026-09-12
Effort: small
Severity: MEDIUM — shipped app-mode rendering disagreed with the documented contract,
so the same program laid out differently on macOS than on the console, Linux GTK or
Windows
Class: Correctness / app-mode term backend parity

Status: **FIXED** in `cbf49ff14` (fix, pin, spec) and `67026a3a8` (the three macOS app
goldens) on branch `bug-594-macos-drawtext-control-column` (based on
`integ-576-590-575`). Proven at the emitted-code level only — see "What the instrument
proves, and where it stops". Merging this archive onto main must also remove main's
open `bugs/bug-594-macos-drawtext-does-not-advance-the-column-for-a-control-character.md`.

Regression Test: `tests/cli/cli_macos_app_term_draw_text.rs` —
`macos_app_draw_text_control_character_advances_one_column_without_stamping` (RED on
the base), plus the positive pins
`macos_app_draw_text_printable_path_still_clips_and_advances_by_display_width` and
`macos_app_write_string_keeps_its_own_walk`.

Found by the bug-540 WIN-04 agent while making Windows `drawText` match `io::write`
(recorded on its unlanded branch `bug-540-win04`, `c6e4113dc`).

## The contract

- `mfb man term drawText`: "Control characters (below U+0020, including newline and
  tab) are skipped — **they advance one column but stamp nothing**".
- `src/docs/spec/app/04_term-backend.md`, GTK section: "control bytes **taking a
  column** without stamping".
- The console backend (`src/codegen/term/core/term.rs`, the `skip_ctrl` branch:
  "Control characters (< 0x20): skip stamping, advance one column + byte") and, after
  bug-540 WIN-04, the Windows app backend.

Four sources against one implementation: the macOS code was the wrong side.

## The code (confirmed by re-reading before any change)

`src/target/macos_aarch64/app/term_view.rs`, `emit_term_draw_text_helper` (the IMP for
`TermView mfbDrawText:`, symbol `_mfb_macapp_term_drawText`) branched a control
character straight to `dt_advance_i`, which advances the UTF-16 index and loops. The
only write to the running column is `add col, col, width` at `dt_after_stamp`, which
that branch skipped. Emitted (base, `macos-app-mode-io`):

```
{'op': 'cmp_imm', 'lhs': 'x19', 'rhs': '32'}
{'op': 'b.lt', 'target': 'dt_advance_i'}
...
{'op': 'label', 'name': 'dt_after_stamp'}
{'op': 'ldr_u64', 'dst': 'x16', 'base': 'sp', 'offset': '104'}
{'op': 'add', 'dst': 'x27', 'lhs': 'x27', 'rhs': 'x16'}
{'op': 'label', 'name': 'dt_advance_i'}
```

So `drawText(row, 0, "a\tb")` stamped `b` in column 1 on macOS and column 2 elsewhere.

## The fix

The control branch now goes to `dt_control`, which stores width 1 into the width slot
and joins the stamp path's own column advance at `dt_after_stamp`:

```rust
asm.push(abi::branch_lt("dt_control"));
...
asm.push(abi::label("dt_control"));
asm.push(abi::move_immediate(width, "Integer", "1"));
asm.push(abi::store_u64(width, abi::stack_pointer(), off_w));
asm.push(abi::branch("dt_after_stamp"));
```

The column moves by the SAME instruction a width-1 stamp uses, so the two cannot drift.
Nothing is stamped (no store outside the frame, no call). The right-edge clip is
unaffected: a control character at or past the edge moves the column further past it,
and the next printable cluster still ends the run at the loop's clip.

`emit_app_draw_glyph` (`app_io.rs`) was NOT touched: `drawGlyph` draws one glyph at an
explicit column and has no running column to advance.

## The instrument

The `NSView` cannot be read back headlessly, and a test-only readback hook is the same
open product decision bug-540 WIN-02/03 is blocked on. So the pin reads the emitted code
plan (`mfb build -app -target macos-aarch64 -ncode`) for two programs (a `term::` build,
which embeds the width table, and a plain app, which does not). It derives the loop, the
UTF-16 index register, the column register (the first right-edge `b.ge exit` after the
loop top) and the single `add col, col, width` from the body's structure, then executes
the control branch from its `b.lt` target to the loop back-edge and asserts it:

1. reaches the stamp path's column advance;
2. with the width register holding exactly 1 (constant-tracked through the stack slot);
3. performs no store outside the frame and no call (stamps nothing);
4. still advances the UTF-16 index.

RED on the base, on the behavioural assertion (`cargo test --release --no-fail-fast
--test cli_macos_app_term_draw_text`, exit 101):

```
macos_dt_ctrl_term: a control character in term::drawText must advance the running
column (x27) ... but the control branch (`b.lt dt_advance_i`) never reaches the column
advance at instruction 286 ({"dst":"x27","lhs":"x27","op":"add","rhs":"x16"}).
Control path: [287, 288, 289]
```

(A first base run failed on a PREMISE assertion instead — it wrongly required every
right-edge exit test to use one register, while the stamp's wide-at-edge and wide-pair
tests compare scratch registers. That run was not counted as RED; the premise was
corrected and the pin re-run.) The two positive pins passed on the base.

Positive pins (pass before and after):

- the printable path still ends the run at the right edge before its first stamp,
  still drops a wide cluster whose trailing cell is off the grid
  (`cmp_imm w,#2; b.ne; add_imm t,col,#1; cmp t,cols; b.ge exit`), still advances the
  column after the stamp from the width slot, and takes that width from the utf8proc
  table exactly when the program uses `term::`;
- `mfbWriteString:` (`io::write`'s grid walk) shares no label with the drawText IMP and
  keeps its own `'\n'` test.

Measured containment, function by function (`/tmp` dumps of the three macOS app
fixtures, base binary vs fixed binary, JSON compared per symbol): in each of
`macos-app-mode-io` (52 functions), `macos-app-mode-plumbing` (37) and
`macos-app-mode-term` (117) exactly ONE function changed, `_mfb_macapp_term_drawText`:
one `b.lt` retargeted and four instructions added; its relocations are equal;
`_mfb_macapp_term_writeString` and every other function and data object are
byte-identical. The same `term` fixture built for `linux-x86_64`, `linux-aarch64` and
`windows-x86_64` is identical in all 122 / 122 / 102 functions.

### What the instrument proves, and where it stops

The pin proves which instructions the emitted control branch runs. The artifact gate
hashes the pre-link code plan. Neither runs an app binary, so nothing here proves the
pixels an `NSView` presents.

## Linux GTK — already correct, no change

`src/target/linux_gtk/term_draw.rs`, `emit_term_write_helper` under
`TermWriteMode::DrawText` (symbol `_mfb_gtkapp_term_draw_text`): `cmp #32; b.lt
tw_control`, and `tw_control` adds one to the column, resets the last-base cell, and
falls into `tw_next` (index advance) with no stamp. Emitted by the base binary for the
`macos-app-mode-term` fixture:

`linux-aarch64`:
```
{'op': 'cmp_imm', 'lhs': 'x10', 'rhs': '32'} {'op': 'b.lt', 'target': 'tw_control'}
{'op': 'label', 'name': 'tw_control'}
{'op': 'mov_imm', 'dst': 'x19', 'type': 'Integer', 'value': '1'}
{'op': 'add_imm', 'dst': 'x26', 'src': 'x26', 'imm': '1'}
{'op': 'mov_imm', 'dst': 'x9', 'type': 'Integer', 'value': '0'}
{'op': 'mvn', 'dst': 'x9', 'src': 'x9'}
{'op': 'str_u64', 'src': 'x9', 'base': 'sp', 'offset': '120'}
{'op': 'label', 'name': 'tw_next'}
{'op': 'add', 'dst': 'x21', 'lhs': 'x21', 'rhs': 'x19'}
{'op': 'b', 'target': 'tw_loop'}
```

`linux-x86_64` (column spilled to `rsp+248`):
```
{'op': 'label', 'name': 'tw_control'}
{'op': 'mov_imm', 'dst': 'rbp', 'type': 'Integer', 'value': '1'}
{'op': 'ldr_u64', 'dst': 'r14', 'base': 'rsp', 'offset': '248'}
{'op': 'add_imm', 'dst': 'r14', 'src': 'r14', 'imm': '1'}
{'op': 'str_u64', 'src': 'r14', 'base': 'rsp', 'offset': '248'}
{'op': 'mov_imm', 'dst': 'r12', 'type': 'Integer', 'value': '0'}
{'op': 'mvn', 'dst': 'r12', 'src': 'r12'}
{'op': 'str_u64', 'src': 'r12', 'base': 'rsp', 'offset': '120'}
{'op': 'label', 'name': 'tw_next'}
{'op': 'add', 'dst': 'r11', 'lhs': 'r11', 'rhs': 'rbp'}
```

With this fix, console, Linux GTK, macOS and (after bug-540 WIN-04) Windows all advance
one column for a control character.

## Docs

The man page and the GTK spec paragraph were already right. The macOS spec paragraph
(`### mfbDrawGlyph: and mfbDrawText:`) said the IMP "iterates `characterAtIndex:`,
stamping one cell per unit ... skipping control characters", which was stale on two
counts (the walk is cluster/width-aware since plan-70-D) and silent on the column; it
now states that a control character takes one column and stamps nothing.

## Gate

Before any change, the base binary's dumps matched the committed goldens (`cmp` exit 0
for the `io` and `plumbing` `.ncode`; all four `macos-app-mode-term` `.ncodesum` values
equal).

`bash scripts/regen-outside-ncode.sh target/release/mfb` (exit 0): 17 goldens refreshed,
exactly **3 moved** — `macos-app-mode-io` `macos-aarch64.app.ncode`,
`macos-app-mode-plumbing` `macos-aarch64.app.ncode`, `macos-app-mode-term`
`macos-aarch64.app.ncodesum`. Each equals the fixed-binary dump that was diffed
function by function above. The `term` fixture's `windows-x86_64`, `linux-x86_64` and
`linux-aarch64` `.app.ncodesum` did not move. No `.run` golden moved.

`bash scripts/artifact-gate.sh target/release/mfb all` on the fixed release binary, after
the regeneration: `1431 tests, 1597 build(s), 2009 golden(s) checked, 0 diff(s)`, exit 0.

Exit codes observed: base release build 0; first base pin run 101 (premise; not counted);
second base pin run 101 (RED on the behaviour); fixed release build 0; fixed pin run 0
(3 passed); regen 0; full gate 0; rustfmt `--check` on both touched Rust files 0.
