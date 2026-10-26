The only banned-word hit is the derived ErrOutOfMemory message, which the standard carves out. I have everything needed. Findings follow.

### 1. Meta is reported on a plain terminal
UNIT:      man-page:io/readKey
CLAIM:     "any key with Meta | the key; meta is never set" (the last row of the plain-terminal table)
VERDICT:   wrong
EVIDENCE:  The decoder maps the xterm/kitty modifier bit 8 to `meta` (`src/codegen/io/input/decode/csi.rs`, `d.imm(&m, MOD_META)`; corpus case "Meta+Down (xterm meta / kitty super)" `b"\x1b[1;9B"` → `Down, MOD_META` in `src/codegen/io/input/corpus.rs`). Probe, piped into the dump program at /tmp/plan-125-scratch/D-phase6-current/man-page-io-readKey/dump.mfb:
```
printf '\x1b[1;9B' | ./p_dump/build/proj.out
Down g=0: ev=NotSupported s=FALSE c=FALSE a=FALSE m=TRUE
```
An xterm-family terminal sends exactly that sequence for Meta+Down, so a plain terminal does set `meta` for Meta with an arrow, Home, End, Page Up/Down, Insert, Delete or F-key. Only Meta+letter goes unreported.
SUGGESTED: Replace the row with: "Meta+letter | the letter; `meta` is not set", and fold Meta into the arrow/navigation row: "Shift, Ctrl, Alt or Meta with an arrow, Home, End, Page Up/Down, Insert, Delete, F-key | the key, with those modifiers".

### 2. A pipe or file can deliver Release and Repeat
UNIT:      man-page:io/readKey
CLAIM:     "Everywhere else — another terminal, a console outside term::on, a pipe, a file — every key arrives once, as it is typed, with event = io::KeyEvent.NotSupported, and no release ever arrives."
VERDICT:   misleading
EVIDENCE:  The decoder recognises kitty `CSI … u` sequences on every input, not only under `term::on`. Probe with kitty-encoded bytes on a pipe:
```
printf '\x1b[97;5u\x1b[97;2u\x1b[97;9u\x1b[97;1:3u' | ./p_dump/build/proj.out
Unicode g=1:a ev=NotSupported s=FALSE c=TRUE a=FALSE m=FALSE
Unicode g=1:A ev=NotSupported s=TRUE c=FALSE a=FALSE m=FALSE
Unicode g=1:a ev=NotSupported s=FALSE c=FALSE a=FALSE m=TRUE
Unicode g=1:a ev=Release s=FALSE c=FALSE a=FALSE m=FALSE
```
A file of recorded kitty input, or a terminal with the protocol already enabled by something else, yields `Release` (and `shift`/`meta`) with no `term::on` anywhere. A reader who takes "no release ever arrives" literally and drops the `k.event <> Release` guard on a pipe will double-count. The second example on the page keeps the guard, which is the right advice; the prose contradicts it.
SUGGESTED: "Everywhere else — another terminal, a console outside term::on, a pipe, a file — a key arrives once, as it is typed, with event = io::KeyEvent.NotSupported. Press, Repeat and Release only arrive when the input carries kitty-protocol key reports, so always check event rather than assuming it; the second example below does."

### 3. Description never states the ErrWrongMode condition
UNIT:      man-page:io/readKey
CLAIM:     The Description's error coverage ends at "A thread other than the main thread must subscribe with thread::openStdIn before reading, or the call raises ErrInvalidContext."
VERDICT:   incomplete
EVIDENCE:  `ErrWrongMode` is in the descriptor's error list (`src/codegen/builtins/io/func_read_key.rs`, `errors: vec![…"ErrWrongMode"]`), and the lowering takes the app-mode input path (`gen_read_key_family.rs`, `emit_app_raw_input_mode`). The only statement of when it fires is buried in the derived Errors-table message. The standard (§2) requires every raisable error and its condition in the page's own prose. `ErrInputFailed` and `ErrOutOfMemory` are likewise unmentioned, though those are the generic "standard input failed" / "out of memory" cases.
SUGGESTED: Append to the last paragraph: "In an --app build the call needs a window to take key events from, so it raises ErrWrongMode in app::Mode.None. ErrInputFailed means standard input itself failed; ErrOutOfMemory means the program ran out of memory."

### 4. Ctrl with a non-letter key is undocumented
UNIT:      man-page:io/readKey
CLAIM:     "Ctrl+letter | the letter, with ctrl"
VERDICT:   incomplete
EVIDENCE:  The decoder also maps the other C0 bytes a terminal sends for Ctrl combinations (corpus: "Ctrl+Space NUL" → `" "` with ctrl; "Ctrl+\\" → `"\\"`; "Ctrl+_" → `"_"`). Probe:
```
printf '\x00' | ./p_dump/build/proj.out
Unicode g=1:  ev=NotSupported s=FALSE c=TRUE a=FALSE m=FALSE
```
A developer reading the table would expect Ctrl+Space to be invisible or to arrive as a bare NUL; it arrives as a space with ctrl set.
SUGGESTED: "Ctrl+letter, Ctrl+Space, Ctrl+\, Ctrl+] , Ctrl+^, Ctrl+_ | the character, with ctrl"

No findings on the rest. Every other sentence checked out against the binary: Shift+letter arrives as the capital with `shift` clear; Alt+key arrives with `alt`; Enter is `"\n"` for LF, CR and CRLF alike; Backspace is U+007F, Escape U+001B, Tab `"\t"`; F13 (`CSI 25~`) and a lone 0xFF byte are skipped; `readKey(0)` on an open empty pipe raises ErrTimeout and on a closed empty pipe raises ErrEndOfFile; `readKey(-1)` raises ErrInvalidArgument; `readKey(50)` with a lone Escape then a held-open pipe times out as the page says; readKey/readLine/readKey over `a\nhello\nb` gave `a||h` with nothing lost or reordered; an unsubscribed worker thread got code 77050019 and a subscribed one read the key; under a pseudo-terminal a lone Escape was reported after 52 ms wall clock (25 ms timer plus polling), nothing was echoed while waiting, and ECHO/ICANON were back on before the next statement ran. All three page examples compiled from the descriptor's fenced blocks and printed the expected output (`typed a` / `a key with no text`, `2,1` for Right Right Down q, `no key this frame` on an empty open pipe and `key z` with input). Programs and outputs are under /tmp/plan-125-scratch/D-phase6-current/man-page-io-readKey.
