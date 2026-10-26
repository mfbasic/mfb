### 1. Example 1 prints a decimal number after `U+`
UNIT:      man-page:io/readScalar
CLAIM:     "Read one character and show its code point:" with `io::print(toString(c) & " is U+" & toString(toInt(c)))`
VERDICT:   example-broken
EVIDENCE:  Compiled as `p_ex1` and ran `printf 'A' | p_ex1/build/p_ex1.out` → `A is U+65`. `U+` notation is hexadecimal by convention (A is U+0041); the example prints the decimal value with a `U+` prefix, so its output is wrong as a code point and will confuse a reader who looks up "U+65" (which is `e`).
SUGGESTED: Either drop the `U+` prefix (`io::print(toString(c) & " is code point " & toString(toInt(c)))`) or format in hex if a hex formatter exists; and change the lead-in to "Read one character and show its code point as a number:".

### 2. Ctrl+H and Ctrl+M do not arrive as U+0008 and U+000D
UNIT:      man-page:io/readScalar
CLAIM:     "Ctrl+A through Ctrl+Z as U+0001 through U+001A."
VERDICT:   wrong
EVIDENCE:  `printf '\x08Z' | p_multi/build/p_multi.out` → `U+127` then `U+90` (Ctrl+H, byte 0x08, arrives as U+007F). `printf '\rZ' | p_multi.out` → `U+10` then `U+90` (Ctrl+M, byte 0x0D, arrives as U+000A). Source: `src/codegen/io/input/decode/mod.rs:437` excludes `0x08 | 0x09 | 0x0A | 0x0D | 0x1B` from the plain-control path and maps them to the named keys Backspace/Tab/Enter/Escape, whose text is given in the preceding sentence. Ctrl+I and Ctrl+J happen to coincide (U+0009, U+000A), but H and M do not.
SUGGESTED: "The other Ctrl+letter keys arrive as U+0001 through U+001A, except that Ctrl+H is Backspace (U+007F) and Ctrl+M is Enter (U+000A), as listed above."

### 3. In piped data an Escape byte followed by more bytes is never returned
UNIT:      man-page:io/readScalar
CLAIM:     "Escape as U+001B"
VERDICT:   incomplete
EVIDENCE:  `printf '\x1bZ' | p_multi.out` → `U+90` only; `printf '\x1baZ' | p_multi.out` → `U+97`, `U+90`; `(printf '\x1b'; sleep 0.5; printf 'Z') | p_multi.out` → `U+90` only; `printf '\x1b\x1bZ' | p_multi.out` → `U+90` only. Only `printf 'Z\x1b'` (Escape as the last byte of input) yields `U+27`. Per `src/codegen/io/input/reader.rs:30-37`, a lone ESC is decided after a 25 ms delay on a keyboard but waits "for as long as it takes" on a pipe or file, and `decode/mod.rs:310` treats ESC + any byte other than `[`/`O` as an Alt prefix, which `readScalar` drops along with the modifier. The page says the function reads "typed or piped" input and promises Escape as U+001B without saying this holds only on a keyboard; for piped data the Escape byte vanishes unless it is the final byte.
SUGGESTED: Add after the Escape sentence: "Escape is recognised on a keyboard, where a lone Escape is decided after a short pause. In piped or redirected input an ESC byte followed by another byte is read as that key with Alt held, and only the key's text is returned; a modifier alone (Alt, Ctrl, Shift) is never reported."

### 4. Two errors in the table have no condition in the description
UNIT:      man-page:io/readScalar
CLAIM:     Errors table rows `77020005 ErrInputFailed` and `77050020 ErrWrongMode`
VERDICT:   incomplete
EVIDENCE:  Descriptor `src/codegen/builtins/io/func_read_scalar.rs:register` declares `ErrInputFailed` and `ErrWrongMode`; the lowering raises `ErrInputFailed` from `input_error` (terminal configure failure, read failure, or the unread-buffer guard in `gen_read_key_family.rs:emit_take_scalar`) and app-mode `emit_app_raw_input_mode` traps `ErrWrongMode` in `app::Mode.None`. The description names the condition for `ErrTimeout`, `ErrInvalidArgument`, `ErrEndOfFile`, `ErrInvalidContext` and `ErrOutOfMemory` but never mentions the other two. `.ai/man-content.md` §2 requires "Every raisable error and the condition that raises it".
SUGGESTED: Append to the last paragraph: "`ErrInputFailed` means standard input could not be read or the terminal could not be switched to single-key mode. In an `--app` build the program must be in `app::Mode.Console` or `app::Mode.Canvas`, where a window supplies key events; in `app::Mode.None` the call raises `ErrWrongMode`."

### 5. Terminal-mode claims could not be checked in this sandbox
UNIT:      man-page:io/readScalar
CLAIM:     "on a terminal each key is read as it is pressed, with nothing echoed, and the previous mode is restored before the call returns."
VERDICT:   UNVERIFIED
EVIDENCE:  Tried `printf 'A' | script -q /dev/null p_ex1/build/p_ex1.out`; output was `^D^H^HA^D is U+4^M`, meaning the pty delivered the `^D` that `script` writes at end of input rather than the `A` that was still in the canonical line buffer. That is a `script` timing artifact, not evidence about the claim. The lowering does configure and restore the terminal (`gen_read_key_family.rs:lower_key_reader` calls `emit_configure_stdin_terminal` with echo and canonical mode cleared and `emit_restore_stdin_terminal` at `done`), so the claim is consistent with the source, but no interactive run confirmed it. Needs a real terminal.
SUGGESTED: No wording change; main thread should run example 1 interactively and type one key.

Everything else on the page checked out against probes: `printf 'e\xcc\x81Z'` gave `U+101`, `U+769`, `U+90` (one scalar per call); the ZWJ emoji sequence gave three scalars; `printf '\xff\xfeAB'` gave `U+65`, `U+66` (invalid bytes skipped); `\x1b[A`, `\x1b[3~` and `\x1bOP` were skipped; `\r\n` gave one `U+10`; `readScalar(-1)` gave 77050002; `readScalar(0)` on an open empty pipe gave 77050008 and on `/dev/null` gave 77020003; example 2 with a 2 s silent pipe printed `no key`; a `readScalar`/`readLine`/`readScalar` mix on `ab\ncd\ne` gave `U+97`, `[b]`, `U+99`; a worker thread without `thread::openStdIn` got 77050019 and with it got 65.
