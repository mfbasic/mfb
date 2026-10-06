# bug-973: in an `--app` window, `io::readKey` right after `io::readLine`/`io::input` (or a canvas `readLine`) first returns the Return key's `Release`, a release whose press was never reported

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (UNCLEAR — needs owner decision)

Status: Open
Regression Test: (proposed) macOS GUI test posting Return-down, Return-up, `a` after a `readLine`, asserting the first `readKey` is `a` Press

**Owner decision (2026-10-05):** suppress it. Every app backend tracks pressed keys (as GTK already does with
`_mfb_gtkapp_keys_held`) and drops a key Release whose Press was consumed by a line read
(`io::readLine` / `io::input`) or was never delivered as a key event. The first `readKey` after
a line read returns the next real key event. Fixture: Return-down, Return-up, `a` after a
`readLine`. The first `readKey` is `a` Press.

`io::readLine` commits on the Return key-*down* (line mode). The program then calls
`io::readKey`, which switches the input mode to raw; the Return key-*up* arrives after
that, and the transcript's `keyUp:` "reports the release in raw mode only" — so it is
written to the pipe and becomes the first key event: `Enter`, `Release`. On the canvas
view releases are always reported, so the same happens there after a `readLine`.
With real typing, key-up follows key-down by ~100 ms, so a program that reads a line
and then waits for a key practically always sees this unpaired release first.

The behavior matches the spec text literally (release reported in raw mode), but the
event is spurious from the program's point of view (no matching Press was ever
delivered). Owner decision: suppress releases for keys whose press was consumed by a
line read (e.g. track pressed keys as GTK already does with `_mfb_gtkapp_keys_held`),
or document it.

References:

- `src/docs/spec/app/03_console-io.md` "Key events in the kitty form" and "Mode
  toggles".

## Failing Reproduction

`/tmp/hardening/B17/ki` (input, readLine, 6× readKey, input) driven with posted key
events (`/tmp/hardening/B17/keypost`, CGEventPostToPid):

```
mfb build --app ki; ki/build/p1.app/Contents/MacOS/p1 & P=$!; sleep 1.5
./keypost $P t:abcX k:51 t:d k:36 s:200 t:line2 k:36 s:200 k:12 k:123 k:0:m
cat ki.log
```

- Observed: `input=[abcd]`, `readLine=[line2]`, then `key Enter|\n|Release|...`, then
  `q` Press, `q` Release, `Left` Press/Release, Cmd-`a` Press.
- Canvas (`/tmp/hardening/B17/ck`, `setMode(Canvas)`, readLine, readKey; posted
  `hi`, Return, `a`): `readLine len=2 bytes=6869`, then `key Enter|\n|Release`.
- Expected (proposed): first `readKey` is `q` (resp. `a`) Press.

## Root Cause

`src/target/macos_aarch64/app/keys.rs:emit_key_up_helper` checks only the current
input mode, not whether the key's press was delivered as a raw event.

## Blast Radius

- GTK: line-mode `key-released` "only marks the key up" (spec), so a press consumed in
  line mode should not produce a raw release — not measured.
