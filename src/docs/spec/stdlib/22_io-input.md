# io Input: Keys, Text and Lines

The model behind the `io::` readers — `readRaw`, `readScalar`, `readGrapheme`,
`readKey`, `readLine` and `input`: where their bytes come from, how a byte stream
becomes keys, how long a read waits, and how a terminal or an `--app` window is
brought to deliver every key in one form. The per-function API (signatures,
parameters, errors, examples) is `./mfb man io`; the per-thread standard-input
log the readers sit on is `./mfb spec threading os-integration`; the `--app`
window pipe is `./mfb spec app console-io`.

## Sources: a keyboard or data

Standard input is one of two kinds of source, and the readers treat them
differently:

- **A keyboard** — an `--app` window (always), or a console whose standard input
  is a terminal. Bytes arrive as keys are pressed, and a pause between two bytes
  means something (a lone Escape, not the start of a sequence).
- **Data** — a console whose standard input is a pipe or a file. The bytes are
  already written; a pause means only that the writer is slow.

A console reader asks `isatty(0)` (`GetConsoleMode` on Windows) the first time it
runs on a thread and caches the answer in the input-buffer region's source word;
an `--app` build is a keyboard without asking.
[[src/codegen/io/input/reader.rs:emit_reader_setup]]
[[src/codegen/io/input/mod.rs:SOURCE_KEYBOARD]]
[[src/codegen/io/input/mod.rs:SOURCE_DATA]]

## The input buffer

Every reader reads through a per-thread **input buffer**, an arena-state region
reserved only in a program that calls one of the readers
(`INPUT_BUFFER_READERS`) or polls the mouse (`program_reads_mouse`: the terminal
delivers mouse reports on standard input). Its layout:

| Offset | Field |
|---|---|
| 0 | length of the buffered bytes |
| 8 | pending — when the front of the buffer started waiting to be classified (monotonic ns), or 0 |
| 16 | source — 0 unknown, 1 keyboard, 2 data |
| 24 | the decoder's token record (192 bytes) |
| 224 | the buffered bytes, oldest first, up to 4096 |

[[src/codegen/io/input/mod.rs:INPUT_STATE_LEN_OFFSET]]
[[src/codegen/io/input/mod.rs:INPUT_STATE_PENDING_OFFSET]]
[[src/codegen/io/input/mod.rs:INPUT_STATE_SOURCE_OFFSET]]
[[src/codegen/io/input/mod.rs:INPUT_STATE_TOKEN_OFFSET]]
[[src/codegen/io/input/mod.rs:INPUT_STATE_BUF_OFFSET]]
[[src/codegen/io/input/mod.rs:INPUT_BUFFER_CAPACITY]]
[[src/codegen/io/input/mod.rs:INPUT_BUFFER_READERS]]

Bytes enter the buffer only from the source read (below) and leave it only when a
reader consumes them, so every reader takes buffered bytes before new ones and
mixing readers never loses or reorders input. Two operations put bytes back at
the **front**: a timed-out line read returning its half-read line, and
`readScalar` returning the rest of a key that typed several scalars.
[[src/codegen/io/input/buffer.rs:emit_consume]]
[[src/codegen/io/input/buffer.rs:emit_unread]]

## The decoder

`_mfb_rt_input_decode` is a pure function of the buffer, its length and three
flags — `FINAL` (no more input is coming for this decision), `EVENTS` (the source
reports press/repeat/release, so a sequence with no event field is a press rather
than `NotSupported`), `CLUSTER` (return a grapheme cluster, not one scalar) — that
writes one token into the region's record: **need more**, **skip `n` bytes**, or
**a key** with its `KeyName`, `KeyEvent`, modifier bits, grapheme and the text a
text reader sees. It accepts UTF-8 (an ill-formed maximal subpart is a skip), the
C0 keys, the legacy xterm/VT220/rxvt/Linux-console sequences with xterm modifier
parameters and `modifyOtherKeys`, and kitty `CSI … u` with event types,
modifiers and associated text. It never waits. The grapheme-cluster code (and so
the Unicode property tables) is emitted only when a program calls
`readGrapheme` or `readKey`.
[[src/codegen/io/input/decode/mod.rs:lower_input_decode]]
[[src/codegen/io/input/decode/mod.rs:FLAG_EVENTS]]
[[src/codegen/io/input/keys.rs:KeyName]]
[[src/codegen/io/input/keys.rs:KeyEvent]]
[[src/codegen/io/input/mod.rs:CLUSTER_READERS]]

## The read engine

Each reader body emits one copy of the read engine and jumps into it for every
unit it needs. One pass yields a token (token mode), a dropped run of ill-formed
UTF-8, or one raw byte (byte mode, `readRaw`), or leaves for the caller's
timeout / end-of-input / error label.
[[src/codegen/io/input/reader.rs:emit_engine]]
[[src/codegen/io/input/reader.rs:ENGINE_TOKEN]]
[[src/codegen/io/input/reader.rs:ENGINE_BYTE]]

- **Buffer first.** The decoder runs on what is buffered. Only when it answers
  "need more" (or the buffer is empty) is the source read — one byte, through the
  shared byte read (the broadcast log in a console build, the window pipe in an
  `--app` build, behind the mouse decoder when the program uses the mouse), and
  appended to the buffer. The engine has exactly one source-read site because the
  mouse decoder's labels are per function.
  [[src/codegen/builtins/io/gen_read_family.rs:emit_stdin_byte_read]]
- **The input delay.** When the decoder needs more on a **keyboard**, the engine
  waits for the deciding byte at most `INPUT_DELAY_MS` (25 ms) from the moment
  the front of the buffer started waiting (the region's pending word), then
  decodes again with `FINAL` — a lone ESC is then the Escape key. On **data** it
  waits for as long as the call allows: the rest of the sequence is on its way,
  and an escape byte in data is text. The pending word survives the call, so a
  program polling with `timeoutMs = 0` sees a lone Escape once 25 ms have passed
  in total. The mouse decoder's lone-`ESC` wait uses the same constant.
  [[src/codegen/io/input/mod.rs:INPUT_DELAY_MS]]
  [[src/codegen/io/mouse/decode.rs:ESCAPE_DELAY_MS]]
- **The wait.** Every source read first waits, until the call's deadline, for
  input to be ready: the calling thread's broadcast-log view first (bytes staged
  there are invisible to `poll`), then `poll` on standard input with the
  remaining milliseconds rounded up. A wait that runs out leaves the buffer
  exactly as it was.
  [[src/codegen/io/input/reader.rs:emit_read_wait]]
  [[src/codegen/io/stdin/stdin_broadcast/mod.rs:emit_stdin_poll_ready_check]]

### Timeouts

Every reader takes `[timeoutMs]` by the language's timeout convention: the
omitted argument is the `i64::MIN` sentinel (`DefaultValue::Fill`) and waits
without bound; `0` makes one attempt (the buffer, then the source once with a
zero-length wait); a positive value is a deadline `now + timeoutMs` on the
monotonic clock (capped at 10^12 ms); any other negative value raises
`ErrInvalidArgument` before the reader touches the terminal or the source. A
deadline that passes raises `ErrTimeout`.
[[src/codegen/error/constants/error_constants.rs:TIMEOUT_UNBOUNDED_SENTINEL]]
[[src/codegen/io/input/reader.rs:TIMEOUT_MS_CAP]]

## The readers

The four keystroke readers share one body and differ in what they take from the
engine. Each switches a terminal to single-key, no-echo mode for the call (and
restores it), and an `--app` window to raw input, for the wait as well as the
read. [[src/codegen/builtins/io/gen_read_key_family.rs:lower_key_reader]]

| Reader | Engine mode | Takes | Skips |
|---|---|---|---|
| `readRaw` | byte | the next byte as delivered | nothing |
| `readScalar` | token | the first scalar of a text key; the rest go back to the buffer | releases, keys with no text, ill-formed UTF-8 |
| `readGrapheme` | token, `CLUSTER` | the grapheme cluster of a text key (a fresh arena `String`) | the same |
| `readKey` | token, `CLUSTER` | every key event, as an `io::Key` record | ill-formed UTF-8 |

The `io::Key` record's fields, in declaration order: `key` (`KeyName`),
`grapheme` (`String`), `event` (`KeyEvent`), `shift`, `ctrl`, `alt`, `meta`. The
Escape key's grapheme is U+001B. The `End` key's variant is spelled `EndKey`
because `END` is a reserved word.
[[src/codegen/builtins/io/mod.rs:KEY_TYPE]]

`readLine` and `input` read a line one of two ways:
[[src/codegen/builtins/io/gen_read_line_family.rs:lower_read_line_family]]

- **Keystrokes** (a keyboard source): text keys append their text, Backspace
  erases one scalar, Enter ends the line, and keys with no text, Escape and
  releases are skipped. This reads correctly whether a terminal's line discipline
  committed the text or an `--app` window delivered key events.
- **Data** (a pipe or a file): the bytes up to the next LF, a CR before it
  dropped, checked as UTF-8 as they arrive.

Either way ill-formed UTF-8 raises `ErrEncoding` — a line reader reads data, and
a silently dropped byte would be a shortened line. The whole line is still consumed through its LF (or the end of input) and
`ErrEncoding` is raised once, when the line ends, so the next call starts at the
next line and the tail of a bad line never comes back as a line of its own — the
same answer `fs::readLine` gives. [[src/codegen/builtins/io/gen_read_line_family.rs:lower_read_line_family]]
A line read that times out
raises `ErrTimeout` and gives the part read so far back to the front of the
buffer; a part longer than the buffer holds is finished without a deadline
instead of lost. Under `term::on` the terminal is returned to cooked, echoing
mode for the line and single-key mode is re-applied after.

## Terminals: the kitty keyboard protocol

A legacy terminal sends keys as bytes whose meaning is partly ambiguous (a lone
ESC; Ctrl+I and Tab) and reports no releases. `term::on`, in a console program
whose readers decode keys, switches a terminal that speaks the kitty keyboard
protocol (<https://sw.kovidgoyal.net/kitty/keyboard-protocol/>) to it:

1. After stdin is in raw mode — only then, so the replies are not echoed, and
   only when the protocol is not already on — it writes `CSI ? u` (report the
   current flags) and `CSI c` (Primary Device Attributes, which every terminal
   answers).
2. It reads the replies for up to `KITTY_PROBE_MS` (500 ms): a flags reply
   `CSI ? … u` before the DA reply `CSI ? … c` means the terminal speaks the
   protocol. Every other byte that arrives meanwhile — a key the user typed — is
   appended to the input buffer in order. End of input, a read error or the
   deadline ends the probe with the protocol off.
3. If it does, `term::on` pushes flags 31 — disambiguate (1), event types (2),
   alternate keys (4), all keys as escape codes (8), associated text (16) — with
   `CSI > 31 u`, and sets the process-global `_mfb_rt_input_events` word, which
   the console readers turn into the decoder's `EVENTS` flag.

`term::off` pops the flags (`CSI < u`) and clears the word; the exit teardown
calls `term::off`, so an uncaught error or a SIGINT/SIGTERM leaves the terminal
clean. `readLine`/`input` under `term::on` pop the flags before the cooked line
read and push them after single-key mode is re-applied.
[[src/codegen/term/core/term/kitty.rs:emit_kitty_enable]]
[[src/codegen/term/core/term/kitty.rs:emit_kitty_disable]]
[[src/codegen/term/core/term/kitty.rs:emit_kitty_line_window]]
[[src/codegen/term/core/term/mod.rs:KITTY_PROBE_MS]]
[[src/codegen/builtins/io/gen_read_key_family.rs:INPUT_EVENTS_SYMBOL]]

Under the protocol a terminal sends Ctrl+C as a key (`CSI 99 ; 5 u`) instead of
the `0x03` the line discipline turns into SIGINT. The console reader restores the
line discipline's behavior: while the events word is set, a Ctrl+C press or
repeat with no other modifier raises SIGINT and is consumed, not returned. POSIX
only — a Windows console keeps `ENABLE_PROCESSED_INPUT` and never has the
protocol on in practice. [[src/codegen/io/input/reader.rs:emit_interrupt_key]]

Keys typed between two reads while the protocol is off are whatever the terminal
sends in its current mode. On a Windows console the readers set
`ENABLE_VIRTUAL_TERMINAL_INPUT` only for the call (special keys typed between
calls lose their sequences), while `term::on` holds it for its whole duration.
[[src/target/win_x86_64/code.rs:emit_apply_raw_mode]]

## `--app` windows: one encoding

Every `--app` key handler — macOS `keyDown:`/`keyUp:`, GTK
`key-pressed`/`key-released`, the Windows window procedure and the transcript
EDIT's subclass — turns a platform key event into the bytes a kitty-protocol
terminal sends for it under flags 31, through one shared encoder
`_mfb_rt_input_encode_key`, and writes them into the window input pipe. The
decoder therefore reads an `--app` window exactly as it reads a kitty terminal,
and every `--app` read reports `Press`/`Repeat`/`Release`. The per-platform
handlers are specified in `./mfb spec app console-io`.
[[src/codegen/io/input/encode.rs:lower_input_encode_key]]
[[src/codegen/io/input/encode.rs:INPUT_ENCODE_SYMBOL]]

## See Also

* ./mfb man io — the reader API
* ./mfb spec threading os-integration — the per-thread standard-input log
* ./mfb spec app console-io — the `--app` window pipe and key handlers
* ./mfb spec app term-backend — `term::on`/`term::off` and the mouse decoder
