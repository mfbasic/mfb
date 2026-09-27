### 1. EOF rule omits unterminated final lines
UNIT:      man-page:io/overview  
CLAIM:     “End of input is reported as an error, not as an empty or sentinel result.”  
VERDICT:   misleading  
EVIDENCE:  Probe `main.mfb` read one scalar, one byte, then `io::readLine`; `printf 'éZtail' | .../io_overview_probe.out` printed `ch=[é]`, `byte=90`, `line=[tail]` with no error. `src/codegen/builtins/io/func_read_line.rs:DESC` confirms that EOF after line bytes returns the final unterminated line, and only the next call raises `ErrEndOfFile`.  
SUGGESTED: “End of input before any input for a character, byte, or line read is reported as an error. A line ending at end of input is returned as a final unterminated line; the next read reports end of input.”

### 2. Runtime implementation detail on a developer page
UNIT:      man-page:io/overview  
CLAIM:     “The runtime reads standard input once, in chunks, into a single log all threads share; each subscribed thread reads at its own position in that log, so every subscriber sees the whole stdin stream from the point it subscribed, and a byte one thread reads is still there for the others.”  
VERDICT:   out-of-scope  
EVIDENCE:  This teaches the implementation (“runtime,” “chunks,” “single log,” and read positions), contrary to `.ai/man-content.md` §3. The developer-visible behavior is corroborated by `src/codegen/builtins/thread/func_open_std_in.rs:DESC`: subscribers have independent views and a late subscriber sees only subsequent input.  
SUGGESTED: “Each subscribed thread has its own view of standard input: reading in one thread does not take input away from another, and a thread sees input that arrives after it subscribes.”

### 3. Compiler implementation detail on a developer page
UNIT:      man-page:io/overview  
CLAIM:     “The compiler subscribes the main thread at program entry, so a single-threaded program is byte-identical to a direct per-byte reader.”  
VERDICT:   out-of-scope  
EVIDENCE:  `.ai/man-content.md` §3 prohibits compiler mechanics. `src/codegen/engine/function/entry.rs:emit_entry_body` does perform the subscription, but that is not a terminal developer concern.  
SUGGESTED: “The main thread can read standard input without calling `thread::openStdIn`; worker threads must subscribe first.”

### 4. Subscription capacity can still produce ErrInvalidContext
UNIT:      man-page:io/overview  
CLAIM:     “A thread other than main must subscribe with `thread::openStdIn` before reading, or the read raises `ErrInvalidContext`.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/io/stdin/stdin_broadcast.rs:lower_stdin_subscribe` limits the subscriber registry to `STDIN_LOG_MAX_SUBSCRIBERS` (128). At capacity, `thread::openStdIn` returns normally but does not register the thread; its later read raises `ErrInvalidContext`. The source explicitly calls this the documented limit.  
SUGGESTED: “A worker thread must subscribe with `thread::openStdIn` before reading. At most 128 threads can be subscribed at once; an additional thread cannot read and raises `ErrInvalidContext`.”