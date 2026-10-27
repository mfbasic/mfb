### 1. "declared once, whatever block it sits in" is false: sibling and later blocks may reuse a name
UNIT:      man-topic-page:variable/package
CLAIM:     "Within one function a name is declared once, whatever block it sits in: a second `LET` or `MUT` of the same name inside an `IF`, `FOR` or `WHILE` body is rejected (`SYMBOL_DUPLICATE_LOCAL`), not a new inner variable."
VERDICT:   wrong
EVIDENCE:  `p/q06b_sibling_blocks.mfb` declares `LET inner AS Integer = 5` in one `IF` body and `LET inner AS Integer = 7` in a second, sibling `IF` body; it builds and prints `13`. `p/t06_after_block.mfb` declares `inner` inside an `IF` body and again at function level after `END IF`; it builds and prints `13`. Only `p/p06_dup_local.mfb` (a `MUT n` at function level, then `LET n` inside an `IF` nested within its scope) is rejected with `SYMBOL_DUPLICATE_LOCAL`. The rule the binary enforces is "a name still in scope cannot be declared again", not "declared once per function". (Built with /Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb via `run.sh`.)
SUGGESTED: "A block does not open a new namespace for a name that is already in scope: declaring a second `LET` or `MUT` with the same name inside an `IF`, `FOR` or `WHILE` body while the outer one is still visible is rejected (`SYMBOL_DUPLICATE_LOCAL`), not a new inner variable. Once a block has ended, its names are gone and may be declared again. A `MUT` declared outside a block may be assigned from inside it."

### 2. A thread does not get a copy of the current top-level state; it starts from the declared initial values
UNIT:      man-topic-page:variable/package
CLAIM:     "A thread started with `thread::start` gets its own copy of the top-level state instead (see `mfb man thread`)."
VERDICT:   misleading
EVIDENCE:  `p/r21_thread_copy.mfb`: top-level `MUT shared AS Integer = 0`; `main` sets `shared = 5`, then starts an `ISOLATED FUNC peek` that does `shared = shared + 100` and returns it; `thread::waitFor` prints `worker saw 100, main has 5`. The worker's `shared` began at the initializer `0`, not at a copy of main's current `5`. "Its own copy of the top-level state" reads as a snapshot of the starter's values, which is not what happens. (`mfb man thread` line 14 uses the same wording; that page is a separate unit but should agree.)
SUGGESTED: "A thread started with `thread::start` does not see this variable at all: it starts with its own fresh top-level state, every binding at its declared initial value, and nothing it assigns is visible to the thread that started it (see `mfb man thread`)."

### 3. "An empty list or map still needs its type spelled" — the binary does not enforce it (bug-882)
UNIT:      man-topic-page:variable/package
CLAIM:     "An empty list or map still needs its type spelled."
VERDICT:   misleading
EVIDENCE:  `p/r04b_let_empty.mfb` (`LET xs = []` then `io::print(toString(len(xs)))`) builds and prints `0`. `p/s04c.mfb` (`MUT xs = []` then two `collections::append(xs, n)`) fails not with a developer diagnostic but with the internal error `error: native collection list item must be Unknown, got Integer while lowering assign xs`. The intended contract is the one the page states; the binary silently accepts the binding and crashes later. This is already filed as `bugs/bug-882-unannotated-empty-list-binding-internal-error.md` (row in `bugs/status.md`, LOW). Documentation task: no code touched; the page should stay on the contract but not imply the compiler points out the omission today.
SUGGESTED: Keep the sentence, and make the consequence explicit so the reader does not wait for a diagnostic: "An empty list or map still needs its type spelled — `MUT xs AS List OF Integer = []` — because `[]` on its own says nothing about what it will hold."

### 4. Payload-rebuild mechanics are a compiler fact, not a developer fact
UNIT:      man-topic-page:variable/package
CLAIM:     "Updating a field writes it where it lies, without rebuilding the payload, when the field is a scalar, a collection changed by a builtin such as `collections::append`, or a fixed-size record such as `vector::Float3`; a field whose size depends on its value, such as a `String`, is rebuilt."
VERDICT:   out-of-scope
EVIDENCE:  `p/q17c_state_whole_and_alias.mfb` updates `f.state.pos` (scalar) and `f.state.label` (String, grown to a longer value) and reads both back through a parameter alias: `after: 13 [longer label]`. There is no observable difference between the "written where it lies" and the "rebuilt" case; the sentence describes storage layout (`.ai/man-content.md` §1 and §3: a sentence that only matters to someone reading compiler source does not belong on the page). The rest of the paragraph (`f.state = value` replaces the whole payload, `f.state.field = value` updates one field, default start, travels with the handle) is verified by the same probe.
SUGGESTED: Delete the sentence. If the point is cost, it is a candidate for `mfb spec memory` / `planning/plan-125-belongs-in-spec.md`, not this page.

### 5. "the second one has exactly one exception" names the wrong idea
UNIT:      man-topic-page:variable/package
CLAIM:     "There are only two ideas, and the second one has exactly one exception:"
VERDICT:   misleading
EVIDENCE:  Read against the list that follows it: idea 1 is "every value is independent"; idea 2 is "A RES handle is the exception." The exception belongs to the first idea; as written the sentence says the exception itself has an exception, and nothing in the list supplies one.
SUGGESTED: "There are only two ideas, and the second is the one exception to the first:"

---

Everything else on the page checked out. All 12 fenced blocks were extracted from the rendered page (`mfb man variable --all`, the P-125 binary) into `/tmp/plan-125-scratch/A-recheck-iter2/man-topic-page-variable-package/ex/ex01..12.mfb`, staged into an `mfb init` project and built and run; 11 produced exactly the output the page shows and block 2 fails to build with `SYMBOL_SHADOWS_TOP_LEVEL_BINDING: `limit` is already a top-level binding`, as the page says. Probes under `p/` confirmed: `LET x = 5` inference and bare `MUT total AS Integer` defaults (`5|0|[]|FALSE|0.00`); bare `LET x AS Integer` rejected (`TYPE_LET_REQUIRES_VALUE`); assigning a `LET` and assigning a parameter both `TYPE_ASSIGN_REQUIRES_MUT`; `q.x = 50` rejected with the WITH hint; top-level `MUT` shared by two functions (`2`) and across files via `PUBLIC` (`2`); `RES g = f` then using `f` → `TYPE_USE_AFTER_MOVE`; `RETURN f` hand-over writes through the caller's binding; use after `fs::close` refused at compile time, and through a parameter alias raised at runtime as `Resource handle is already closed.` (`errorCode::ErrResourceClosed`); a record copy's `RES` field is an alias and closing via one record closes it for the copy; the named `LogFile[name := …, handle := f]` form also works; bare `List OF fs::File` → `TYPE_RESOURCE_REQUIRES_RES`; `STATE Integer` and a `STATE` record with a `RES` field both `TYPE_STATE_INVALID`; `/dev/fd` counts (`fs::listDirectory`) go 4 → 6 → 4 around a scope holding a `List OF RES fs::File` and 4 → 5 → 4 around a scope that fails mid-way, so handles close at scope end including on failure; `collections::forEach` lambda appending to an outer `MUT` list prints `3`. Every cross-reference (`types`, `flow`, `lambda`, `thread`, `thread transfer`, `fs types`, `tcp types`, `spec memory`) renders. `scripts/man-census.sh --memory-scope variable` and `--scope variable` both report 0 hits.
