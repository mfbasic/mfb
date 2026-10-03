# bug-755: a user function that hands back a handle it was passed gets that handle closed at the caller's inner scope

Last updated: 2026-10-03
Effort: large — a runtime owner word on every resource record plus a spec rewrite (guess)
Severity: HIGH
Class: Correctness (resource closed without the program asking)

Status: Open — fix decided (the resource holds its scope, below); ready to implement
Regression Test: none yet — see Validation

Found by the bug-643 lane. bug-643 fixed the `collections` helpers; this is the same
adoption defect for a **user** function, which bug-643's static rule cannot see
through.

**The single correct behavior a fix produces:** a resource is closed by the
outermost scope that holds it. Binding a `RES` from a call — or receiving one as a
parameter, or `RETURN`ing one — never moves the resource's cleanup scope inward, so
an inner-scope binding of a handed-back handle exits without closing it.

## Reproduction

Minimal (measured 2026-10-03, macos-aarch64, `target/release/mfb` at `285f146e9`;
`mfb init /tmp/b755`, this as `src/main.mfb`, `mfb build . && ./build/b755.out`):

```basic
IMPORT io
IMPORT fs

FUNC ident(RES h AS fs::File) AS RES fs::File
  RETURN h
END FUNC

FUNC main() AS Nothing
  fs::writeText("/tmp/b755/data.txt", "hello\n")
  RES a AS fs::File = fs::open("/tmp/b755/data.txt", "read")
  IF TRUE THEN
    RES y AS fs::File = ident(a)
  END IF
  io::print(fs::readLine(a))
END FUNC
```

Observed: `Error: 7-703-0004` / `Resource handle is already closed.`, exit 255.
Expected: `hello`, exit 0. Control: replacing the `RES y = ident(a)` line with
`io::print("inner")` prints `inner` then `hello`, exit 0.

The collection form, and the `TRAP` observation used by the original report:

```basic
IMPORT io
IMPORT fs
IMPORT errorCode

FUNC state(RES h AS fs::File) AS String
  LET line = fs::readLine(h) TRAP(e)
    IF e.code = errorCode::ErrResourceClosed THEN
      RECOVER "CLOSED"
    END IF
    RECOVER "open"
  END TRAP
  RETURN line
END FUNC

FUNC ident(RES h AS fs::File) AS RES fs::File
  RETURN h
END FUNC

FUNC same(xs AS List OF RES fs::File) AS List OF RES fs::File
  RETURN xs
END FUNC

FUNC main() AS Nothing
  fs::writeText("/tmp/b750.txt", "")
  RES a AS fs::File = fs::open("/tmp/b750.txt", "read")
  IF TRUE THEN
    RES y AS fs::File = ident(a)
  END IF
  io::print("a: " & state(a))
  RES b AS fs::File = fs::open("/tmp/b750.txt", "read")
  LET lb AS List OF RES fs::File = [b]
  IF TRUE THEN
    LET p = same(lb)
  END IF
  io::print("b: " & state(b))
END FUNC
```

Observed (macos-aarch64, `a0082e4ae` and after bug-643): `a: CLOSED`, `b: CLOSED`.
Expected: `a: open`, `b: open`.

The same holds for a mixed return (`RETURN collections::append(xs, fs::open(p))`,
where the caller must take the fresh handle but not `xs`'s), and for a
`collections` helper over another call's result (`take(makeFiles(), 1)` keeps
adopting, by bug-643's rule, because that call's handles may be fresh).

## The model

The language model this bug violates, stated once:

1. **A resource holds its cleanup scope. A `RES` does not.** Every `RES` binding,
   `RES` parameter, record field, and collection slot is a *reference* to the one
   resource; none of them is an owner.
2. **A resource's scope only floats up, never down.** It starts at the scope that
   produced it. Adding it to a collection whose scope is further out moves it out.
   Nothing moves it in: binding a reference in an inner scope, passing it to a
   callee, or a callee handing it back leaves it where it is.
3. **`RETURN` from the owning scope releases it to the caller.** A resource produced
   inside the callee (or floated to the callee's scope) has no live owner between
   the callee's `RETURN` and the caller's binding; the caller's binding scope takes
   it. A resource the callee only received as a parameter is still owned by its
   outer scope, and `RETURN`ing it changes nothing.

Walking the repro under that model: `RES a` is produced at `main()`'s scope. `ident(a)`
passes a reference, so it is still at `main()`'s scope. `RES h` is a reference, still
at `main()`'s scope. `RETURN h` is not a return from the owning scope, so the resource
stays at `main()`'s scope, and `RES y` is one more reference. The `IF` block exits
owning nothing.

## Root cause

The compiler attaches a close to a **binding**, not to the resource.
`src/codegen/engine/control/builder_control.rs`, the `NirOp::Bind` lowering: a
resource binding whose initializer is a call registers an owning
`ActiveCleanup::Resource` at the binding's scope, and a `List OF RES` binding from a
call seeds an owned-list from every element (`emit_owned_list_seed_from_collection`),
except for the `collections` helpers bug-643 exempted statically
(`call_result_holds_only_argument_resources`). Neither knows whether the resource
already has a live owner further out. The static escape analysis
(`mfb spec architecture escape-analysis`) is per-function and syntactic, so it cannot
see that `ident` returns its parameter.

The spec currently documents the defect as intended:
`src/docs/spec/language/15_resource-management.md` §15.6 says "the caller's binding
scope **adopts** them" and "A user function's result is still adopted whole,
including handles it was passed and hands back."

## Fix — compiler

Make rule 1 literal: put the owner on the resource record and have every cleanup
consult it.

### Owner word on the resource record

- Add one u64 **owner** word to the **canonical header** every resource shares
  (`tag` `+0`, `handle` `+8`, `closed` `+16`, `state` `+24`): **owner at `+32`**, a
  new `RESOURCE_OFFSET_OWNER` beside `RESOURCE_OFFSET_STATE` in
  `src/codegen/error/constants/error_constants.rs`. The header becomes `0..40` and
  every type-specific tail starts at `+40` instead of `+32`.
  - Not the spare bits of the `closed` word: the drop and transfer paths, and the
    non-crossing handles, test that word for non-zero
    (`src/codegen/resource/mod.rs:closed_or_moved_guard`).
- **Rebase every tail on one constant.** The tail offsets are literal today
  (`FILE_OFFSET_*` in `error_constants.rs`; `PROC_STDIN_W = 32` …
  `PROC_STDERR_BUF = 88` with `assert!(PROC_STDIN_W == 32)` in
  `src/codegen/builtins/process/gen_shared.rs`; the audio `H_*`, TLS `TLS_*` /
  `REC_*`, `IMAGE_*`, and `FONT_BYTES` constants). Add a `RESOURCE_TAIL_START`
  (`= RESOURCE_OFFSET_OWNER + 8`), define each tail as `RESOURCE_TAIL_START + n`,
  and turn the `== 32` asserts into `== RESOURCE_TAIL_START`. Then sweep the
  emitted assembly and runtime helpers for raw tail offsets that bypass the
  constants (`#32`, `[x, 40]`, `32(%r…)` against a resource record); each one is a
  silent corruption after the shift.
- **Record size.** The longest tail (`Process`) ends at `+96` today, so `+104`
  after the shift; `RESOURCE_RECORD_SIZE(_BYTES)` becomes **112** (kept a 16-byte
  multiple). The existing `… + 8 <= RESOURCE_RECORD_SIZE_BYTES` asserts catch any
  tail that no longer fits.
- `0` means **unowned**: freshly produced and not yet bound, or released by a
  `RETURN` and not yet taken. A closed-default record carries `0`.
- Check before landing: every allocation of a resource record (built-in producers,
  native `LINK` resources, imported-package resources, the thread-transfer copy,
  the closed-default record) goes through the size constant, so a precompiled
  package cannot hand over a 96-byte record.

### Scope tokens

- A scope that can own a resource (any scope with a `RES` binding, an owned-list, or
  a resource-union binding) takes a **token** on entry from a per-thread counter that
  only increases. Scopes nest strictly, and a scope that exits leaves no record
  owned by it (rule below), so of two live tokens the **smaller is the outer**
  scope. No token is ever stale.
- Scopes that cannot own a resource take no token, so code with no resources pays
  nothing.
- The counter is per thread. `thread::transfer` marks the record moved, as today, and
  `thread::accept` sets the owner to the accepting scope's token.

### The operations

| Event | Owner-word rule |
| --- | --- |
| Producer (`fs::open`, a `LINK` constructor, …) | writes `0`; the binding or owned-list it lands in claims it |
| `RES` binding, `List OF RES` binding, owned-list seed | **claim**: owner `0` → set to this scope's token and register the cleanup; owner non-zero → reference only, register nothing |
| Float into an outer collection's owned-list | owner ← the collection scope's token when that is smaller (outer); otherwise unchanged |
| `RETURN` | for each returned record (each element for a collection, each `RES` field for a record) whose owner is a token of the returning function's scopes: owner ← `0`. A record owned further out is left alone |
| Call result the caller never binds | close every returned record whose owner is `0` |
| Scope exit (normal, `EXIT`, `FAIL`, `PROPAGATE`, `TRAP` routing, `EXIT PROGRAM`) | close a registered record **only if its owner equals this scope's token**; then the closed flag makes any later close a no-op, as today |
| `thread::transfer` / `thread::accept` | transfer marks moved; accept claims at the accepting scope |

Because a scope exit closes only records it currently owns, this one guard covers
the cases the codegen handles with separate mechanisms today — the per-binding
`owner_flag_slot` (bug-648), the `RETURN` pointer-identity skip
(`src/codegen/engine/control/builder_exits.rs`, `escaping_value_slot`), and the
`deactivate_resource_cleanup` of a floated `TRAP` producer. Fold each into the owner
check where it is the same decision; keep one only if a test shows it carries
something the owner word does not.

The static escape analysis stays as the optimizer: where it proves the outcome
(a binding of a producer call that is never returned, the bug-643 `collections`
exemption), codegen may skip the runtime claim test. It is no longer what makes
the program correct.

### Cost

One load, compare, and store per claim and per scope-exit close; one counter bump
per resource-owning scope entry; a `RETURN` walks only the resources it returns.
No change to how values are passed across a call, `LINK` thunks, or packages: the
information travels on the record every reference already shares.

## Fix — spec

Every change below lands in the same change set as the compiler fix (Hard
Completion Gate, `.ai/specifications.md`). The spec describes HEAD, so none of it
is written ahead of the code.

### Terminology: "reference", not "pointer"

MFBASIC has no pointers and no `NULL`. In language-level spec topics, a `RES`
binding, a `RES` parameter, a record field, and a collection slot hold a
**reference** to the resource. "Pointer" stays only where the text means a machine
pointer as an implementation detail (the 8-byte record pointer, a handle slot, a
register). Reword in the topics this fix touches:

- `src/docs/spec/language/15_resource-management.md` — "a `RES` is a pointer to
  it", "that same pointer", "Reading a pointer or passing it", "Its pointer may be
  copied", "handle pointer" (the View bullet), "a unique pointer", "Pointer alias"
  and "copying the pointer" (the binding table), the `' pointer passed` comments in
  the `exec` example, "holds a copy of the one handle pointer" (record field and
  §15.6), "a collection slot is a pointer", "a collection of pointers", "pointers to
  parent thread handles", every "pointer" in the float-up rule and its consequence
  bullets, "yield a **pointer**", and "any pointer to the resource" / "holds
  pointers to resources".
- `src/docs/spec/language/14_memory-semantics.md` — the resource cases ("Resource
  slots copy the pointer", "may hold a **pointer** to", "`RES new = old` can copy
  its pointer", "merely reading or passing a pointer"). Function-value wording in
  this topic and in `language/04_types.md` is outside this bug and is left as is.
- `src/docs/spec/language/04_types.md` — the `RES` record-field paragraph ("copies
  the pointer").
- `src/docs/spec/architecture/23_escape-analysis.md` — "hands back a pointer to a
  resource", "resource pointers", "handle pointers", "a borrowed pointer". "Record
  pointer" in the runtime-identity layer is a machine pointer and stays (that layer
  is rewritten anyway, below).

### Contract changes

- **§15 opening and the identity model**
  (`language/15_resource-management.md`): state the model above as the normative
  rule. The resource holds its cleanup scope; every `RES` is a reference; the scope
  only floats up; `RETURN` from the owning scope releases it to the caller, and
  `RETURN` of a resource owned further out does not. The "Cleanup obligation"
  bullet says the obligation belongs to the resource's scope, not to a view. Cite
  `RESOURCE_OFFSET_OWNER` and the claim helper.
- **The binding table**: add rows for `RES y = f(…)` where the call hands back a
  resource with a live owner (reference, no new cleanup) and where it hands back a
  released one (this scope takes it).
- **§15.6, returning a resource collection**: replace "the caller's binding scope
  **adopts** them" with: the caller's binding scope takes each returned resource
  that the callee released, and no other. Delete the `collections`-member exception
  paragraph as a *contract* (it becomes an optimization the escape-analysis topic
  may mention) and delete "A user function's result is still adopted whole,
  including handles it was passed and hands back."
- **§15.6, float-up rule**: keep "ownership floats up" and add that it is the only
  direction a scope moves; an inner binding or a callee never lowers it.
- **`mfb spec architecture escape-analysis`**: the three single-close layers become
  static hand-off (an optimization), the **owner word** (replacing runtime pointer
  identity as the correctness layer), and the closed/moved flag. State that the
  analysis's `Local`/`Float` decisions are what codegen may skip the runtime claim
  for, not what decides ownership.
- **`mfb spec memory heap-values`** (Resource Record): 112 bytes; the canonical
  header is `0..40` with the owner word at `+32`, `0` = unowned; the type-specific
  tail starts at `+40`, so every per-kind offset table (File/Socket/Audio/Native,
  the TLS table) shifts by 8; the per-thread scope token and the "smaller live
  token is outer" invariant. Update every other topic that states the record size: grep
  `96-byte\|96 zeroed\|96 bytes` under `src/docs/spec` (it hits
  `memory/04_arenas.md`, `memory/08_program-startup.md`,
  `language/17_native-libraries.md`, `architecture/04_ir.md` and others) and change
  only the resource-record mentions.
- **`mfb spec threading`**: `thread::accept` claims at the accepting scope.

Verify with `./scripts/spec-census.sh --citations language architecture memory
threading` and `--links`, then `cargo build` and `cargo test --bin mfb spec`.

## Validation

RED first, under `tests/rt-behavior/resources/`, one fixture per case, each failing
at HEAD before the fix:

1. The minimal repro above (`ident` in an inner `IF`): prints `hello`.
2. A `List OF RES` handed back (`same(lb)` in an inner block): `b` still open.
3. Mixed collection return (`collections::append(xs, fs::open(p))` bound in an inner
   block): the fresh handle closes at the inner block's exit; `xs`'s handles do not.
4. Mixed single return (`IF c THEN RETURN h ELSE RETURN fs::open(p)`), both
   branches: the fresh handle is closed by the binding's scope, the passed one by its
   owner.
5. Discarded call result (`ident(a)` as a statement; `fs::open(p)` returned and
   ignored): `a` stays open; the fresh handle is closed, with `alloc_calls =
   free_calls`.
6. Float-up across a call: a handle passed to a function that adds it to a
   collection the caller declared further out closes once, at that outer scope.
7. A record with a `RES` field handed back from a user function, bound in an inner
   block: the field's resource stays open.
8. `thread::transfer` then `thread::accept` inside an inner scope of the worker:
   closed by the accepting scope, once.

Then the existing resource fixtures and the full suite must stay green, and every
golden that changes must be explained by the header shift, the record-size change,
or the owner-word code before it is re-baselined (AGENTS.md, "Never edit a
test/golden to pass"). The header shift touches every resource kind's tail, so run
the per-kind runtime suites (File buffering, process, audio, TLS on each backend,
canvas `Image`/`Font`) on each target, not only macos-aarch64.
