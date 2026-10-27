Nothing further to gather. Repository is untouched (`git status --short` prints 0 lines). Findings follow.

### 1. Overview and cancel say only receive/accept wake on cancel; send/transfer pages say send waits wake too, and the probe confirms send does
UNIT:      man-pkg:thread
PAGE:      overview (and thread::cancel, thread::isCancelled)
CATEGORY:  consistency
CLAIM:     "The one exception is a worker blocked in thread::receive or thread::accept: it wakes and fails with ErrInterrupted" (overview); cancel page: "except that a worker blocked in thread::receive or thread::accept wakes and fails with ErrInterrupted"; isCancelled page: "exception: it wakes and fails with ErrInterrupted" (same pair). Meanwhile thread::send and thread::transfer each say "If the calling thread is cancelled with thread::cancel while waiting, it raises ErrInterrupted."
VERDICT:   inconsistent (the overview/cancel/isCancelled wording is also wrong)
EVIDENCE:  Probe /tmp/plan-125-scratch/B-recheck/man-pkg-thread/pA: worker loops `thread::send(w, …)` on a thread started with `thread::start(talker, "x", 1, 1)`; parent polls until one message is queued, then calls `thread::cancel(t)` and `thread::waitFor(t)`. Output: `worker trapped 77050009 Operation was interrupted before completion.` / `worker returned -1`. The worker was blocked in send on a full queue and woke with ErrInterrupted. Source: `src/codegen/builtins/thread/mod.rs:128`, `func_cancel.rs:19`, `func_is_cancelled.rs:18` name only receive/accept; `queue.rs:thread_queue_write_helper` has the `_interrupted` exit.
SUGGESTED: In all three places: "The one exception is a worker blocked in a channel call — thread::receive, thread::accept, thread::send or thread::transfer while it waits for room: it wakes and fails with ErrInterrupted, which it may trap to shut down in order."

### 2. accept says six resource types may arrive; transfer and the overview say a Thread may cross too, and it does
UNIT:      man-pkg:thread
PAGE:      thread::accept
CATEGORY:  consistency
CLAIM:     "The same six built-in resources may arrive here that thread::transfer may send: fs::File, tcp::Socket, tcp::Listener, udp::Socket, tls::Socket and tls::Listener."
VERDICT:   inconsistent
EVIDENCE:  thread::transfer page: "These may — fs::File, tcp::Socket, tcp::Listener, udp::Socket, tls::Socket, tls::Listener and a Thread itself … a coordinator thread may be handed the threads it waits for." Overview: "and so may a Thread (never a ThreadWorker)". Probe pE: `collector` worker declared `ThreadWorker OF RES Thread OF Nothing TO Integer TO Integer` accepts a child Thread via `thread::accept(w, 2000)` and waits for it; parent transfers it with `thread::transfer(col, c, 2000)`. Output: `collector returned 42`. Predicate: `src/codegen/resource/mod.rs:is_builtin_sendable_resource_type` returns `!worker` for any `ThreadHandle`.
SUGGESTED: "What thread::transfer may send may arrive here: fs::File, tcp::Socket, tcp::Listener, udp::Socket, tls::Socket, tls::Listener, and a Thread itself — a coordinator may accept the threads it is to wait for. A ThreadWorker never arrives."

### 3. Overview restricts thread-sendable containers to "immutable" ones; a MUT List crosses the message channel
UNIT:      man-pkg:thread
PAGE:      overview
CATEGORY:  overview-mismatch
CLAIM:     "Values allowed to cross are called thread-sendable: numbers, String, records, unions and immutable containers are, as long as everything inside them is."
VERDICT:   misleading
EVIDENCE:  Probe pB: `MUT data AS List OF Integer = [1, 2, 3]`, `thread::send(t, data)` on a `Thread OF List OF Integer TO Integer`, then `collections::set(data, 0, 100)`; worker sums its copy and sends `[total, 99]` back. Compiles and prints `back 6 2` / `result 6` — the mutable List crossed, the worker saw the pre-mutation copy, and a List came back. Nothing in the language marks a List "immutable", so the reader has no way to know whether their `List OF Integer` qualifies. `mod.rs:78`.
SUGGESTED: "numbers, String, records, unions, and List, Map and Set are, as long as everything inside them is; each arrives as its own copy. Functions, lambdas and resources are not (resources go on the resource channel)."

### 4. No page says what omitting timeoutMs does on receive/accept/send/transfer
UNIT:      man-pkg:thread
PAGE:      package-wide (thread::receive, thread::accept, thread::send, thread::transfer)
CATEGORY:  coverage
CLAIM:     Each of the four declares `[timeoutMs AS Integer]` as optional and describes only the "waiting up to timeoutMs milliseconds" case; the parameter desc is "How long to wait, in milliseconds, for a message to arrive." The overview's own examples (`thread::send(t, "acknowledged")`) omit it without comment.
VERDICT:   missing
EVIDENCE:  `grep -n -i "indefinitely\|omit\|forever" all.txt` finds "omit" only on the openStdIn/closeStdIn pages. Probe pD: worker calls `thread::receive(w)` with no timeout and nothing is ever sent; the program is still blocked after 5 s (`perl -e 'alarm 5'` exit 142). Runtime comment `queue.rs:ThreadReadMode` doc: "The no-arg receive/accept overload passes the block sentinel … and waits indefinitely". The omitted-form on the parent side also ends when the worker finishes (ErrNotFound), which is the only way out, and the pages never say so either.
SUGGESTED: On each parameter desc: "How long to wait, in milliseconds. Omit it to wait without limit — until a message arrives, the thread is cancelled, or the other side has finished (ErrNotFound)." Mirror on send/transfer: "Omit it to wait without limit for room."

### 5. `mfb man thread types` is a dead end; the Thread/ThreadWorker type pages live under `mfb man types` and nothing in this unit points there
UNIT:      man-pkg:thread
PAGE:      overview and the types stub
CATEGORY:  discoverability
CLAIM:     `mfb man thread types` prints "The thread package has no public types. Run mfb man thread to list its functions." The overview spends six paragraphs on `Thread OF Msg RES Res TO Out`, STATE, RES binding and ThreadWorker rules, and never names where those two types are documented.
VERDICT:   missing
EVIDENCE:  `mfb man thread types` output above. `mfb man types | grep -n Thread` lines 150-161: a "Concurrency" section defines both handle types. `grep -n "mfb man" all.txt` in the thread overview shows only the `… — mfb man <pkg> types` See-also rows for other packages' resources; no `mfb man types` and no `mfb man variable` link, despite §4.5 of the standard saying handle/copy semantics link `mfb man variable` rather than being restated (the overview restates alias/scope/RES rules in two paragraphs). `mod.rs:381` registers the types as opaque source types, which is why the stub renders.
SUGGESTED: Add to the overview's handle paragraphs: "Thread and ThreadWorker are built-in types, described with the rest under mfb man types; how a RES handle, an alias and scope-exit closing work is mfb man variable." If the stub text is derived, have the renderer's stub for a package with `add_source_types` point at `mfb man types` instead of only "Run mfb man thread".

### 6. Overview's "every built-in socket and listener may cross" list leads with a file and omits that the list is complete
UNIT:      man-pkg:thread
PAGE:      overview
CATEGORY:  overview-mismatch
CLAIM:     "Every built-in socket and listener may cross — fs::File, tcp::Socket, udp::Socket, tcp::Listener, tls::Socket and tls::Listener — so a server may accept on one thread and hand each connection to a worker, and so may a Thread (never a ThreadWorker)."
VERDICT:   misleading
EVIDENCE:  fs::File is neither a socket nor a listener (`src/codegen/builtins/fs/mod.rs:188 sendable: true`); a reader scanning for "may my File cross?" is told about sockets. The transfer page states the same set correctly as "Only some resource types may cross at all. These may — …". The sentence also attaches "and so may a Thread" to the server clause, so it reads as "so may a Thread accept on one thread".
SUGGESTED: "Seven built-in resources may cross — fs::File, tcp::Socket, tcp::Listener, udp::Socket, tls::Socket, tls::Listener and a Thread itself (never a ThreadWorker) — so a server may accept on one thread and hand each connection to a worker."

Checked and found consistent: ErrInvalidContext for a worker reading stdin without openStdIn (probe pC printed code 77050019 with that message); the queue-limit default and ErrThreadLimit wording match `func_start.rs`; the function table reaches every page and the overview's reading order (start, channels, waitFor, handles, STATE, cancel, stdin) matches the functions delivered.
