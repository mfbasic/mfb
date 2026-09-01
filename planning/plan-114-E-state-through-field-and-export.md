# plan-114-E: `STATE` through a record field, and exporting a resource-carrying record

Last updated: 2026-08-30
Effort: medium (1h–2h)
Depends on: plan-114-D

Letter D made `handle AS RES fs::File` legal but deferred two things: a `STATE T`
clause on the field type, and exporting such a record across a package boundary.
This letter finishes both and closes the feature out.

The `STATE` half mirrors what §15.6 already says for a collection element — *"The
STATE rides the **element**, not the binding … the uniform state type is what lets
an extracted element type `.state`"* — applied to a field. Behavioral outcome:
with `TYPE Holder { handle AS RES fs::File STATE Cursor }`, the expression
`h.handle.state` (two dots) reads the resource's `STATE` payload, and it reads the
same payload the resource's owning binding sees, because a record field holds the
same one handle pointer.

References:

- `src/docs/spec/language/15_resource-management.md` §15.5 (what `STATE` means at
  each position), §15.6 ("A resource collection element may carry `STATE`").
- `.ai/resources-packages.md` — "Collection element STATE carry
  (`List OF RES Stream STATE S`)": the census of what a `STATE`-carrying element
  costs, and the one real break it found.
- `.ai/resources-packages.md` — "Type-export closure feeds BOTH validation and
  codegen"; `.ai/specifications.md`.
- `src/types.rs:629-647` — `split_state` / `state()` / `without_state`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-114-D complete and landed | `ls planning/completed/plan-114-D-*` → one match | NOT MET |
| Working tree clean; release `mfb` built | `git status --porcelain` → empty | MET (2026-08-30) |
| No other artifact-gate / test-accept running | `pgrep -f '[a]rtifact-gate\|[t]est-accept'` → no output | MET (2026-08-30) |

If plan-114-D is not complete, this letter cannot start, full stop. Everything
below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `handle AS RES fs::File STATE Cursor` is a legal field declaration, replacing the
  "not yet supported" rejection letter D installed.
- `h.handle.state` reads the payload; `h.handle.state.pos` reads a field of it.
- The payload read through a record field is the **same** payload the owning `RES`
  binding sees — proven at runtime by writing through the binding and reading
  through the record copy.
- A package may `EXPORT` a record type with a `RES` field (with or without `STATE`),
  and an importer can declare, construct, and read it — replacing letter D's
  baseline fixture with a working one.
- The feature's docs are complete: spec §15/§15.6, `04_types.md` §4.2,
  `./mfb spec architecture escape-analysis`, and the `.ai/*` invariant docs.

### Non-goals (explicit constraints)

- **No `STATE` *write* through a record-field chain.** `h.handle.state.pos = 5` is
  out of scope; see §2 for why, and Open Decisions for the recommendation. Writing
  is available with no new surface via `RES f AS fs::File STATE Cursor = h.handle`
  followed by `f.state.pos = 5`, which is the existing, tested path.
- **Per-field STATE types stay uniform per field.** Exactly as for a collection
  element, the `STATE` type is the one named on the field; nothing is read from a
  runtime tag.
- No change to the resource-record header, `RESOURCE_OFFSET_STATE`, or the STATE
  rebuild path (`NirOp::StateAssign`).
- No thread-plane change: a `STATE` payload still rides the resource plane, and a
  record carrying a resource still cannot cross the data plane (`2-203-0138`).

## 2. Current State

### `.state` reading is generic — with one measured gap

`lower_field_access` (`src/codegen/memory/value/builder_value_semantics.rs:460`)
takes a `target: &NirValue`, so `h.handle.state` lowers as nested `MemberAccess`
and the inner field read happens first. Its `member == "state"` arm (`:474`) gates
on `target_value.type_.state()` and then calls `emit_resource_record_ptr` — the
same concrete-resource path a plain `RES` binding takes.

The gap is what type the inner read produces. Read `lower_field_access:504-512`:
the record branch returns the declared `field_type.clone()`. For a field declared
`RES fs::File STATE Cursor` that is `Res(Stateful { base: File, state: Cursor })`.
Now read `split_state` (`src/types.rs:629-634`) — it matches `Stateful` **only at
the top level**, so `Res(Stateful{..}).state()` is `None`, and the `.state` arm
would fall through to "record has no field 'state'".

The collection element path already solved the same problem by stripping the
marker on extraction: `.ai/resources-packages.md` records that
`general::list_element("List OF RES Socket") == "Socket"`, and that every consumer
which strips only `RES ` and keeps the remainder already carries the `STATE`
through. **So the fix is the same one: a record field read yields the `RES`-stripped
field type.** That is the single load-bearing change in this letter.

### `STATE` writing is anchored to a bare identifier

`resource.state = v` and `resource.state.field = v` are recognized by a
**token-level** pattern in `src/ast/stmt.rs:226-262`: `Identifier` `.` `state`
[`.` `Identifier`] `=`. It reads `self.peek()` and `tokens[current+1..+5]`
directly. A record-qualified target (`h.handle.state.pos =`) does not match that
shape — it is `Identifier . Identifier . state . Identifier =` — and extending the
matcher to an arbitrary chain is a parser change well beyond a field read. This is
why writing is a non-goal (§1) and why the one-line workaround is documented
instead.

### Package export

`enqueue_referenced_types` (`src/binary_repr/mod.rs:781`) closes the transitive
type set by pushing every maximal identifier substring of each field's **rendered**
type via `push_type_identifiers` (`:804`), filtered by owner-pool membership. For a
field rendered `RES fs.File STATE Cursor` that yields the candidates `RES`,
`fs.File`, `STATE`, `Cursor` — of which the `STATE` record `Cursor` is the one that
genuinely must reach the importer, and `RES`/`STATE` should be discarded by the
pool filter. Whether they are, and whether the importer needs the *resource* type
registered as well (validation and native codegen need different things — see
`.ai/resources-packages.md`, "Type-export closure feeds BOTH validation and
codegen"), is measured in Phase 1, not assumed.

### Measured populations

| What | Count | Command |
|---|---|---|
| `"state"`-keyed sites in codegen | 7 | `grep -rn '"state"' src/codegen/ --include='*.rs' \| wc -l` → `7` (one read arm in `builder_value_semantics.rs`, five in `builder_control.rs`, one in `validation.rs`) |
| Of those, sites anchored to `NirValue::Local(name)` (write path) | 3 | `src/codegen/engine/control/builder_control.rs:72`, `:154`, `:231` — each `matches!(inner.as_ref(), NirValue::Local(n) if n == resource)` |
| Token-level `.state` assign matcher | 1 | `src/ast/stmt.rs:226-262` |
| `tests/rt-behavior/resources` fixtures | 39 | `ls tests/rt-behavior/resources \| wc -l` → `39` |

### Verified properties

- **The `.state` read arm is target-shape agnostic.** Read
  `lower_field_access:460-490`: it operates on `target_value` (an already-lowered
  `ValueResult`) and its `type_`, never on the `NirValue`'s shape. So a
  `MemberAccess` target reaches it unchanged — the only obstacle is the type
  spelling, above.
- **The write path is not.** Read `builder_control.rs:72`, `:154`, `:231`: all three
  require `NirValue::Local(name)` identical to the resource name. This is a
  deliberate pattern-match (the in-place scalar-store optimization,
  `try_inplace_state_scalar_assign`) and is why §1 makes writing a non-goal.
- **UNVERIFIED:** whether `push_type_identifiers`'s owner-pool filter discards `RES`
  and `STATE`, and whether an importer needs the resource type itself in its tables
  to lay out a record with a `RES` field. Phase 1 measures both with a real
  two-package fixture; do not assume either.

## 3. Design Overview

Two independent pieces, and they are independent enough to land in either order —
they are sequenced read-first because the read is what the goal is stated in terms
of.

1. **`STATE` on a field, and the two-dot read.** Accept the `STATE T` clause in the
   field-type grammar (replacing letter D's rejection), and make a record field read
   yield the `RES`-stripped field type so `.state()` sees the clause. Then verify
   agreement checking (`TYPE_STATE_MISMATCH`) applies at the field position the same
   way it applies at a collection element.
2. **Package export.** Make the type-export closure carry what an importer needs for
   a resource-carrying record, driven by the two-package fixture letter D left as a
   baseline.

**Design uncertainty concentrates in (2)** — the §2 UNVERIFIED rows — so Phase 1
measures it before any code is written.

**Correctness risk concentrates in (1)**, in a way `.ai/resources-packages.md`
already documented for the collection twin and that is worth repeating here: a
`STATE`-carrying resource type mismatch surfaces as a codegen-time
`TYPE_CALL_ARGUMENT_MISMATCH` (a bare `error:` with no context) during a **full**
`mfb build` — `mfb build -ast -ir` exits 0 without it. So a `tests/syntax/*`
fixture, which runs only `-ast -ir`, **cannot** protect this. Every negative test in
this letter must be a `tests/rt-behavior/*` fixture.

**Byte-identity is not this letter's gate** (new source shapes, new fixtures), but
`artifact-gate.sh all` must show diffs only in the new fixtures; a diff in a
pre-existing one is a bug to localize with objdump, not an expected cost.

Rejected alternatives:

- *Make `split_state` see through `Res`.* Rejected: `split_state` is documented as
  answering the narrow "is this type's **own** top-level clause" question, with
  `contains_state` as the broad twin (`src/types.rs:649-660`); widening it would
  break that split and every consumer that relies on the narrow reading. Strip the
  marker at the field-read site instead, exactly as the collection element does.
- *Extend the `.state` assign token matcher to arbitrary chains.* Rejected as
  non-goal (§1): it is a parser change with its own blast radius, and the
  `RES f = h.handle` rebind gives full write access today with one extra line.

## 4. Detailed Design

### 4.1 Field read yields the `RES`-stripped type

In `lower_field_access`'s record branch
(`src/codegen/memory/value/builder_value_semantics.rs:504-512`), the produced
`ValueResult.type_` becomes `field_type` with a top-level `ParameterType::Res`
unwrapped, leaving `Stateful { base, state }` (or the bare resource when the field
carries no `STATE`). The value itself is unchanged — the slot already holds the
record pointer. Mirror the wording of the collection-element rule in the doc
comment so the two stay recognizably the same rule.

Then `h.handle.state` reaches the existing `member == "state"` arm with
`state() == Some(Cursor)` and takes the concrete-resource path
(`emit_resource_record_ptr` → identity for a concrete resource → load
`RESOURCE_OFFSET_STATE`), which is already exercised by every stateful `RES`
binding.

### 4.2 `STATE` agreement at the field position

`RES f AS fs::File STATE Other = h.handle` must be rejected with
`TYPE_STATE_MISMATCH`, exactly as it is for a collection element
(§15.6: *"binding it with `RES x AS fs::File STATE Cursor` and reading `x.state` is
checked for agreement (`TYPE_STATE_MISMATCH`) exactly as for a concrete stateful
resource"*). Check whether the existing agreement check reaches a field-read
initializer; if it does not, route it there. Phase 2 task.

### 4.3 Export closure

Driven by Phase 1's measurement. The likely shape, per
`.ai/resources-packages.md`: validation needs the field-referenced user types (the
`STATE` record), and native codegen additionally needs anything it inlines. A `RES`
field is an 8-byte pointer slot (letter B), so codegen needs no inlined size for the
resource itself — but it does need the resource registered so
`resource_cleanup_symbol` and the close-op tables resolve. Confirm against the
"Imported native-resource close-op: 4 starved layers, 2 spellings" section before
writing code; that section is the map of how this fails silently.

## Compatibility / Format Impact

- A record field type may render as `RES fs.File STATE Cursor` in `.ir` and `.mfp`.
  No new encoding — `ParameterType::Res` and `Stateful` already render and
  round-trip (`ParameterType::parse(s).name() == s` is load-bearing and pinned by
  the round-trip corpus).
- Importer-visible: a package may export such a record. Additive.
- No ABI, resource-header, or runtime-format change.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work it describes; `- [~]` for partial with one line on what
> remains; `- [x] ~~text~~ — moot: <evidence>` for a dropped task. An unticked box
> means NOT DONE.

### Phase 1 — Measure the export path

Answers the two §2 UNVERIFIED rows before any code depends on them.

- [ ] Take letter D's baseline two-package fixture and record exactly what it does
      today: does the `.mfp` write, does the importer resolve the record, and if it
      fails, with which message and at which of the four layers named in
      `.ai/resources-packages.md` ("Imported native-resource close-op: 4 starved
      layers, 2 spellings")?
- [ ] Read `push_type_identifiers` (`src/binary_repr/mod.rs:804`) and its owner-pool
      filter; record whether `RES` and `STATE` are discarded as non-pool names.
- [ ] Write both answers into Corrections. Phase 4's scope comes from them.

Acceptance: the failure (or success) mode is named, located at a specific layer, and
written down.
Commit: —

### Phase 2 — `STATE` on a field type, and the two-dot read

- [ ] Accept `STATE T` in the field-type grammar, replacing letter D's rejection.
      Reuse the collection-element clause parse; do not add a second `STATE` parser.
- [ ] Strip the top-level `Res` from a record field read's result type per §4.1.
- [ ] Verify `TYPE_STATE_INVALID` still gates the `STATE` type to a copyable,
      defaultable data type at the field position (§15.5), and that
      `TYPE_STATE_MISMATCH` fires on a disagreeing rebind per §4.2; route it if not.
- [ ] Tests — **`tests/rt-behavior/*` only**, per §3 (a `syntax/*` fixture runs only
      `-ast -ir` and cannot see the codegen-time mismatch):
      - `record-res-field-state-rt` — write through the owning binding
        (`f.state.pos = 42`), then read `h.handle.state.pos` through a **copy** of
        the record and print it; the assertion is that it prints `42`, proving one
        shared payload rather than two;
      - `record-res-field-state-mismatch-rt` — a disagreeing `STATE` rebind →
        `TYPE_STATE_MISMATCH`;
      - `record-res-field-state-invalid-rt` — a non-copyable `STATE` type →
        `TYPE_STATE_INVALID`.

Acceptance: the round-trip fixture prints `42` (not `0`); both negative fixtures
show their codes from a **full** `mfb build`, not from `-ast -ir`.
Commit: —

### Phase 3 — Document the write path honestly

- [ ] Add to §15.6 (or §15.5, wherever the field rules land) that a `STATE` **write**
      through a record-field chain is not spelled directly, and give the
      `RES f AS fs::File STATE Cursor = h.handle` rebind as the way to write. State
      it as a language rule with its reason (the member-target assignment is the
      one exception to "no member assignment", §15 `:58`), not as a limitation note.
- [ ] Fixture `record-res-field-state-write-rt` proving the rebind path writes a
      payload the record's other holders then read.

Acceptance: the fixture writes through the rebind and reads the new value back
through the record; the spec states the rule.
Commit: —

### Phase 4 — Export a resource-carrying record (largest blast radius)

- [ ] Implement Phase 1's finding in the type-export closure
      (`enqueue_referenced_types` / `read_package_type_exports_resolved`), checking
      each of the four layers in `.ai/resources-packages.md` — a fix to one leaves
      the others silent.
- [ ] Promote letter D's baseline fixture to a working two-package fixture: the
      package exports `TYPE Holder { handle AS RES fs::File STATE Cursor }` and a
      constructor; the importer declares one, reads `h.handle.state`, and closes at
      scope exit.
- [ ] Regenerate every affected golden. Editing a package's `.mfb` ripples to every
      importer's `.ir` and all five `.ncodesum`s — keep the edit line-neutral where
      possible and prove the delta is only yours by normalizing `"line": N` before
      diffing (`.ai/resources-packages.md`, "Editing an embedded builtin .mfb ripples
      to EVERY importer's goldens").

Acceptance: the importer fixture builds, runs, prints the payload it expects, and
closes exactly once (close-site count, as in letter C).
Commit: —

### Phase 5 — Feature closeout

- [ ] Spec: `15_resource-management.md` §15.5 (add the field row to the
      position table — a field, like an element, names the type and rides it) and
      §15.6 (a field carries `STATE` exactly as an element does);
      `04_types.md` §4.2; `./mfb spec architecture escape-analysis` (the record
      edges from letter C, if letter C did not already land them).
- [ ] `.ai` invariant docs: `.ai/resources-packages.md` gains a "Record `RES` field"
      section (the `RES`-stripped field read, the owned-list-by-binding-name fact
      from letter C, and the arena-transferable split from letter B);
      `.ai/codegen-invariants.md` gains the two-predicate distinction if letter B did
      not land it there.
- [ ] Full `scripts/artifact-gate.sh target/release/mfb all` and
      `scripts/test-accept.sh target/release/mfb /tmp/plan114e-scratch`.
- [ ] `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`
- [ ] Move `planning/plan-114-A` … `plan-114-E` to `planning/completed/`.

Acceptance: full gate and acceptance harness green; every doc listed above updated;
all five sub-plans archived.
Commit: —

## Validation Plan

- Tests: five new `tests/rt-behavior/resources/` fixtures (state round-trip, state
  mismatch, state invalid, state write via rebind, package export) plus the
  two-package importer fixture. **No `tests/syntax/*` fixture may be the only guard
  for a `STATE` type error** — §3.
- Coverage check: measure with `--bin mfb`; the new field-read strip in
  `lower_field_access` must be in the denominator. Integration fixtures run in an
  uncaptured subprocess and do not count.
- Runtime proof: `record-res-field-state-rt` printing `42` read through a record
  **copy** after a write through the owning binding. That single value is the proof
  that a record field aliases one resource rather than duplicating it — the whole
  premise of the feature.
- Doc sync: `src/docs/spec/language/15_resource-management.md` §15.5/§15.6,
  `src/docs/spec/language/04_types.md` §4.2,
  `./mfb spec architecture escape-analysis`, `.ai/resources-packages.md`,
  `.ai/codegen-invariants.md`. Per `.ai/specifications.md` the embedded spec must
  stay current with every compiler change.
- Acceptance: `cargo test --no-fail-fast` (redirect to a file; check cargo's exit
  status); `cargo check --all-targets`;
  `scripts/test-accept.sh target/release/mfb /tmp/plan114e-scratch`;
  `scripts/artifact-gate.sh target/release/mfb all`; `cargo fmt`.

## Open Decisions

- **`STATE` write through a record-field chain.** Recommendation: leave it out, as
  §1 states, and document the rebind (Phase 3). The token matcher at
  `src/ast/stmt.rs:226-262` would have to become a chain parser, and the three
  `NirValue::Local(name)` anchors at `builder_control.rs:72/:154/:231` would each
  need a chain-aware form — a parser plus three codegen pattern-matches, for
  something one extra line already expresses. If it is wanted, it is its own plan,
  not scope here.
- **Should a bare (no-`STATE`) `RES` field read still strip the `RES`?**
  Recommendation: yes, unconditionally — the collection element does
  (`list_element("List OF RES Socket") == "Socket"`), and one rule for both is what
  keeps them from drifting.

## Corrections

**C1 (Phase 1, answered ahead of schedule) — the owner-pool filter DOES discard
`RES` and `STATE`, and §2's candidate list has the tokenization slightly wrong.**
Read `push_type_identifiers` (`src/binary_repr/mod.rs:819-831`) and the loop that
consumes its queue (`:769-784`).

The tokenizer keeps runs of `[A-Za-z0-9_]` and breaks on everything else — so a
`.` is a separator. For a field rendered `RES fs.File STATE Cursor` the queue
therefore receives **five** tokens, not the four §2 lists:

    RES, fs, File, STATE, Cursor

(§2 says the candidates are `RES`, `fs.File`, `STATE`, `Cursor`; the qualified
name is actually split into `fs` and `File`.) That does not change the
conclusion, but it does change what a debugger would expect to see in the queue.

The filter is not a name blocklist — it is existence in the owner's export list:

```rust
let Some(owner_exports) = owner_pool.get(&owner) else { continue };
let Some(def) = owner_exports.iter().find(|candidate| candidate.name == name) else { continue };
```

So a token survives **only** if it names an actual export of the owner package.
`RES`, `STATE`, `fs` and `File` are not exports of the exporting package and are
dropped silently; `Cursor`, the `STATE` record, resolves and is pulled into the
closure — which is the one that genuinely must reach the importer. The
module's own doc comment states this design intent at `:788-794` ("pulling the
identifier tokens over-approximates the referenced names — the caller keeps only
those that resolve to an actual export"), so the over-approximation is
deliberate and `RES`/`STATE` need no special-casing.

**Still UNVERIFIED, and Phase 1 must still measure it:** whether an importer needs
the *resource type itself* registered in its tables to lay out and close a record
with a `RES` field. That is a different question from the type closure above —
it concerns `resource_cleanup_symbol` and the close-op tables, i.e. the four
starved layers in `.ai/resources-packages.md`. Nothing here answers it, and the
two-package fixture is still the way to find out. Do not read C1 as closing
Phase 1.

**C2 (Phase 1, answered by letter D's baseline measurement) — the export half
is DONE, and Phase 4's real obstacle is not about resources.**

letter D measured the `.mfp` case rather than fixturing it (plan-114-D Correction
C3). Two results, and they move this letter's weight:

1. **A `RES` field already round-trips through a `.mfp`.** A package exporting
   `TYPE Holder { label AS String, handle AS RES fs::File }` builds, writes its
   `.mfp`, and an importer links against it and calls an exported function that
   constructs the record internally — running to exit 0. So the type-export
   closure needs no change for the resource itself, which is what C1's reading of
   `push_type_identifiers` predicted. §2's second UNVERIFIED row ("does an
   importer need the resource type registered in its tables") is answered **no**
   for this path.

2. **An importer cannot construct a user-package record — and that is
   pre-existing and resource-independent.** Both spellings fail:
   `pkg::Holder[...]` with `2-203-0043 TYPE_UNKNOWN_VALUE`, and the unqualified
   `Holder[...]` with a bare codegen `error: native code field access target
   'holder_pkg.Plain' is not a record or variant`. The second reproduction uses a
   resource-free `EXPORT TYPE Plain { label AS String }`.

**Phase 4 is therefore re-scoped.** Its task list assumes the obstacle is the
resource and that fixing the type-export closure lets an importer "declare one,
read `h.handle.state`, and close at scope exit". The resource is not the
obstacle; user-package record construction is, for every record. Two honest
options, to be decided in Phase 4 with the evidence above:

- **(a)** Write the fixture so the importer never constructs the record — it
  calls an exported constructor function and reads the returned value. This
  exercises everything this feature owns (export closure, field layout across the
  boundary, `STATE` read, close at scope exit) and stays inside plan-114's scope.
- **(b)** Fix user-package record construction. That is a real defect worth a
  bug of its own, but it is **not this feature**: it blocks plain records equally,
  it predates plan-114, and folding it in would hide a general package-system fix
  inside a resource plan.

**Recommendation: (a), and file (b) separately.** Option (b) is exactly the
"absorb an unrelated fix into this plan" move that makes a letter unlandable, and
the measurement above is what a bug report for it needs anyway.

<!-- Further corrections filled in during execution. -->

## Summary

The load-bearing change in this letter is one line of type spelling: a record field
read must yield the `RES`-stripped field type, or `.state()` returns `None` and the
two-dot read dies as "record has no field 'state'". The collection element solved
the identical problem the identical way, which is both the design and the evidence
it works. The real risk is the test *tier*: a `STATE` type error surfaces only in a
full `mfb build`, so a `syntax/*` fixture would pass while the feature was broken —
every negative test here is `rt-behavior/*` for that reason. Deliberately left
undone and documented as such: writing `STATE` through a record-field chain.
