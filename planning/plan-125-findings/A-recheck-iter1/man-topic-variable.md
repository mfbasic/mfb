### 1. One name per function: block shadowing is refused, and the page never says so
UNIT:      man-topic:variable
PAGE:      variable (LET and MUT)
CATEGORY:  coverage
CLAIM:     "A name declared inside a function cannot reuse the name of a top-level LET or MUT that its file can see" — the only reuse rule on the page.
VERDICT:   missing
EVIDENCE:  Probe `p1_shadow.mfb` (outer `LET x` in `main`, inner `LET x` inside `IF TRUE THEN … END IF`): `mfb build -q proj_p1_shadow` → `error[2-201-0009 SYMBOL_DUPLICATE_LOCAL]: local symbol is declared more than once — Local binding `x` is already declared in this function.` A developer from any block-scoped language will try this first; the page only forbids the top-level case and implies the local case is fine.
SUGGESTED: Add one sentence under the top-level rule: "Within one function a name is declared once, whatever block it sits in: a second LET or MUT of the same name inside an IF, FOR or WHILE body is rejected (SYMBOL_DUPLICATE_LOCAL), not a new inner variable." Then: "A MUT declared outside a block may be assigned from inside it" (p14 printed `6`).

### 2. The declaration forms the page never shows: inferred type, MUT without an initializer
UNIT:      man-topic:variable
PAGE:      variable (LET and MUT)
CATEGORY:  coverage
CLAIM:     Every example on the page writes `LET name AS Type = value`; nothing says the `AS Type` may be omitted or that a MUT may start with no value.
VERDICT:   missing
EVIDENCE:  `p2_infer.mfb` (`LET x = 5`, `MUT y = "s"`) builds and prints `5st`. `p4_noinit.mfb` (`MUT x AS Integer` then `x = 6`) builds and prints `6`. `p17_let_noinit.mfb` (`LET x AS Integer`) → `error[2-203-0035 TYPE_LET_REQUIRES_VALUE]: immutable binding must have an initializer`. Sibling `mfb man types` already uses the inferred form freely (`LET nums = [1, 2, 3]`, `LET origin = Point[x := 0, y := 0]`, `LET c = Color.Green`), so the two pages show different declaration syntax without the owner of the concept explaining it.
SUGGESTED: In "LET and MUT": "The `AS Type` is optional when the value makes the type obvious: `LET x = 5`. A LET must be given its value on the line that declares it; a MUT may be declared bare (`MUT total AS Integer`) and starts at the type's default, and an empty list or map still needs the type spelled."

### 3. Top-level variables are the shared state the page's title denies
UNIT:      man-topic:variable
PAGE:      variable (title line, Description, LET and MUT)
CATEGORY:  overview-mismatch
CLAIM:     "What a variable holds, when it can change, and the one thing that is shared" and "A RES handle is the exception … Everything else follows from those two."
VERDICT:   misleading
EVIDENCE:  `p5_toplevel_mut.mfb`: top-level `MUT counter AS Integer = 0`, two functions call `bump()` which does `counter = counter + 1`, `main` prints `2`. A top-level MUT is a single cell every function in the file sees and changes — shared mutable state, which is the second thing a developer looking for "where do I keep state across calls" needs. The page mentions top-level bindings only inside the shadowing rule and never says they exist, that MUT ones can be changed from any function, or that each thread gets its own copy (`mfb man thread`: "It gets its own copy of the top-level state of the project").
SUGGESTED: Add a short section "Top-level bindings" after "LET and MUT": a LET or MUT outside any FUNC is visible to every function in the file (PUBLIC: in other files); a top-level MUT is one cell that every function reads and assigns; a thread started with thread::start gets its own copy of it (link `mfb man thread`). Reword the title line so "the one thing that is shared" does not read as "nothing but a handle is ever shared", e.g. "…and the one kind of value two names can share".

### 4. Parameters cannot be assigned, and the example hides it
UNIT:      man-topic:variable
PAGE:      variable (Every value is independent)
CATEGORY:  coverage
CLAIM:     The `bump` example copies its parameter into `MUT local AS Integer = n` before changing it, with no sentence saying why.
VERDICT:   missing
EVIDENCE:  `p18_param_assign.mfb` (`FUNC bump(n AS Integer)` with `n = n + 1`) → `error[2-203-0048 TYPE_ASSIGN_REQUIRES_MUT]: assignment target must be mutable — Binding `n` is immutable and cannot be assigned.` The same code applies to a LET (`p3_reassign_let.mfb` gives the identical message). The page never states either: that a LET cannot be assigned, or that a parameter is a LET.
SUGGESTED: One sentence before the `bump` example: "A parameter is a LET: it cannot be assigned inside the function (TYPE_ASSIGN_REQUIRES_MUT), so copy it into a MUT when you need to change it, as bump does." And in "LET and MUT": "Assigning to a LET is refused with the same error."

### 5. "There is no way … that changes a List its caller can see" contradicts the lambda page's forEach exception
UNIT:      man-topic:variable
PAGE:      variable (Every value is independent) vs lambda (Capture rules)
CATEGORY:  consistency
CLAIM:     "There is no way to write a function in MFBASIC that changes a List its caller can see."
VERDICT:   misleading
EVIDENCE:  `mfb man lambda` §"The non-escaping callback exception": a lambda passed to collections::forEach "may mutate an outer MUT binding directly … Mutating a captured MUT List or MUT Map works the same way, and the update is reflected in the outer binding after the call." Probe `p11_lambda_mut_list.mfb`: `MUT xs = []`, `collections::forEach([1,2,3], LAMBDA(v AS Integer) -> xs = collections::append(xs, v))`, prints `3`. The lambda is a function value that changes a list the enclosing scope sees; the two topics describe the same fact as "no way" and "the one exception".
SUGGESTED: "No function can change a List through its parameters: what it receives is its own copy. The one route to an outer MUT list is a lambda handed straight to collections::forEach, which mfb man lambda describes." Add `mfb man lambda` to "Where to look next" (lambda already links back to variable twice).

### 6. Returning a handle is the common case and the page reads as if RETURN closes it
UNIT:      man-topic:variable
PAGE:      variable (Handles close themselves / Where a handle can live)
CATEGORY:  coverage
CLAIM:     "A handle closes when its binding's scope ends — on every way out, including a RETURN and a failure" and "Handing a handle to another thread is the one place they do not."
VERDICT:   misleading
EVIDENCE:  `p23_return_res.mfb`: `FUNC openLog() AS fs::File` binds `RES f`, writes, `RETURN f`; `main` binds `RES g = openLog()`, writes again, closes, reads back `opened` / `still open`. Returning the handle hands it out exactly like `RES g = f` does, and nothing closes it at the RETURN. The page shows a handle passed in, moved by `RES g = f`, stored in a record, a list and a thread, but never the helper-that-opens-and-returns pattern, and its wording ("every way out, including a RETURN") says the opposite of what p23 does.
SUGGESTED: After the `RES g = f` paragraph: "RETURN f does the same hand-over: the function's name ends, the caller's RES binding takes the one open thing, and it closes when that binding's scope ends." Amend "every way out" to "every way out that does not RETURN the handle itself".

### 7. "Where to look next" points at a scope page that does not exist, and misses the two siblings that depend on this page
UNIT:      man-topic:variable
PAGE:      variable (Where to look next)
CATEGORY:  discoverability
CLAIM:     "mfb man flow — scopes, and the ways out of one."
VERDICT:   wrong
EVIDENCE:  `mfb man flow --all | grep -n -i scope` hits only lines 29–30 (the os::exit sentence: "runs lexical cleanup for every live scope"). flow's subtopic list is if / for / forEach / while / do / match / pipeline; it has no section on scopes or on what ends one. Meanwhile `mfb man lambda` says twice "mfb man variable explains copies / aliases" and `mfb man thread` builds on top-level copies and RES aliasing, and neither is listed here.
SUGGESTED: Replace the flow bullet with what flow actually offers ("mfb man flow — the block forms a LET or MUT can be declared inside, and EXIT/CONTINUE/os::exit, which all run the cleanup this page describes"), and add "mfb man lambda — what a lambda copies, what it aliases, and the forEach exception" and "mfb man thread — each thread's own copy of top-level state; handing a handle across".

Checked and found consistent, for the audit trail: record-with-RES-field copy is an alias and closing through either name closes both (`p6`, `p27` → `7-703-0004 Resource handle is already closed`); `RES g = f` and use-after-close are compile-time `TYPE_USE_AFTER_MOVE` (`p7`, `p12`); LET of a resource is `TYPE_RESOURCE_REQUIRES_RES` (`p13`); STATE rejects a non-record and a record with a RES field with `TYPE_STATE_INVALID` and starts at defaults with whole and per-field assignment working (`p8`, `p9`, `p15`); record field assignment is refused with the WITH hint (`p10`); the top-level shadow message matches the page (`p16`); all seven cross-references render (`mfb man thread transfer`, `fs types`, `tcp types`, `types`, `flow`, `thread`, `mfb spec memory`); the types, thread and fs types pages describe RES aliasing and record/list holding in the same words as this page.
