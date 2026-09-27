# tools/example-eof-probe

Regression probe for bug-715: `hello_input`, `wide-demo`, `hangman`, and
`audio` must not crash with the compiler's raw `Error: 7-702-0003` text and
exit 255 when stdin ends before a read is satisfied — and the two
alternate-screen apps (`wide-demo`, `hangman`) must restore the terminal
*before* reporting, so the message is visible on a real terminal.

## Usage

```
python3 tools/example-eof-probe/probe.py
```

Builds each example via `target/release/mfb build` (the caller must have
already built the compiler, `cargo build --release --bin mfb`), runs it with
piped/redirected stdin engineered to run out mid-program, and asserts:

1. Exit code is not 255.
2. The output never contains the raw `Error: 7-702-0003` text.
3. Output contains an example-authored message (e.g. `"hello_input: ..."`).
4. For `wide-demo`/`hangman`: the terminal-restore escape sequence
   (`\x1b[?1049l`) appears before that message in the byte stream.

Exits 0 with PASS lines on success; exits 1 with a diagnostic on the first
failing case. This is a reusable check across every example with an unguarded
read call, not a one-off — new cases can be appended to the `cases` list in
`probe.py`.
