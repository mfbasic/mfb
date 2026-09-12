# bug-594: macOS `term::drawText` does not advance the column for a control character

Last updated: 2026-09-12
Effort: small for the fix; the instrument is the work (see Phase 1)
Severity: MEDIUM — shipped app-mode rendering disagrees with the documented contract,
so the same program lays out differently on macOS than on the console or Windows
Class: Correctness / app-mode term backend parity

Status: **Open — confirmed from code against the documented contract; not reproduced
visually**, because the macOS view cannot be read back headlessly.
Regression Test: to be decided in Phase 1 (a codegen-inspection pin or a readback hook).

Found by the bug-540 WIN-04 agent while making Windows `drawText` match `io::write`,
and recorded only in bug-540's doc on its unlanded branch (`bug-540-win04`,
`c6e4113dc`): "the macOS `mfbDrawText:` leaves the column unchanged for a control
character, where the man page and the console advance one column — a macOS
divergence, outside this backend." Filed here so it is not lost when that branch
lands. Defect search before filing (bugs/ and bugs/completed/, for drawText +
control-character + column advance) found nothing else.

## The contract

- `mfb man term drawText`: "Control characters (below U+0020, including newline and
  tab) are skipped — **they advance one column but stamp nothing** — so a stray control
  character can never corrupt the presented frame."
- `src/docs/spec/app/04_term-backend.md` (~line 674): "control bytes **taking a column**
  without stamping".
- The console backend and, after bug-540 WIN-04, the Windows app backend both advance
  one column.

## The code

`src/target/macos_aarch64/app/term_view.rs`, `emit_term_draw_text_helper` — the IMP
for `TermView mfbDrawText:`:

```rust
// Control char (< 0x20): advance i by L, leave the column unchanged.
asm.push(abi::compare_immediate(cp, "32"));
asm.push(abi::branch_lt("dt_advance_i"));
```

It moves the UTF-16 index `i` past the control character and does **not** advance
`col`. So `drawText(row, 0, "a\tb")` stamps `b` in column 1 on macOS and in column 2
on the console and on Windows.

**Docs vs code — which side is wrong:** four sources state "advance one column"
(man page, spec, console, Windows after WIN-04) against one implementation that does
not. The macOS code is the wrong side. The in-code comment describes what it does, not
what the contract requires.

Not the same as `emit_app_draw_glyph` (`src/target/macos_aarch64/app/app_io.rs`,
~line 1311), which returns early for a control code point. `drawGlyph` takes an
explicit column and draws one glyph, so there is no running column to advance there.
Do not "fix" that path.

## Phase 1 — instrument first

The macOS `NSView` cannot be read back headlessly, which is also why bug-540's Windows
GDI proof is weak. Before changing code, choose one:

1. a **codegen-inspection pin** in the style of bug-540 WIN-04's
   `tests/cli/cli_win_app_term_fidelity.rs` (which compares drawText's walk to
   `io::write`'s instruction-for-instruction): assert macOS `mfbDrawText:`'s
   control-character branch advances `col` exactly as the stamp path does; or
2. a test-only readback of the cell grid. That adds product surface, which is the same
   open decision bug-540 WIN-02/03 is blocked on — so prefer (1).

## Goal

A control character in `term::drawText` advances the column by one and stamps nothing
on macOS, matching the man page, the spec, the console, and Windows.

### Non-goals

- Do not change `drawGlyph`.
- Do not change the documented contract to match macOS; four sources agree.

## Also check (Linux)

bug-539 put the Linux GTK app backend under byte-identity coverage. Whether its
`drawText` advances the column for a control character has not been checked here —
confirm it in Phase 1 so the fix closes parity for all three app backends, not two.
