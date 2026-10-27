### 1. Dangling cross-reference to a spec topic that does not exist
UNIT:      man-topic:tour
PAGE:      tour typescript, tour python ("Where to go next")
CATEGORY:  discoverability
CLAIM:     "`mfb spec language type-inference` — how far inference goes without annotations."
VERDICT:   wrong
EVIDENCE:  `mfb spec language type-inference` prints `error: unknown topic `type-inference` in spec `language``. The subtopic table from `mfb spec language` lists design-principles, lexical-structure, templates, types, bindings-and-scope, functions, error-model, pattern-matching, control-flow, operators, collections, modules-and-packages, memory-semantics, resource-management, ... and no inference topic. §9.1 of `.ai/man-content.md` requires every cross-reference to render.
SUGGESTED: Point at a topic that exists and covers inference (`mfb spec language bindings-and-scope`, whose summary is "Types are inferred from the initializer", or `mfb spec language types`), or drop the bullet. Apply to both pages.

### 2. The same "Where to go next" bullet in two versions, one of them cut off
UNIT:      man-topic:tour
PAGE:      tour typescript, tour python versus tour c, tour java, tour go
CATEGORY:  consistency
CLAIM:     C/Java/Go: "`mfb man variable` — values, copies and handles; `mfb spec language memory-semantics` has the exact contract." TypeScript/Python: "`mfb man variable` — values, copies and handles; `mfb spec language memory-semantics`" (sentence ends there).
VERDICT:   inconsistent
EVIDENCE:  `sed -n '/Where to go next/,$p' src/docs/man/tour/0{1,2,3,4,5}_*.md`: the TS and Python files end the bullet after the spec reference with no predicate; the other three say "has the exact contract." The five pages are templated on one another and this bullet is meant to be identical.
SUGGESTED: Use the full C/Java/Go wording on the TypeScript and Python pages.

### 3. Two shared code blocks are fragments that read like programs and are never introduced as fragments
UNIT:      man-topic:tour
PAGE:      all five comparison pages ("Values and threading", "Error handling")
CATEGORY:  consistency
CLAIM:     The threading block's second half ("' main package" ... `thread::send(t, "done")` / `LET words = thread::waitFor(t)`) and the error block's tail ("' handle one call locally instead of writing a handler:" `LET port = loadPort("port.conf") TRAP(e) ... END TRAP`) are presented as code that runs as written.
VERDICT:   misleading
EVIDENCE:  Compiled verbatim at top level (worker and parent in one executable, `/tmp/plan-125-scratch/B-recheck/man-topic-tour/p_thr_frag`): `mfb build` fails at the `thread::send` line with `MFB_PARSE_UNEXPECTED_STATEMENT: Expected an IMPORT, LET, MUT, FUNC, TYPE, UNION, ENUM, RESOURCE, or LINK declaration at the top level.` The top-level `LET port = loadPort(...) TRAP(e) ... END TRAP` (`p_toplet`) fails the same way at `END TRAP`. Wrapped in `FUNC main()` both work: the thread example prints `9` (`p_thr_run`) and the loadPort example returns 8080 / 8181 as documented (`p_res`). The surrounding prose on every page never says these are fragments; §9 of the standard: "Where a block is a deliberate fragment rather than a whole program, it must be introduced as one in the surrounding prose; an unrunnable block that reads like a program is a defect."
SUGGESTED: Either wrap the parent-side statements in a `FUNC main() AS Nothing ... END FUNC` (and the inline-TRAP tail likewise, as the overview already does in its "Errors" section), or add one sentence before each block: "The parent-side lines below belong inside a FUNC; shown bare for brevity." Same fix on all five pages.

### 4. Prose names a type the code block never shows
UNIT:      man-topic:tour
PAGE:      tour typescript, tour python ("Values and threading")
CATEGORY:  consistency
CLAIM:     "`Thread OF String TO Integer` is the `postMessage` contract you wish `Worker` had" (TS) and "`Thread OF String TO Integer` declares both directions of the contract" (Python).
VERDICT:   misleading
EVIDENCE:  The block above each sentence contains only `ThreadWorker OF String TO Integer` (the worker's parameter); `Thread OF ...` appears nowhere in the block because `RES t = thread::start(...)` is inferred. `mfb man thread` explains the two names: "thread::start launches it and gives you a Thread handle; the function itself receives the matching ThreadWorker handle. Both name the same running thread from its two ends." The tour pages mention neither that there are two handle types nor which one is in the code, so a reader searching the block for the type the prose names finds nothing. The C/Java/Go pages avoid the problem by not naming the type.
SUGGESTED: "`ThreadWorker OF String TO Integer` on the worker side — and the matching `Thread OF String TO Integer` the parent gets back from `thread::start` — is the `postMessage` contract ...", or annotate the binding in the block: `RES t AS Thread OF String TO Integer = thread::start(...)` as the `thread start` page does.

### 5. The one-page tour never introduces the operators and comment syntax its own snippets use
UNIT:      man-topic:tour
PAGE:      tour (overview)
CATEGORY:  coverage
CLAIM:     "This page walks the whole language once; every heading ends with where to read more."
VERDICT:   missing
EVIDENCE:  `grep -c "'" src/docs/man/tour/package.md` shows `'` line comments in every block, `&` is used for string concatenation in Control flow, Functions and Errors, `^` in Functions, `:=` in Data, `<>`/`DIV` on the comparison pages, yet no sentence on the overview says what any of them is, and no "More:" line reaches `mfb spec language operators` (which exists and is the operators table) or `mfb man strings` / `mfb man unicode` even though Data introduces `String` and `Scalar`. A newcomer's first questions after hello-world (how do I comment, how do I join strings, is `=` comparison or assignment) have no answer in the unit; every other concept the snippets rely on gets a heading.
SUGGESTED: Add a short "Operators and comments" paragraph after Bindings or Data: `'` starts a comment; `&` joins strings; `=`/`<>` compare and `=` also assigns to a MUT; `^` is power, `DIV` is division; `:=` sets a field in a constructor or `WITH`. End it with "More: `mfb spec language operators`, `mfb man strings`, `mfb man unicode`."

What I checked and found correct: every `mfb man` and `mfb spec` reference on the six pages except `type-inference` renders (including `mfb man tour c|java|go|typescript|python`, `mfb spec language worked-example`, `memory-semantics`, `architecture`); `mfb init hello` / `mfb build hello` produce `hello/build/hello.out` as the overview says; the overview's Data, Control flow, Functions (defaults re-evaluated per call, `#1 #2 #99`), Errors (function-level and inline TRAP), Resources, and pipeline blocks all compile and print the documented results; FOR EACH over a Set and a Map works; records compare structurally (`a = b` prints `equal`); use-after-close and double-close are compile errors (`TYPE_USE_AFTER_MOVE`); `fs::open(dst, "write")` is a valid mode; `toByte(300)` and `nextId` at the 64-bit max fail with 77050010, `toInt("12,5")` with 77050003, and `percentDone(1, 0)` fails at the infinity boundary; `thread::send`/`receive` carry `timeoutMs`, the default queue bound is 64, and `waitFor` re-raises the worker's error, all as the comparison pages state.
