### 1. `flow` does not deliver what `variable` promises about EXIT and CONTINUE
UNIT:      man-topic:variable
PAGES:     `mfb man variable` (Where to look next), `mfb man flow`
CATEGORY:  broken-seam
QUOTE-A:   `mfb man variable`: "mfb man flow — the block forms a LET or MUT can be declared inside, and EXIT, CONTINUE and os::exit, which all run the cleanup this page describes."
QUOTE-B:   `mfb man flow` (only cleanup sentence on the whole topic, `mfb man flow --all | grep -n -i cleanup`): "os::exit(code) terminates the process after running lexical cleanup for every live scope; no TRAP catches it." Nothing on `flow` says EXIT or CONTINUE close a handle declared inside the loop body.
VERDICT:   The behavior is real (probe `p13_exit_closes`: a RES opened in a FOR body, then `EXIT FOR`, the file is closed and readable afterwards), but the reader sent to `flow` for it finds nothing. Either `flow` must say it or `variable` must stop promising it. Since `variable` owns the value model, the cheapest consistent fix is for `variable` to state the fact itself in "Handles close themselves" and narrow the link.
SUGGESTED: In `variable`'s "Handles close themselves": "…on every way out that does not RETURN the handle itself — a normal end of scope, RETURN, FAIL, a failure that propagates, EXIT or CONTINUE out of a loop body, and os::exit." Then the `flow` bullet becomes: "mfb man flow — the block forms a LET or MUT can be declared inside."

### 2. Use-after-close: `variable` says compile-time where visible plus ErrResourceClosed at run time; `tour` says always a compile error
UNIT:      man-topic:variable
PAGES:     `mfb man variable` (The exception: RES handles), `mfb man tour` (compared with C)
CATEGORY:  divergence
QUOTE-A:   `mfb man variable`: "Using a closed handle is refused at compile time where the compiler can see it, and reported as ErrResourceClosed where it cannot."
QUOTE-B:   `mfb man tour`: "Using a handle after closing it, or closing it twice, is a compile error (the move analysis tracks the close obligation the way you track it in your head)."
VERDICT:   `variable` wins and is measured: probe `p3_record_res_close` closes through `a.handle` and writes through the copied record's `b.handle`; it builds and fails at run time with 77030004 (errorCode::ErrResourceClosed per `mfb man errorCode`). The same-binding cases (`p9`, `p12`) are indeed TYPE_USE_AFTER_MOVE at compile time, so `tour` is only true for the direct case. `tour`'s parenthetical also names "move analysis", a compiler-internal term §3 bans.
SUGGESTED: `tour`: "Using a handle after closing it, or closing it twice, is refused at compile time where the compiler can see it, and fails with ErrResourceClosed where it cannot (mfb man variable)." Drop the parenthetical.

### 3. Passing a handle to a call: `variable`/`fs types` say alias, `thread` says "lends"
UNIT:      man-topic:variable
PAGES:     `mfb man variable`, `mfb man fs types`, `mfb man thread` (overview and `waitFor`)
CATEGORY:  divergence
QUOTE-A:   `mfb man variable`: "The call was given an alias, not a copy, and it did not take the handle away." `mfb man fs types`: "Passing one to a call … gives a second name for the same open handle — an alias, not a copy".
QUOTE-B:   `mfb man thread`: "Passing it to a function lends it, so the caller still holds it afterwards" (`src/codegen/builtins/thread/mod.rs:109`) and `mfb man thread waitFor`: "Passing t to a function lends it, so a helper may wait for…" (`func_wait_for.rs:20`).
VERDICT:   `variable` wins. §4.1 allows exactly four words and "alias" is the one for this fact; "lends"/"holds" is the borrow model under a synonym, and a reader who has just learned "alias" on `variable` meets a second vocabulary for the same event on the next page.
SUGGESTED: Both `thread` sentences: "Passing it to a function gives the function an alias, so the caller can still use it afterwards."

### 4. `tour`'s list of closing exits omits the RETURN-the-handle hand-over `variable` carves out
UNIT:      man-topic:variable
PAGES:     `mfb man variable` (Handles close themselves), `mfb man tour` (Resources and cleanup)
CATEGORY:  divergence
QUOTE-A:   `mfb man variable`: "A handle closes when its binding's scope ends — on every way out that does not RETURN the handle itself, including a failure." and "RETURN f does the same hand-over: … the caller's RES binding takes the one open thing".
QUOTE-B:   `mfb man tour`: "It is closed automatically when its scope ends, on every exit path — normal scope exit, RETURN, FAIL, propagated errors, and TRAP routing."
VERDICT:   `variable` wins; probe `p8_return_handle` returns a RES from a function and writes through the caller's binding afterwards, so an unqualified "RETURN" in `tour`'s list is misleading for the one RETURN that matters here.
SUGGESTED: `tour`: "…on every exit path — normal scope exit, RETURN (unless the handle itself is returned, which hands it to the caller), FAIL, propagated errors, and TRAP routing."

### 5. `fs types` and `tcp types` restate the handle model in a paragraph before linking
UNIT:      man-topic:variable
PAGES:     `mfb man variable`, `mfb man fs types`, `mfb man tcp types` (identical preamble)
CATEGORY:  redundancy
QUOTE-A:   `mfb man variable`: "This page is the one place the whole model is written down, so that no other page has to explain it."
QUOTE-B:   `mfb man fs types` / `mfb man tcp types` (word-for-word the same): "Passing one to a call, or copying a record that holds one, gives a second name for the same open handle — an alias, not a copy — and closing through either name closes it. Binding it to a new RES name (RES g = f) instead hands the handle to g; f cannot be used again. A handle closes itself when the binding that holds it leaves scope. A handle may be a field of a record and an element of a collection — written List OF RES <Type>, never a bare List OF <Type>. mfb man variable covers all of that in one place."
VERDICT:   §4.5 allows one sentence then a link. Five sentences covering alias, rebinding, scope close, record fields and `List OF RES` is the full model re-owned on every resource-type page; the next edit to `variable` will leave these stale. The last sentence of the restatement already concedes the point.
SUGGESTED: Replace the paragraph on both pages with: "Each of these is a RES handle: a second name for one is an alias of the same open thing, never a copy. mfb man variable explains the handle model in full. Whether a particular handle may be handed to another thread is stated with it."

### 6. Compiler accepts `LET xs = []` that `variable` and `types` say needs a spelled type
UNIT:      man-topic:variable
PAGES:     `mfb man variable` (LET and MUT), `mfb man types` (List)
CATEGORY:  fact-escaped-iter2 (page is on the intended contract; the defect is in the compiler, flagged for the main thread to file)
QUOTE-A:   `mfb man variable`: "An empty list or map still needs its type spelled."
QUOTE-B:   `mfb man types`: "An empty list needs an expected type". Compiler: probe `p6_empty_infer` (`LET xs = []` then `len(xs)`) builds and prints 0; probe `p6b` (`LET xs = []` then `collections::append(xs, 1)`) fails with the internal message `error: native collection list item must be Unknown, got Integer while lowering bind ys AS List OF Unknown` instead of a user-facing diagnostic at the declaration.
VERDICT:   The two pages agree and should stay as written. The compiler infers `List OF Unknown` for a bare `[]` and only breaks later with an internal error. This is a bug to file, not a page edit. The review rules forbid me from touching the repository, so the main thread should file it in `bugs/` with the repro under /tmp/plan-125-scratch/A-recheck-iter3/man-topic-variable/proj_p6b.
SUGGESTED: No wording change. File the bug: a bare `[]` with no annotation or expected type must be rejected at the LET with a real diagnostic.

Compared, for the record: every cross-reference on the page renders (`mfb man types`, `flow`, `lambda`, `thread`, `thread transfer`, `fs types`, `tcp types`, `mfb spec memory`). `lambda`'s forEach exception and capture wording, `thread`'s per-thread copy of top-level state and "Thread is a RES handle", `types`' record-with-RES-field copy rule, and `thread transfer`'s "sending binding cannot be used again" all agree with `variable`. Factual probes that matched the page: parameter assignment is TYPE_ASSIGN_REQUIRES_MUT (`p1`), `RES g = f` then using `f` is TYPE_USE_AFTER_MOVE (`p2`), STATE of a non-record is rejected (`p4`), duplicate LET in an IF body is SYMBOL_DUPLICATE_LOCAL (`p5`), the top-level shadow note reads exactly "`limit` is already a top-level binding" (`p7`), bare MUT defaults to 0 / empty (`p10`), and `f.state.pos = 7` works (`p11`).
