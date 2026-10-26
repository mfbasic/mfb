### 1. Piped escape sequences are decoded as keys, not read as text
UNIT:      man-page:io/overview
CLAIM:     "From a pipe or a file the same calls read the data as it is: an escape byte in the data is text, not a key. io::readRaw returns the next byte exactly as it arrived, whatever the source."
VERDICT:   wrong
EVIDENCE:  `/tmp/plan-125-scratch/D-phase6-current/man-page-io-overview/p_key` (loop of `io::readKey()` printing `k.key`, `k.grapheme`, `k.ctrl`, `k.alt`), run as `printf 'a\x1b[A\x1bb\x01\n' | p_key.out` printed:
```
key=Unicode g=[a] ctrl=FALSE alt=FALSE
key=Up g=[] ctrl=FALSE alt=FALSE
key=Unicode g=[b] ctrl=FALSE alt=TRUE
key=Unicode g=[a] ctrl=TRUE alt=FALSE
key=Enter g=[
] ctrl=FALSE alt=FALSE
ERR 77020003
```
`ESC [ A` from a pipe is the Up key, `ESC b` is Alt+b, `0x01` is Ctrl+a. `p_graph` (`io::readGrapheme()` loop) with `printf 'e\xcc\x81x\x1b[A\n'` printed `<é>`, `<x>`, `<\n>` and skipped the whole `ESC [ A`; with `printf '\x1b\n'` the lone ESC was skipped too. `p_scalar` (`io::readScalar()` loop) with `printf 'a\x1b[Ab\n'` printed `<a>`, `<b>`, `<\n>`. Only `io::readRaw` (`p_raw`, `printf '\x1b[A'` → `27 91 65`) and `io::readLine` (whose own page says "An escape byte in the data is just a…", `src/codegen/builtins/io/func_read_line.rs:DESC`) treat ESC as data. The `readKey` sibling's own description (`func_read_key.rs:DESC`, "a console outside `term::on`, a pipe, a file — every key arrives once") agrees with the binary, not with this overview.
SUGGESTED: "From a pipe or a file the keystroke readers decode the same way: an escape sequence in the data is still a key (`ESC [ A` is Up, `ESC b` is Alt+b, a control byte is a Ctrl chord), so `io::readGrapheme` and `io::readScalar` skip it just as they would a typed arrow. `io::readLine` and `io::input` are the line readers and keep an escape byte as text; `io::readRaw` returns every byte exactly as it arrived, whatever the source."

### 2. The keystroke text readers drop ill-formed UTF-8 silently; they do not reject it
UNIT:      man-page:io/overview
CLAIM:     "Line and text reads reject ill-formed UTF-8 rather than substituting replacement characters."
VERDICT:   wrong
EVIDENCE:  `p_graph` with `printf 'a\xffb'` printed `<a> len=1`, `<b> len=1`, `ERR 77020003` — the `0xFF` vanished with no error. `p_scalar` with `printf 'a\xffb\n'` printed `<a>`, `<b>`, `<\n>`. Neither `readGrapheme` nor `readScalar` lists `ErrEncoding` in its descriptor (`func_read_grapheme.rs`/`func_read_scalar.rs`, `errors:` vectors), so they cannot raise it. Only the line readers reject: `p_lines` with `printf 'ok\n\xff\xfe\nafter\n'` printed `[ok]` then `ERR 77020004` (ErrEncoding).
SUGGESTED: "io::input and io::readLine reject an ill-formed UTF-8 line with ErrEncoding rather than substituting replacement characters. io::readGrapheme and io::readScalar skip a byte that is not valid UTF-8 and move on to the next character; use io::readRaw to see the bytes themselves."

### 3. readScalar returns a Scalar, not text
UNIT:      man-page:io/overview
CLAIM:     "io::readGrapheme and io::readScalar return only text, one user-perceived character or one Unicode scalar at a time, and skip keys that type nothing."
VERDICT:   misleading
EVIDENCE:  `LET s AS String = io::readScalar()` fails to compile: `error[2-203-0007 TYPE_BINDING_MISMATCH]: Binding `s` has initializer type Scalar, expected String` (first build of `p_scalar`). `readGrapheme` returns `String` (`func_read_grapheme.rs`, `return_type: ParameterType::String`); `readScalar` returns `Scalar` (`lower_key_reader(..., KeyReader::Scalar)`). A reader following "return only text" into `LET … AS String` is stopped by the compiler.
SUGGESTED: "io::readGrapheme returns one user-perceived character as a String; io::readScalar returns one Unicode scalar as a Scalar (convert with toString to join it to text). Both skip keys that type nothing."

### 4. readLine and input strip a CR before the LF; the overview never says so
UNIT:      man-page:io/overview
CLAIM:     "io::input reads a whole line with normal terminal echo and an optional prompt; io::readLine reads a line without a prompt and, on a terminal, suppresses echo while it reads"
VERDICT:   incomplete
EVIDENCE:  `p_lines` with `printf 'a\r\nb\n' | p_lines.out | od -c` printed `[ a ] \n [ b ] \n` — the CR is gone. `printf 'a\rb\nc\r\n'` printed `[ a \r b ]` then `[ c ]`: only a CR immediately before the LF is removed. The `readLine` sibling documents this (`func_read_line.rs:DESC`, "a carriage return (CR) immediately before the LF — a CRLF ending — is removed too"); the overview, which is where a reader piping a Windows file first lands, does not.
SUGGESTED: Append to the paragraph: "A line ends at a line feed; a carriage return just before it (a CRLF ending) is removed with it, and the returned String carries neither."

### 5. ErrWriteFailed, ErrInputFailed and ErrOutOfMemory have no stated condition; a broken pipe ends the program instead of raising
UNIT:      man-page:io/overview
CLAIM:     Errors table rows `77020002 ErrWriteFailed`, `77020005 ErrInputFailed`, `77010001 ErrOutOfMemory` (no sentence in the Description names what raises any of them)
VERDICT:   incomplete
EVIDENCE:  The Description mentions ErrTimeout, ErrEndOfFile, ErrInvalidContext (and alludes to encoding) but never says when a write fails. The one write failure a developer will actually hit — the reader of `prog | head` going away — does **not** raise: `p_epipe` (200000 `io::print` calls, then `io::printError("finished loop")`, with a function-level TRAP that prints the code) run as `p_epipe.out | head -1` printed `line 0`, nothing from the TRAP, nothing from `printError`, and exited with status 141 (SIGPIPE). Source: `gen_write_family.rs` comment "bug-467 … the pipeline convention (`prog | head` ends when `head` exits) is restored explicitly by putting SIGPIPE back to `SIG_DFL` and re-raising it. Every other errno still raises `ErrWriteFailed`". ErrInputFailed is wired at `gen_read_key_family.rs:input_error` / `gen_read_line_family.rs:input_error` for a failed read system call; I could not provoke it from a pipe (UNVERIFIED as to a reproducible trigger).
SUGGESTED: Add: "A write or flush that the operating system refuses raises ErrWriteFailed, with the unwritten text kept for a later flush. The one exception is the reader of a pipeline closing early (`prog | head`): the program then ends quietly the way any shell tool does, and no error is raised. A read that the operating system refuses raises ErrInputFailed."

### 6. A negative timeoutMs raises ErrInvalidArgument; the overview omits the case
UNIT:      man-page:io/overview
CLAIM:     "Every reader takes an optional timeoutMs: omitted, the call waits for as long as it takes; 0 makes one immediate attempt, which is how a game loop asks "was a key pressed?" without stopping; a positive value waits up to that many milliseconds."
VERDICT:   incomplete
EVIDENCE:  `p_timeout`: `io::readLine(-1) TRAP(e)` printed `NEG 77050002`; `p_negscalar`: `io::readScalar(-5)` printed `NEGS 77050002` (ErrInvalidArgument). The Errors table lists `77050002 ErrInvalidArgument` and nothing in the Description says what raises it; the parameter's name does not say the bound is zero-inclusive. The positive cases hold: `(printf 'abc'; sleep 1; printf 'def\n'; sleep 1) | p_timeout.out` printed `T 77050008`, `b=[abcdef]` (half-read line kept), and `readLine(0)` on a silent open pipe printed `ZERO 77050008`; `printf 'hi\n' | (sleep 0.3; p_zero.out)` printed `[hi]`.
SUGGESTED: "…a positive value waits up to that many milliseconds, and a negative value raises ErrInvalidArgument."

### 7. App-mode claims could not be exercised
UNIT:      man-page:io/overview
CLAIM:     "From a terminal or an --app window they read keys" / "in app mode all of them are routed to the application transcript window, which is treated as an interactive terminal" / Errors row `ErrWrongMode` ("…they trap in app::Mode.None alone")
VERDICT:   UNVERIFIED
EVIDENCE:  No `--app` window can be opened in this sandbox. Everything else on the page that a pipe or a pseudo-terminal can reach was run: terminal predicates `in=FALSE out=FALSE err=FALSE` under a pipe (`p_term`) and `tty=TRUE/TRUE` under `script -q /dev/null` (`p_echo`); `io::input("Prompt: ")` echoed `visible` on the pty while `io::readLine()` did not echo `hidden`; `p_closed` run `<&-` printed `READ-ERR 77020003` and `>&-` / `2>&-` succeeded with exit 0; `p_buf` run `echo hi | p_buf.out 2>&1` printed `buffered0=FALSE`, `buffered1=TRUE`, `BA` (held until flush), `PQhi` (drained before the read), `XYFALSE` (setBuffered(FALSE) drains), `END-UNFLUSHED` (drained at exit); `p_thread` printed `worker-unsubscribed=77050019` then `worker-subscribed=5`; `p_bytes` emitted `a \r \n b \t c \ d \n é` byte for byte; `p_bad1`/`p_bad2`/`p_attr_err` were rejected at compile time for Integer and AttributedString arguments to `print`/`printError`, and `p_attr` printed an AttributedString through `print` and `write`. The 128-subscriber cap was verified only by source (`src/codegen/io/stdin/stdin_broadcast/subscription.rs`, comment at the `find` label: a thread past `STDIN_LOG_MAX_SUBSCRIBERS` is left unregistered so its reads raise ErrInvalidContext), not by spawning 129 threads.
SUGGESTED: Main thread to run `p_key`, `p_term` and a `Mode.None` read under `--app`; no wording change proposed from here.
