# plan-53-B: `BIND STATE` — populate a native resource's STATE from an OUT struct

Status: **LANDED + runtime-proven against libsndfile.** `BIND STATE file = info`
parses, and the producing thunk marshals the OUT struct into the resource record's
STATE@16 (reusing `marshal_struct_out`, superseding 53-A's default-init). Proven
end-to-end: `sf_open` on a real WAV, read as `.state` →
`samplerate=8000 / channels=1 / frames=4` (the actual file metadata, not zeros).
libsnd's `openFile`/`closeFile` compile as written (the only non-STATE gap was
`closeFile`'s `ABI (sndfile CPtr)` omitting its native return `AS status CInt32` —
a pre-existing grammar rule, not a STATE issue). Artifact gate 0 diffs / 1145
goldens; 981 acceptance + 2901 unit green.

Fixtures: `tests/syntax/native/native-bind-state-valid` (a package with the libsnd
`openFile`/`closeFile` shape — builds to `.mfp` with no library present, since a
package dlopen's its native lib at run time). The runtime proof uses the vendored
libsndfile and is reproducible but not an in-tree CI fixture (no tests/ fixture
vendors libsndfile; libsnd is validated out-of-tree). The BIND STATE codegen reuses
`marshal_struct_out`, which is already proven by `getFormat`/`getFormats`.

**Remaining (guardrails):** reject a `BIND STATE` whose OUT struct's `CSTRUCT AS S`
disagrees with the resource's declared STATE type, and reject `BIND STATE` naming a
non-OUT slot (the codegen currently errors at lowering for the latter). Syntax
fixtures for those. Tracked in the plan phases below.

Last updated: 2026-07-17
Effort: medium (1h–2h)
Depends on: plan-53-A (the record representation — there is no STATE slot to write
without it)

Adds the `BIND STATE <resource-slot> = <out-struct-slot>` clause to a native `LINK`
function. It marshals the C struct the native call filled through an `OUT`
parameter into the returned resource's STATE payload — the mechanism `sf_open`
needs to hand back its `SF_INFO` as the `SNDFILE*`'s state.

The single outcome: **`FUNC openFile(path) AS RES SoundFile STATE FileInfo` with
`BIND STATE file = info` (where `ABI (… info OUT SfFileInfo)` and `CSTRUCT
SfFileInfo AS FileInfo`) returns a resource whose `.state` reads the VALUES the
native call wrote into `info` — not the default zeros plan-53-A produced.**

References:

- `plan-53-A` §4.2 — the producing thunk this clause hooks into. **Read first.**
- The existing struct→record marshalling this reuses: `marshal_struct_out`
  (`src/target/shared/code/link_thunk.rs:1642-1875`), already exercised by
  `getFormat`'s INOUT `RETURN info` (`:738-759`). It resolves `CSTRUCT … AS T`,
  arena-allocates the `T` record, writes each field from the post-call C buffer
  (inlining Strings per bug-255), and leaves the record pointer in
  `RESULT_VALUE_REGISTER`.
- The sibling clause: `BIND IN` (`marshal_struct_in`, `link_thunk.rs:1518-1613`),
  parsed by `parse_bind_in` (`src/ast/items.rs:915-980`). `BIND IN` runs *before*
  the call on the input buffer; `BIND STATE` runs *after* on the OUT buffer.
- `bindings/libsnd/src/lib.mfb:113` — `BIND STATE file = info`.

## 1. Goal

- Parse `BIND STATE <res-slot> = <out-struct-slot>` in a native FUNC body — a
  single-line clause (unlike the `BIND IN … END BIND` block).
- `<res-slot>` names the native return slot (the resource); `<out-struct-slot>`
  names an `OUT` ABI slot whose C type is a `CSTRUCT … AS S` where `S` is the
  resource's declared STATE type.
- Codegen: after the native call, run `marshal_struct_out` on the OUT struct to
  produce an `S` record pointer, and store it at `FILE_OFFSET_STATE` of the
  resource record (replacing plan-53-A's default-init — the null-check in
  `emit_resource_state_init`-style logic means the populated pointer is kept).
- The consumer reads the real values: `pos`/`samplerate`/etc. from the native
  struct, not zeros.

### Non-goals (explicit constraints)

- **The record representation** — plan-53-A. This sub-plan assumes an 80-byte
  record with a STATE slot exists.
- **`BIND IN`** — unchanged. `BIND STATE` is a new, separate clause.
- **STATE type agreement** between `<out-struct-slot>`'s `CSTRUCT AS S` and the
  resource's declared `STATE S` — validate they name the same `S`
  (`NATIVE_BIND_STATE_MISMATCH` or reuse `NATIVE_STRUCT_FIELD_MISMATCH`), but the
  cross-declaration STATE agreement for the resource type is plan-53-A's.
- **Multiple `BIND STATE` per func** — reject more than one (a resource has one
  STATE). One clause per native func.

## 2. Current State

- `parse_bind_in` (`src/ast/items.rs:918-927`) hard-rejects anything after `BIND`
  that is not `IN`: "BIND requires a direction: `BIND IN <slot>`." So `BIND STATE`
  is a parse error today (verified — libsnd fails here).
- The native FUNC body loop (`items.rs:854-860`) dispatches `BIND` to
  `parse_bind_in`. `BIND STATE` needs a branch here.
- `LinkFunction.bind_in: Vec<BindIn>` (`src/ast/types.rs:385-397`) — a `bind_state:
  Option<BindState>` field is added beside it.
- `marshal_struct_out` (`link_thunk.rs:1642-1875`) already produces exactly the
  artifact needed: an arena `S` record pointer in `RESULT_VALUE_REGISTER`. It is
  invoked for a `RETURN <struct-slot>`; `BIND STATE` invokes the same helper but
  routes the result to `FILE_OFFSET_STATE` instead of the function return.
- plan-53-A's producing thunk (§4.2 step 4) default-inits the STATE. `BIND STATE`
  supersedes that default for the func that declares it.

## 3. Design Overview

**(a) Parse.** In the native FUNC body loop, when `BIND` is followed by `STATE`
(not `IN`), parse `BIND STATE <res-ident> = <struct-ident>` — one line, no `END
BIND`. Store as `LinkFunction.bind_state: Option<BindState { resource_slot,
struct_slot, line }>`. Reject a second `BIND STATE`.

**(b) Validate.** `<struct-slot>` must be an `OUT` ABI slot whose C type is a
`CSTRUCT … AS S`; `<res-slot>` must be the native return slot; `S` must equal the
resource's declared STATE type. Front-end checks (resolver/verify).

**(c) Codegen.** In `lower_link_thunk`, after the native call and after plan-53-A
allocated the record and stored the handle: if `bind_state` is set, call
`marshal_struct_out` on the OUT struct slot (yielding the `S` record pointer),
then `store_u64(that, rec, FILE_OFFSET_STATE)` — **instead of** plan-53-A's
default-init for this func. This is a near-copy of the `RETURN <struct>` path
(`link_thunk.rs:738-759`) with the destination changed from the function return to
the record's STATE slot.

**Correctness risk:** ordering and register lifetime. `marshal_struct_out`
allocates (clobbers caller-saved); the record pointer must be spilled and reloaded
across it (`.ai/compiler.md`). And `BIND STATE` must run after the OUT buffer is
filled (post-call) but the record `rec` must already exist (plan-53-A allocated it
post-call too) — sequence them within the same post-call block.

**Rejected: a new marshalling path.** `marshal_struct_out` is complete and
bug-fixed (bug-255 String inlining). Reuse it; only the destination differs.

## 4. Detailed Design

Grammar addition (`./mfb spec language grammar` §17):

```
nativeFuncBody += bindState
bindState      = "BIND" "STATE" ident "=" ident ;   (* <res-slot> = <out-struct-slot> *)
```

Codegen sequence in the producing thunk (extends plan-53-A §4.2):

```
; ... native call done, success gated, rec = arena_alloc(80), handle stored at FD@0,
;     CLOSED@8 zeroed, buffer words zeroed ...
if bind_state:
    spill rec
    state_ptr = marshal_struct_out(<out-struct-slot>, S)   ; existing helper → RESULT_VALUE_REGISTER
    reload rec
    store_u64(state_ptr, rec, FILE_OFFSET_STATE)
else:
    ; plan-53-A default-init at FILE_OFFSET_STATE
return rec
```

## Compatibility / Format Impact

- **Grammar:** one new clause. Additive; no existing native func changes.
- **`.mfp`:** none beyond plan-53-A (the STATE type already rides kind-11).
- **Goldens:** the libsnd-shape fixture gains its `BIND STATE`; new fixtures only.

## Phases

### Phase 1 — parse `BIND STATE`

- [ ] `src/ast/items.rs:854-860` — branch `BIND STATE` vs `BIND IN`; add
      `parse_bind_state` (single-line `<res> = <struct>`). Reject a second one.
- [ ] `src/ast/types.rs` — `BindState` struct + `LinkFunction.bind_state`.
- [ ] `src/ir/link.rs` + `src/ir/lower.rs` — carry `bind_state` into `IrLinkFunction`.
- [ ] Tests: `tests/syntax/resources/native-bind-state-*` — parse valid; reject
      double `BIND STATE`, `BIND STATE` naming a non-OUT slot, and a `CSTRUCT AS`
      type disagreeing with the declared STATE type.

Acceptance: `BIND STATE file = info` parses into AST/IR; the invalid shapes are
rejected with a named diagnostic.
Commit: —

### Phase 2 — codegen marshals into the STATE slot

- [ ] `src/target/shared/code/link_thunk.rs` — when `bind_state` is set, call
      `marshal_struct_out` on the OUT slot and store the result at
      `FILE_OFFSET_STATE` of the record (§4), superseding plan-53-A's default-init
      for that func. Spill/reload `rec` across the marshalling alloc.
- [ ] Tests: `tests/rt-behavior/resources/native-bind-state-rt` — a native open
      that fills an OUT struct and binds it as STATE; the consumer reads the real
      values (not zeros).

Acceptance: the runtime fixture prints the values the native call wrote into the
OUT struct (e.g. via a `sqlite3` call that populates an out struct, or the libsnd
shape once its library is available), NOT the plan-53-A defaults. `pos=<real>`, not
`pos=0`.
Commit: —

## Validation Plan

- Tests: the parse/reject fixtures (Phase 1); the runtime marshal fixture (Phase 2).
- Runtime proof: **required.** A build assertion proves the clause is accepted; only
  running proves the OUT struct's values reached `.state`. Distinguish from
  plan-53-A's default (zeros) by writing non-zero values into the OUT struct.
- Doc sync: `./mfb spec language grammar` §17 (the `bindState` production),
  `./mfb spec language native-libraries` (BIND STATE semantics), diagnostics
  registry for any new code.
- Acceptance: `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, `cargo test --bin mfb`.

## Open Decisions

- **New diagnostic vs. reuse.** A `BIND STATE` whose struct type disagrees with the
  declared STATE — new `NATIVE_BIND_STATE_MISMATCH`, or reuse
  `NATIVE_STRUCT_FIELD_MISMATCH` (`2-203-0127`)? Recommend a new code: the failure
  is a STATE/struct disagreement, not a CSTRUCT/record field disagreement, and a
  distinct message helps the binding author.

## Summary

The marshalling `BIND STATE` needs is already written and battle-tested
(`marshal_struct_out`, via `getFormat`); this sub-plan is parse + route-the-result.
The only real care is register lifetime across the marshalling alloc and sequencing
it into plan-53-A's post-call record-build. With A's record and B's population,
libsnd's `openFile` hands back a native handle carrying its `SF_INFO`.
