# plan-74: STATE on a resource union

Last updated: 2026-08-01
Effort: large (3h–1d)

Allow a resource UNION to carry a single, uniform `STATE` payload, lifting the blanket
`TYPE_UNION_STATE_FORBIDDEN` (2-203-0088). Today a resource union is atomic-but-stateless:
`RES s AS Stream STATE PendingState` is rejected at the binding and the return. This plan
makes a resource union carry STATE exactly like a concrete stateful resource does — the
STATE type is declared once at the use site (binding / parameter / return), is the same
whichever variant is active, and rides the active variant's 80-byte resource record.

The single behavioral outcome a correct implementation produces: a program can bind
`RES s AS Stream STATE PendingState = <producer>`, read and write `s.state.<field>` through
the union value and through a `RES … AS Stream STATE PendingState` parameter (mutations
visible to the owner), match the union and reach `.state` on the extracted variant, and
have the STATE freed exactly once on drop — with no leak, no double-free, and no change to
any program that does not attach STATE to a union.

This is the language capability that makes a bundled *and* URL-transparent non-blocking
HTTP handle expressible (`UNION Stream { net::Socket net::TlsSocket }` carrying a
`PendingState` buffer). That consumer is out of scope here; this plan ships the language
feature and its own tests.

References (read first):

- `mfb spec language resource-management` §15.5 (`src/docs/spec/language/15_resource-management.md`)
  — STATE semantics; the "resource union carries no STATE" sentence this plan amends, and
  the "STATE type fixed at the owning binding, no runtime tag" rule this design rests on.
- `planning/plan-13-A-resource-union-params.md` — the precedent language amendment for
  resource unions (variant→union widening in a parameter). Mirror its rigor on
  directionality and its spec-first ordering.
- `src/builtins/resource.rs:254` (`split_state_clause`), `:265` (`base_resource_name`),
  `:273` (`state_type_name`) — the STATE type-string plumbing (already base-agnostic).
- `src/ir/verify/ops.rs:231` and `src/ir/verify/calls.rs:281` — the two ban sites.
- `src/target/shared/code/builder_values.rs:1250` (`UnionWrap`), `:1362` (`UnionExtract`)
  — the resource-union runtime layout.
- `bugs/bug-424-resource-state-mutation-rebuilds-whole-record.md` — a SEPARATE performance
  defect (STATE mutation rebuilds the whole record). See Prerequisites: not a precondition
  of this plan.

## Prerequisites

None. This plan builds and is fully testable on its own with a scalar STATE record; it does
not depend on any other plan or bug.

Explicitly **not** prerequisites (do not braid them in):

| Not required | Why it is independent | Command |
|---|---|---|
| bug-424 (STATE mutation is O(n²)) | This plan makes union STATE *work*; bug-424 makes STATE *mutation fast*. A scalar-STATE fixture proves this plan with no perf dependency. The http consumer needs both, but this feature does not. | `ls bugs/bug-424-*` |
| plan-13-A (variant→union widening in a param) | This plan's param tests pass a *union* into a *union* parameter (exact-type match), never a concrete variant into a union param. 13-A only opens the concrete→union widening. | `ls planning/plan-13-A-*` |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run the
> ban-site and touchpoint greps before starting and again before stopping; this family's
> line numbers rot. Re-locate by symbol, not line.

## 1. Goal

- A `RES` binding, parameter, and return may name a **resource union with STATE**:
  `RES s AS Stream STATE PendingState`, `SUB f(RES s AS Stream STATE PendingState)`,
  `FUNC g(...) AS RES Stream STATE PendingState`. All three verify and run.
- `s.state` and `s.state.field` read/write correctly through a union-typed value and
  through a union-typed `RES` parameter (a callee's write is visible to the owner — the
  §15.5 alias contract).
- `MATCH` on a stateful union binds each variant, and `.state` on the extracted variant
  reads the same payload.
- On drop (every exit path), the active variant's STATE block is freed exactly once, after
  the tag-dispatched close, with no leak and no double-free.
- `thread::transfer` of a stateful resource union carries the STATE intact to the receiver.
- The STATE type is **uniform**: it is the type declared at the use site, independent of the
  runtime variant, and is checked for agreement (`TYPE_STATE_MISMATCH`) exactly as a
  concrete resource's STATE is.

### Non-goals (explicit constraints)

- **No per-variant STATE.** A union has one declared STATE type that applies to whichever
  variant is active. `UNION Stream { net::Socket STATE A net::TlsSocket STATE B }` is not
  introduced — the grammar has no variant-STATE slot (`UnionVariant { name, line }`,
  `src/ast/types.rs:507`) and per-variant STATE reintroduces the runtime-tag problem the
  uniform rule avoids. STATE stays a use-site annotation on the union-typed `RES`.
- **A resource union stays atomic.** It owns exactly one resource at a time (the active
  variant) plus one data STATE payload — a choice among resources, not a bundle of them.
  `TYPE_MIXED_RESOURCE_UNION` and "no data variant in a resource union" are unchanged.
- **Close-exactly-once and drop ordering are preserved.** Close (tag-dispatched to the
  active variant's registered close op) still runs before the STATE free; a re-close is
  still a defined no-op.
- **No type-string encoding change.** `state_type_name`/`base_resource_name` already parse
  `Stream STATE PendingState` (§2 verified). The `.mfp` RESOURCE_TABLE / STATE encoding is
  untouched.
- **No new syntax.** `RES … AS <Union> STATE <T>` uses the existing `parse_optional_state`.
- **No change to concrete-resource STATE, to stateless unions, or to any program that does
  not attach STATE to a union.** This plan only *accepts and lowers* a case previously
  rejected.
- **No `http`/consumer code.** If a phase needs to mention `http`, the scope is wrong.

## 2. Current State

A resource union value is a small `{ tag @0, resource-ptr @8 }` block, **not** a resource
record. A concrete `RES` value's `location` IS the 80-byte record (STATE pointer at
`FILE_OFFSET_STATE = 16`, `src/target/shared/code/error_constants.rs:812`;
`RESOURCE_RECORD_SIZE_BYTES = 80`, `:849`). A union value's `location` is the 16-byte
block; the real record is `*(location + 8)` and its STATE is at `+16`. So `FILE_OFFSET_STATE`
is *out of bounds of the union block* — every STATE path that dereferences
`location + 16` assumes a concrete record and must first load the variant record pointer
from `+8`. This one mismatch is the whole feature.

- **Wrap** (concrete → union): `NirValue::UnionWrap`, `builder_values.rs:1250-1361` —
  allocates 16 bytes, tag at `+0` (`:1347`), handle pointer at `+8` (`:1351`). The variant
  record's own STATE slot is untouched by the wrap.
- **Extract** (`MATCH`): `NirValue::UnionExtract`, `builder_values.rs:1362-1374` — loads the
  variant record pointer from `source.location + 8`, typed as the concrete variant. So the
  extracted value's `location` IS the real 80-byte record — `.state` on it already works via
  the concrete path, *provided the case binding's type string carries the STATE suffix*.
- **The ban** is exactly two emit sites; there is no parameter-position check at all:
  - binding: `src/ir/verify/ops.rs:231-241` (gated on explicit type + `unions.contains_key(base)`)
  - return: `src/ir/verify/calls.rs:281-294` (`check_return_state_declaration`)
- **STATE agreement** (`TYPE_STATE_MISMATCH`) is string-parse based and already fires for a
  union base: `check_binding_state_agreement` (`calls.rs:351`),
  `check_argument_state_agreement` (`calls.rs:238`), `check_thread_transfer_state`
  (`calls.rs:161`). Because every resource can carry any copyable STATE, there is no
  "does this variant support this STATE" constraint to add — the uniform-STATE rule makes
  the existing suffix-agreement check sufficient.
- **Type-string encoding** already supports a union base + STATE suffix — see Verified.

### Measured populations

| What | Count | Command |
|---|---|---|
| `TYPE_UNION_STATE_FORBIDDEN` emit sites | 2 | `rg -n 'TYPE_UNION_STATE_FORBIDDEN' src/ir/verify/{ops,calls}.rs` → ops.rs:236, calls.rs:288 |
| Parameter-position union-STATE checks | 0 | `rg -n 'union' src/ir/verify/calls.rs` in `check_argument_state_agreement` (238-267) — none |
| STATE codegen touchpoints assuming a concrete record ptr | 6 | see §3 table (a,b,c,d/e/f drop, g transfer) |
| Existing resource-union tests | 7 | `ls tests/rt-behavior/resources tests/syntax/resources \| rg -i union` |
| Ban tests to repurpose | 2 | `resource-union-state-invalid`, `resource-return-union-state-invalid` |

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| Type-string encoding permits `Stream STATE PendingState` | **CONFIRMED** | `split_state_clause` (`resource.rs:254`) splits on first `" STATE "` iff base has no space; `"Stream"` has none → `state_type_name`→`Some("PendingState")`, `base_resource_name`→`"Stream"`. Base-agnostic. |
| The ban is only 2 sites; no param-position check | **CONFIRMED** | grep + read of `check_argument_state_agreement` (`calls.rs:238-267`) |
| `MATCH` extract yields the real 80-byte record ptr | **CONFIRMED** | `UnionExtract` resource path loads `+8`, types as concrete variant (`builder_values.rs:1362-1374`) |
| Union drop/transfer currently drop STATE entirely | **CONFIRMED** | `ResourceUnionCleanup` has no `state_type` field; `emit_resource_union_cleanup_call` (`builder_resource_cleanup.rs:78-160`) never frees STATE; `copy_union_to_current_arena` (`builder_arena_transfer.rs:603-654`) raw-copies `{tag,ptr}` only |
| STATE lives in the variant record, reached via union `+8` | **CONFIRMED** | wrap stores handle at `+8`; record STATE at `+16`; union block is 16 bytes (`builder_values.rs:1299-1318`) |
| A union→union (same type) `RES` parameter is accepted today | **UNVERIFIED** — Phase 2 test | pass a `Stream` into a `RES s AS Stream` param; expected accept by exact match |
| Uniform STATE needs no runtime tag | **CONFIRMED by construction** | the STATE type is the use-site declaration, identical for every variant; nothing is read from the tag to type `.state` |

## 3. Design Overview

One conceptual change — **a resource union value carries its STATE in the active variant's
record, reached by a `+8` indirection** — applied at every STATE touchpoint, plus lifting
the two ban sites and confirming agreement. No new representation, no encoding change, no
grammar change.

**Where design uncertainty concentrates:** almost nowhere — the type-string plumbing and
agreement checks already handle a union base (Verified), and MATCH-extract already exposes
`.state`. The one open premise (union→union param acceptance) is falsified cheaply in
Phase 2.

**Where correctness risk concentrates:** **drop-free of the active variant's STATE**
(Phase 4). The union cleanup path currently frees nothing STATE-related; adding a STATE
free must (1) run after the tag-dispatched close, (2) free the *active* variant's record
STATE (not a fixed offset — via `+8`), (3) never double-free when the same resource was
transferred/closed, (4) never leak on any exit path. This is the bug-class the plan guards
with a close/free-count test, mirroring `tests/native_resource_scope_drop.rs`.

Secondary risk: **thread-transfer STATE deep-copy** (Phase 5) — the union transfer copy is
a raw `{tag,ptr}` byte copy today, so it would alias the sender's STATE payload across the
boundary (a bug-257-class UAF). The active variant's STATE must be deep-copied into the
receiver's arena.

**Rejected alternative — store STATE in the union block instead of the variant record.**
Rejected: it duplicates the STATE slot that already exists in every 80-byte record, forces
wrap/extract to move STATE across representations, and diverges the union layout from a
concrete resource for no benefit. The variant record already has the slot; use it.

**Rejected alternative — per-variant STATE types.** Rejected: no grammar slot, and it
reintroduces a runtime-tag-dependent STATE type, contradicting §15.5. Uniform STATE is the
whole reason the feature is sound.

**Rejected alternative — a tag-branch per variant at each STATE access.** Rejected as
waste: the STATE slot is at the same `+16` offset in *every* variant's record, so a single
"load variant ptr from `+8`, then read `+16`" is correct for all variants with no per-tag
branch. (The tag is still needed for close dispatch on drop, which already exists.)

## 4. Detailed Design

### 4.1 Verifier (Phase 2)

- Delete the union rejection in `ops.rs:231-241` and `calls.rs:281-294`; a union base with a
  STATE suffix is accepted at binding and return.
- Leave `check_argument_state_agreement`, `check_binding_state_agreement`, and
  `check_thread_transfer_state` as-is — they already compare the STATE suffix for a union
  base. Add a parameter-position confirmation test (the gap noted in §2) so a
  `RES p AS Stream STATE A` param handed a `Stream STATE B` is rejected `TYPE_STATE_MISMATCH`.
- `TYPE_UNION_STATE_FORBIDDEN` (2-203-0088) becomes unemitted. Per the retired-rule
  precedent, keep the rule row reserved (do not recycle the code); update
  `src/docs/spec/diagnostics/01_rule-codes.md` to mark it retired.

### 4.2 STATE access codegen (Phase 3)

Introduce one helper: given a `ValueResult`, if its `base_resource_name` is a union type,
emit "load the variant record ptr from `location + 8`" and use that as the record pointer;
otherwise use `location` directly. Apply it in:

- `.state` read — `lower_field_access`, `builder_value_semantics.rs:186-202`.
- STATE lazy-init — `emit_resource_state_init`, `builder_value_semantics.rs:10-36`.
- `StateAssign` — `builder_control.rs:564-594`.

MATCH-extract `.state` needs no codegen change; ensure the case-binding's type string
carries the STATE suffix so the concrete path (`state_type_name` non-None) is taken.

### 4.3 Drop-free codegen (Phase 4)

- Add a `state_type: Option<String>` to the resource-union cleanup record (mirror
  `ResourceCleanup.state_type`), populated at the drop-registration site
  (`builder_control.rs:291-308`) from `state_type_name(type_)`.
- In `emit_resource_union_cleanup_call` (`builder_resource_cleanup.rs:78-160`), after the
  tag-dispatched close, if `state_type` is set, load the active variant record ptr (`+8`)
  and free its STATE block via `emit_free_resource_state_block` (`:400`). Preserve the
  close-before-free ordering and the moved/closed guards so a transferred union does not
  double-free.

### 4.4 Thread-transfer (Phase 5)

- In the resource-union transfer copy (`copy_union_to_current_arena`,
  `builder_arena_transfer.rs:603-654`), after copying the `{tag,ptr}` block, deep-copy the
  active variant record's STATE into the receiver arena (reuse the concrete-resource STATE
  copy at `:459-500`), so the receiver owns an independent payload.

## Compatibility / Format Impact

- **Changed:** §15.5 gains the union-STATE rule; `TYPE_UNION_STATE_FORBIDDEN` retired;
  verifier accepts union+STATE; codegen frees/copies union STATE.
- **Unchanged:** the resource-union runtime layout (`{tag,ptr}`) and the 80-byte record;
  the `.mfp` RESOURCE_TABLE / STATE encoding; concrete-resource STATE; stateless unions;
  close-exactly-once and drop ordering; every existing program's behavior (this only
  accepts and lowers a previously-rejected case).

## Phases

> Tick `- [x]` in the same commit as the work. An unticked box means NOT DONE.

### Phase 1 — spec amendment

Land the specified rule before the code, mirroring plan-13-A.

- [x] Amend `src/docs/spec/language/15_resource-management.md`: a resource union may carry a
      STATE, declared uniformly at the use site (binding/parameter/return); the STATE type
      is independent of the active variant; agreement and drop-free behave as for a concrete
      stateful resource; per-variant STATE is not introduced. (New "A resource union may
      carry `STATE`" paragraph in §15.5; removed the "carries no `STATE`" clause.)
- [x] Update `src/docs/spec/diagnostics/01_rule-codes.md`: mark `2-203-0088` retired
      (reserved, not recycled). Also updated `src/rules/table.rs` message to the retired
      note, mirroring the `2-203-0086` precedent.

Acceptance: `mfb spec language resource-management` renders the rule and its uniform-STATE
constraint (verified); the diagnostics topic shows 2-203-0088 retired (verified);
`every_rule_is_documented_in_the_spec` passes.
Commit: —

### Phase 2 — verifier: lift the ban, confirm agreement

- [ ] Remove the union rejections at `src/ir/verify/ops.rs:231-241` and
      `src/ir/verify/calls.rs:281-294`.
- [ ] Tests: convert `tests/syntax/resources/resource-union-state-invalid` and
      `resource-return-union-state-invalid` to **valid** fixtures (the rule they guarded is
      the one being changed by Phase 1 — cite the spec amendment in the fixture). Add
      `tests/syntax/resources/resource-union-state-mismatch-invalid` proving a union param/
      binding with a disagreeing STATE type is still rejected `TYPE_STATE_MISMATCH`, and a
      union→union same-type param is accepted.

Acceptance: the accept/reject matrix is correct at `-ast -ir` — union+STATE accepted at
binding/return/param; disagreeing STATE still rejected. (Runtime lands in Phase 3.)
Commit: —

### Phase 3 — codegen: STATE access through a union

- [ ] Add the union `+8` record-ptr helper; apply in `.state` read
      (`builder_value_semantics.rs:186`), state init (`:10`), and `StateAssign`
      (`builder_control.rs:564`).
- [ ] Ensure the MATCH case-binding type carries the STATE suffix so `.state` on an
      extracted variant resolves.
- [ ] Tests: `tests/rt-behavior/resources/resource-union-state-access-valid` — bind a union
      with a **scalar** STATE record, read/write `s.state.field` through the union value and
      through a `RES … AS Stream STATE …` parameter (prove the callee's write is visible to
      the owner), and via `MATCH`. Assert the printed values.

Acceptance: the runtime fixture prints the correct pre/post-mutation values through all
three access routes.
Commit: —

### Phase 4 — codegen: drop-free the active variant's STATE (largest risk)

- [ ] Add `state_type` to the resource-union cleanup record and populate it at
      `builder_control.rs:291-308`.
- [ ] Free the active variant's STATE after close in `emit_resource_union_cleanup_call`
      (`builder_resource_cleanup.rs:78-160`), preserving close-before-free and the
      moved/closed guards.
- [ ] Tests: `tests/rt-behavior/resources/resource-union-state-drop-valid` (+ an early
      explicit-close and an error-exit path) and a close/free-count assertion in the style
      of `tests/native_resource_scope_drop.rs` proving exactly one close and one STATE free
      per resource, on every exit path.

Acceptance: the drop fixtures run clean (no leak, no double-free) and the count test shows
one close + one STATE free per exit path.
Commit: —

### Phase 5 — codegen: thread-transfer STATE deep-copy

- [ ] Deep-copy the active variant's STATE in `copy_union_to_current_arena`
      (`builder_arena_transfer.rs:603-654`), reusing the concrete-resource STATE copy.
- [ ] Tests: `tests/rt-behavior/resources/resource-union-state-transfer-valid` — transfer a
      stateful sendable resource union to a worker; assert the STATE arrives intact and the
      sender/receiver payloads are independent (no shared-arena UAF under a repeat-run loop).

Acceptance: the transfer fixture passes STATE across the boundary intact and survives a
20× repeat run with no SIGSEGV.
Commit: —

## Validation Plan

- Tests: the syntax accept/reject pair (Phase 2), and the rt-behavior access / drop /
  transfer fixtures (Phases 3–5), including negative STATE-mismatch and error-exit cases.
- Coverage check: `tests/syntax/resources/` and `tests/rt-behavior/resources/` are
  golden-backed and in the gate denominator; seed goldens for new fixtures.
- Runtime proof: the Phase 3 access fixture and the Phase 4 drop-count fixture are the
  end-to-end proof beyond compile-accept.
- Doc sync: `15_resource-management.md` (Phase 1, same change as… itself) and
  `01_rule-codes.md`; check whether `mfb spec package resource-regions` needs a note (it
  should not — encoding is unchanged).
- Acceptance: `cargo test --bin mfb`, `scripts/test-accept.sh target/debug/mfb
  target/accept-actual`, `scripts/artifact-gate.sh`.

## Open Decisions

1. **Where STATE is declared for a union** — **use-site annotation** (`RES s AS Stream
   STATE T`, recommended, reuses `parse_optional_state`, matches concrete resources) vs. a
   STATE clause on the `UNION` declaration. Recommended use-site: no grammar change, and it
   keeps STATE a property of the binding as §15.5 already specifies.
2. **Retire vs. narrow 2-203-0088** — **retire (reserve the code), recommended** vs. keep it
   emitting for a narrower case. Recommended retire: with uniform STATE there is no residual
   union-STATE case to forbid; the remaining guard is `TYPE_STATE_MISMATCH`.

## Corrections

<!-- Filled in during execution. -->

## Summary

The engineering risk is concentrated in Phase 4 (drop-free of the active variant's STATE,
tag-dispatched, ordered after close, exactly once on every exit path) and Phase 5
(thread-transfer STATE deep-copy) — both are the resource-cleanup correctness class, guarded
by close/free-count and repeat-run tests. Everything upstream is small: the type-string
plumbing and STATE-agreement checks already handle a union base, MATCH-extract already
exposes `.state`, and the access change is a single uniform `+8` indirection. Untouched: the
union and record layouts, the `.mfp` encoding, concrete-resource STATE, stateless unions,
and every existing program. Not in scope and not a prerequisite: bug-424 (STATE mutation
performance) and the http consumer that motivates the feature.
