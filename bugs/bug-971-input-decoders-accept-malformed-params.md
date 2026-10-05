# bug-971: the SGR mouse decoder and the key decoder accept malformed/overflowing numeric parameters — 2-field and 0-coordinate mouse reports become events at row/column -1, an oversized modifier parameter reports every modifier

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Coordinator note: measured by the review agent only (Linux runs in podman, or a macOS window screenshot); the coordinator did not re-run it.
Regression Test: (proposed) pty tests next to the mouse/key decoder suites feeding the sequences below

Two console-input decoders turn malformed sequences into plausible-looking events
instead of treating them as "not a recognised report":

1. **SGR mouse** (`term::enableMouse(TRUE)`): `ESC [ < 0 ; 5 M` (only two fields) is
   delivered as `Down Left r-1 c4`; `ESC [ < 0 ; 0 ; 0 M` (SGR is 1-based; 0 is not a
   valid coordinate) as `Down Left r-1 c-1`. A program handing `row`/`column` "straight
   to `term::moveTo` or `term::drawText`" (as `mfb man term pollMouse` advises) gets a
   negative cell.
2. **Key decoder** (`io::readKey`): a modifier parameter past the clamp,
   `ESC [ 1 ; 999999999999999999999 A` or `ESC [ 1 ; 18446744073709551617 A`, is
   reported as `Up` with **shift, ctrl, alt and meta all TRUE** (the parameter is clamped
   to `PARAM_MAX` = 1 000 000 000, whose low bits are all set). An oversized *key*
   number (`ESC [ 99999999999999999999999 u`) is, by contrast, dropped.

The single correct behavior (proposed): a report missing a field, with a zero
coordinate, or with a parameter that overflowed its clamp is not a recognised report —
dropped (or passed through) rather than synthesized into an event.

References:

- `src/docs/spec/app/04_term-backend.md` "The mouse decoder and the read path":
  "Bytes that are not a recognised report reach the program unchanged"; coordinates
  are zero-based cells.
- `src/docs/spec/stdlib/22_io-input.md` "The decoder" (silent on overflow).

## Failing Reproduction

Mouse: `/tmp/hardening/B17/mo1` (term::on, enableMouse, loop `readKey(30)` + drain
`pollMouse`, log, quit on `q`):

```
mfb build mo1
python3 /tmp/hardening/B17/pty_run.py mo1/build/p1.out 10 40 \
  '[["send","\u001b[?62;c"],["sleep",0.1],["send","\u001b[<0;0;0M"],["send","\u001b[<0;5M"],["send","q"]]'
```

- Observed: `mouse Down Left r-1 c-1 ...`, `mouse Down Left r-1 c4 ...`.
- Expected: no mouse events for either.

Keys: `/tmp/hardening/B17/k1` (term::on, readKey loop) with the terminal answering the
kitty probe:

```
python3 /tmp/hardening/B17/pty_run.py k1/build/p1.out 10 40 \
  '[["send","\u001b[?0u\u001b[?62;c"],["sleep",0.1],["send","\u001b[1;999999999999999999999A"],["send","\u001b[113u"]]'
```

- Observed: `Up||Press|sTRUEcTRUEaTRUEmTRUE`.
- Expected: dropped, or `Up` with no modifiers — not every modifier.

Contrast (correct): `ESC [ < 0 ; 5 ; 3 M` → `Down Left r2 c4`; `ESC [ 1 ; 5 : 1 C` →
`Right` with ctrl; `ESC [ 1 ; 65 A` → `Up`, no modifiers (caps-lock bit ignored).

## Root Cause

- `src/codegen/io/mouse/decode.rs:emit_parse_and_enqueue` parses three decimals with
  `emit_parse_decimal`, which never fails: an absent field yields 0, then
  `subtract_immediate(x, x, 1)` makes it -1. No field-count or `>= 1` check.
- `src/codegen/io/input/decode.rs`: parameters clamp at `PARAM_MAX`; the clamped
  modifier then goes through `emit_modifier_bits` as if valid.

## Goal

- Malformed mouse reports and overflowed parameters produce no event.

## Blast Radius

- App backends inject well-formed reports (their own encoder), so only console input
  (a terminal, a pipe, `MFB_MOUSE_INJECT`) can reach these paths.
