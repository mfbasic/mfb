### 1. Thread handles are another shared exception
UNIT:      man-topic-page:variable/package
CLAIM:     “A variable holds a value, and every value is independent. Assigning or passing gives a copy. Changing one name can never change another.”
VERDICT:   wrong
EVIDENCE:  Probe assigned one `Thread` to `original` and `other`, then called `thread::waitFor` through both. It compiled; the first call printed `1`, and the second printed `Error: 7-703-0004 Resource handle is already closed.` `src/ir/verify/resources.rs:is_copyable` classifies `ThreadHandle` as non-copyable.
SUGGESTED: “Copyable values are independent: assigning or passing them gives a copy. `RES` handles and `Thread` handles name one shared open thing; see `mfb man thread`.”

### 2. STATE omits its required default value
UNIT:      man-topic-page:variable/package
CLAIM:     “That value is an ordinary copyable record, and it is the one place a field is updated by assignment rather than with WITH:”
VERDICT:   incomplete
EVIDENCE:  A probe declaring `TYPE BadState` with an `ENUM` field, then `RES f AS fs::File STATE BadState`, failed to build with `TYPE_STATE_INVALID: STATE must be a copyable, defaultable data type`. The binding check is `src/ir/verify/ops.rs`, and its defaultability predicate is `src/ir/verify/resources.rs:is_defaultable`.
SUGGESTED: “The STATE payload must be a copyable, defaultable data record. It is the one place a field is updated by assignment rather than with WITH.”

### 3. Record copies containing handles need an explicit alias warning
UNIT:      man-topic-page:variable/package
CLAIM:     “A record is a value like any other, so it is copied on assignment.”
VERDICT:   incomplete
EVIDENCE:  A probe copied `LogFile { handle AS RES fs::File }`, closed `copy.handle`, then wrote through `log.handle`; it compiled and raised `Error: 7-703-0004 Resource handle is already closed.` `src/ir/verify/resources.rs:record_fields_copyable` permits the record copy, while the contained handle remains shared.
SUGGESTED: “A record’s ordinary fields are copied on assignment. If it has a RES field, the copied record has another alias of that same open handle; closing it through either record closes it for both.”

### 4. Transfer does not close the open resource
UNIT:      man-topic-page:variable/package
CLAIM:     “`thread::transfer` takes the handle: on success the sending name cannot be used again, and the handle is closed by the call rather than at the end of its scope.”
VERDICT:   misleading
EVIDENCE:  The scratch copy of `tests/rt-behavior/native/native-resource-thread-accept-rt` built and printed `used=2000` and `movedCount=2000`: the receiver used each transferred resource successfully. Its source states that the sender is marked moved and the receiver’s scope closes the resource. `src/codegen/builtins/thread/func_accept.rs` likewise says `accept` returns the same open handle.
SUGGESTED: “`thread::transfer` takes the sending name: after success it cannot be used again. `thread::accept` receives the same open handle, which is closed there explicitly or when its new scope ends.”

### 5. Compiler-contributor cross-reference is out of scope
UNIT:      man-topic-page:variable/package
CLAIM:     “`mfb spec memory` — the internal memory model, for anyone working on the compiler itself. Nothing on this page depends on it.”
VERDICT:   out-of-scope
EVIDENCE:  `.ai/man-content.md` §1 and §3 prohibit prose intended only for compiler contributors; this sentence explicitly directs that audience to internal documentation.
SUGGESTED: Remove this bullet from the developer man page.