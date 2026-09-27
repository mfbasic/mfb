# tools/pty-probe

A shared, stdlib-only Python helper (`pty_probe.py`) for regression probes that
must drive a `term::`-based console example over a **real** pty — a plain pipe
is not enough for these examples, because `term::on()` puts the tty into
raw/single-key mode, which only exists on an actual pty.

Two probes currently share it:

- `snake_life_advance_under_input.py` (bug-703) — builds `examples/snake` and
  `examples/life`, drives each over a pty with continuous/rapid keystrokes, and
  asserts the simulation (the snake's head position; life's `Gen` counter)
  keeps advancing on its fixed cadence regardless of whether a key arrived that
  tick, plus that two same-tick turns in snake never compose into a reversal.
- `life_quit_no_return.py` (bug-708) — builds `examples/life`, sends a single
  `q` with no trailing Return over a pty, and asserts the process exits 0
  within ~1s, proving the header comment's claim ("keys need Return") is false.

## Usage

Both scripts are self-contained; run with the repository's Python 3 (no
virtualenv or third-party package needed — `pty_probe.py` only uses the
standard library):

```
python3 tools/pty-probe/snake_life_advance_under_input.py
python3 tools/pty-probe/life_quit_no_return.py
```

Each builds the example it needs via `target/release/mfb build examples/<x>`
(the caller must have already built the compiler itself,
`cargo build --release --bin mfb`) and exits non-zero with a diagnostic if an
assertion fails, so either script also works as a CI-style gate, not just a
manual probe.

## Why `pty_probe.py` instead of a screen-emulation library

The examples this module drives always draw by an explicit cursor move (`CSI
row;col H`) immediately followed by the glyph(s)/text for that cell or line —
never by relying on wrap/scroll to place content. `pty_probe.extract_cups`
exploits exactly that: it scans the raw byte stream for every such move and the
literal text written right after it, which is enough to answer "where is the
head glyph now" or "what does the status line say now" across frames without
needing a full VT100 grid model or a third-party dependency like `pyte`.
