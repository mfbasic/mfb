Evidence gathered. Here are the findings.

### 1. `isInputTerminal` says TRUE where the readers it gates raise `ErrWrongMode`
UNIT:      man-pkg:io
PAGE:      package-wide (overview; io::isInputTerminal, io::isOutputTerminal, io::isErrorTerminal; the readers)
CATEGORY:  consistency
CLAIM:     Overview: "in app mode all of them are routed to the application transcript window, which is treated as an interactive terminal." Each predicate page: "In app mode this call returns TRUE, because the application window is treated as an interactive console." The isInputTerminal example: `IF io::isInputTerminal() THEN io::print(io::input("Name: ")) ELSE io::print(io::readLine())`.
VERDICT:   misleading
EVIDENCE:  `--app` probe under /tmp/plan-125-scratch/B-recheck/man-pkg-io/appproj (program references `app::setMode`, so it starts in `app::Mode.None`), run as `echo hi | appproj.app/Contents/MacOS/appproj`, printed `inputTerminal=TRUE`, `outputTerminal=TRUE`, then `readLine err=77050020` (ErrWrongMode). Code: `src/codegen/builtins/io/gen_is_terminal.rs` emits `TRUE` for every app build with no mode check (`emit_app_io_is_terminal`), while the reader pages correctly say they raise `ErrWrongMode` in `app::Mode.None`. So in Mode.None there is no "transcript window" to be routed to, yet the predicates still answer TRUE, and the isInputTerminal example's "terminal" branch raises.
SUGGESTED: On the three predicate pages and in the overview: "In an --app build this call returns TRUE in every app::Mode, including app::Mode.None, where there is no window: it reports that input is interactive, not that a reader will succeed — the readers raise ErrWrongMode in app::Mode.None." Drop "routed to the application transcript window" from the overview sentence.

### 2. No page shows how to read standard input to the end
UNIT:      man-pkg:io
PAGE:      package-wide (overview; io::readLine, io::readRaw)
CATEGORY:  coverage
CLAIM:     Overview: "End of input is reported as an error, not as an empty or sentinel result, once no input is left". No page ever names `ErrEndOfFile` in the overview prose, and no example on any of the 17 pages loops until it.
VERDICT:   missing
EVIDENCE:  `grep -n ErrEndOfFile src/codegen/builtins/io/func_*.rs` hits only `desc` text and error tables; the only example that handles it is readRaw's single-byte `readRaw(0)`. The processing-a-pipe idiom needs an inline TRAP that ends in RECOVER (a naive `TRAP` that only sets a flag fails to compile: `TYPE_INLINE_TRAP_FALLS_THROUGH`), so a developer cannot guess it. Probe /tmp/plan-125-scratch/B-recheck/man-pkg-io/eofproj (`WHILE more ... io::readLine() TRAP(e) IF e.code = errorCode::ErrEndOfFile THEN more = FALSE; RECOVER "" END IF; FAIL e END TRAP`) with `printf 'a\r\nb\nc' |` printed `1: [a]`, `2: [b]`, `3: [c]`, `lines: 3`, exit 0 — the idiom works and also demonstrates the CRLF and unterminated-last-line rules the overview states.
SUGGESTED: Name `ErrEndOfFile` in the overview's end-of-input sentence, and add a "Read every line of a pipe" example to io::readLine (the probe above, compiled and run), with a cross-reference from the overview and from readRaw ("to drain a binary stream, loop the same way").

### 3. `io::write`'s intro contradicts its own overloads and the overview
UNIT:      man-pkg:io
PAGE:      io::write (also the overview's function table)
CATEGORY:  consistency
CLAIM:     Intro/table row: "Write a String to standard output with no trailing newline". Overview: "io::print and io::write accept a String or an AttributedString".
VERDICT:   inconsistent
EVIDENCE:  `src/codegen/builtins/io/func_write.rs:25` `const INTRO = "Write a \`String\` to standard output with no trailing newline"`; the same page lists overload 2 `io::write(value AS AttributedString)`. Sibling `func_print.rs:27` says "Write text to standard output followed by a newline". A reader scanning the overview table sees print take "text" and write take "a String" and concludes only print accepts AttributedString.
SUGGESTED: Intro: "Write text to standard output with no trailing newline" (mirrors io::print; printError/writeError correctly keep "a String").

### 4. `io::flush` claims to present the TUI frame; the sibling pages say only `term::sync` does
UNIT:      man-pkg:io
PAGE:      io::flush (versus io::print, io::write)
CATEGORY:  consistency
CLAIM:     flush: "In app mode transcript writes are synchronous; while term:: TUI mode is active, this call also asks for the current frame to be presented." print/write: "While the program is in term:: TUI mode, standard output is retained rather than printed and nothing reaches the terminal until term::sync presents the frame."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/io/func_flush.rs:lower_flush`: the console branch only calls `STDOUT_DRAIN_SYMBOL` and reports a write failure ("nothing else", per its own comment); the present-on-flush sequence (`emit_app_io_flush`) is emitted only when `ctx.build_mode.is_app()`. `mfb man term sync | grep -i flush` prints nothing, and `grep -rn 'io::flush' src/codegen/builtins/term/` is empty, so no term page corroborates flush presenting a frame. The sentence's second clause is not visibly scoped to app mode, so a console TUI developer reads it as "io::flush presents". UNVERIFIED at runtime: a TUI probe needs a tty this sandbox does not have.
SUGGESTED: "In app mode transcript writes are synchronous, and in app mode only, while term:: TUI mode is active, this call also asks for the current frame to be presented. In a console program the frame is presented only by term::sync."

### 5. "share one queue" versus "its own view": two vocabularies for one fact
UNIT:      man-pkg:io
PAGE:      package-wide (overview vs io::readRaw, io::readScalar, io::readGrapheme, io::readKey)
CATEGORY:  consistency
CLAIM:     Overview: "Each thread that reads standard input has its own view of it: reading in one thread does not take input away from another". Four reader pages: "The io readers share one queue of input, so they can be mixed without losing or reordering anything."
VERDICT:   inconsistent
EVIDENCE:  `grep -n 'share one queue\|own view' src/codegen/builtins/io/*.rs` — mod.rs:139 uses "view", func_read_raw.rs:49, func_read_scalar.rs:54, func_read_grapheme.rs:60 use "queue" (readKey says the same in prose). `mfb man thread openStdIn` uses "every subscriber gets its own independent view". Both statements are true, but "share one queue" read next to the overview's per-thread view suggests the opposite (one program-wide queue), and nothing on the reader pages says the sharing is within a thread.
SUGGESTED: On the four reader pages: "Within a thread, the io readers share one queue of input, so they can be mixed without losing or reordering anything; each thread has its own view (see the package overview and thread::openStdIn)."

Checked and found consistent: every reader (key family and line family) drains the stdout buffer before blocking, matching the overview's "before any read" (`grep -n STDOUT_DRAIN src/codegen/builtins/io/*.rs`: gen_read_key_family.rs:169, gen_read_line_family.rs:208); buffering, per-thread flag and app-mode inertness agree across setBuffered/isBuffered/flush/overview; Escape and Alt handling agree across overview, readKey, readScalar, readGrapheme and KeyEvent.NotSupported; the overview's function table is alphabetical and its prose walks output, keystroke readers, line readers, timeouts, errors, predicates and threads in that order, so every page is reachable from it. No repository file was edited; scratch files are under /tmp/plan-125-scratch/B-recheck/man-pkg-io.
