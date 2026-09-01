# plan-114-D: Lift the ban — `RES` record fields become legal source

Last updated: 2026-08-30
Effort: medium (1h–2h)
Depends on: plan-114-C

Letters B and C built the layout and the ownership routing behind a still-closed
front door. This letter opens it: the field-type grammar accepts the `RES`
ownership marker, `TYPE_RESOURCE_FIELD_FORBIDDEN` (`2-203-0084`) is retired, and a
record field carrying a resource is checked by the same two rules that already
govern a collection element — `TYPE_RESOURCE_REQUIRES_RES` for a bare resource
field and `TYPE_RES_REQUIRES_RESOURCE` for `RES` on a non-resource.

Behavioral outcome: a program that declares `TYPE Holder { handle AS RES fs::File }`,
opens a file into one, copies the record, uses the handle through the copy, and
lets the scope end, compiles, runs, and closes the file exactly once — and doing
the same 200 times in a loop does not exhaust file descriptors.

References:

- `src/docs/spec/language/15_resource-management.md` §15 (`:40`, the ban) and
  §15.6 (the collection-slot rules this mirrors).
- `src/docs/spec/language/04_types.md` §4.2 — record construction and `WITH`.
- `src/docs/spec/diagnostics/01_rule-codes.md:434` — the `2-203-0084` row.
- `src/rules/table.rs:1109` — the precedent for retiring-but-reserving a code
  (`RESOURCE_SHADOWS_BUILTIN`), and `:996` for a reserved-not-deleted rule
  (`2-203-0086`).
- `.ai/testing-gates.md` — the gate is blind to diagnostic prose.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-114-C complete and landed | `ls planning/completed/plan-114-C-*` → one match | NOT MET |
| plan-114-B complete and landed | `ls planning/completed/plan-114-B-*` → one match | NOT MET |
| plan-114-A complete and landed | `ls planning/completed/plan-114-A-*` → one match | NOT MET |
| Working tree clean; release `mfb` built | `git status --porcelain` → empty | MET (2026-08-30) |
| No other artifact-gate / test-accept running | `pgrep -f '[a]rtifact-gate\|[t]est-accept'` → no output | MET (2026-08-30) |

If any letter A–C is not complete, this letter cannot start, full stop. Everything
below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `TYPE Holder` with `handle AS RES fs::File` parses, type-checks, compiles, links,
  and runs.
- `handle AS fs::File` (no `RES`) is rejected with `TYPE_RESOURCE_REQUIRES_RES`
  (`2-203-0082`), exactly as `List OF fs::File` is.
- `handle AS RES Integer` is rejected with `TYPE_RES_REQUIRES_RESOURCE`
  (`2-203-0083`), exactly as `List OF RES Integer` is.
- `2-203-0084 TYPE_RESOURCE_FIELD_FORBIDDEN` is **reserved, not deleted** and never
  emitted.
- A record carrying a resource is not comparable, cannot be a `Map` key, and cannot
  cross a thread data plane (`2-203-0138`, from letter A) — each proven by a
  fixture, not asserted.
- Runtime proof: one close per opened handle, over 200 iterations, with the handle
  used through a record copy after the original binding.

### Non-goals (explicit constraints)

- **No `STATE` through a record field yet.** `handle AS RES fs::File STATE Cursor`
  and reading `h.handle.state` are letter E. A `STATE` clause on a field type is
  rejected in this letter with the existing `TYPE_STATE_*` machinery, or parses and
  is rejected at verify — Phase 1 decides which and says so.
- **No `.mfp` export of a resource-carrying record.** Letter E.
- **A union may still not mix data and resource variants** (`TYPE_MIXED_RESOURCE_UNION`,
  `2-203-0087`). Untouched.
- **Threads unchanged.** A resource still crosses only on the `RES` plane.
- Records are still not printable or serializable — see §2, `toString` has never
  accepted a record, and `json::stringify` accepts only `Json` and its member
  types. This letter adds no print/serialize surface and removes none.

## 2. Current State

### Where the ban lives — four layers, three still standing

| Layer | Site | State after letters B–C |
|---|---|---|
| Grammar | field-type position rejects the `RES` marker | **standing** — this letter |
| Front end | `src/ir/verify/types.rs:25-35` → `TYPE_RESOURCE_FIELD_FORBIDDEN` | **standing** — this letter |
| NIR backstop | `src/target/shared/validate/mod.rs:144` (`type_owns_resource`) | already relaxed by letter B |
| Spec | `src/docs/spec/language/15_resource-management.md:40` | **standing** — this letter |

Measured grammar behavior (`target/release/mfb build` on a scratch project):

- `a AS RES fs::File` → `1-102-0003 MFB_PARSE_INVALID_IDENTIFIER: Expected a type name.`
  pointing at `RES`.
- `a AS fs::File` → `2-203-0084 TYPE_RESOURCE_FIELD_FORBIDDEN: Record `Test` field `a`
  is resource `fs.File`; records cannot own resources.`

`ParameterType::Res(Box<ParameterType>)` already exists in the one canonical type
grammar (`src/types.rs:64`) and renders as `RES {inner}` (`:720`), so this is a
parse-position change, not a new type.

### The spec's stated rationale no longer holds

§15 `:40` justifies the ban as: *"a resource field would either trap copyable data
behind move-only semantics or let one value own several resources at once."* §15.6
`:159` answers both for collections, in the same chapter: *"A `RES` binding, a `RES`
parameter, and a collection slot … all hold a copy of the one handle pointer.
Copying the pointer never duplicates the resource … Collections of resources are
ordinary copyable collections of pointers — no move-only or linearity."* The
rationale is a leftover from the binding-owns model that plan-59-E retired (see the
retired `2-203-0086` at `src/rules/table.rs:996`). This letter rewrites `:40`
accordingly rather than deleting it.

### Measured populations

| What | Count | Command |
|---|---|---|
| Files naming `TYPE_RESOURCE_FIELD_FORBIDDEN` | 8 | `grep -rln "TYPE_RESOURCE_FIELD_FORBIDDEN" src/ tests/` → `src/ir/verify/types.rs`, `src/ir/verify/tests.rs`, `src/rules/table.rs`, `src/docs/spec/language/05_bindings-and-scope.md`, `src/docs/spec/language/15_resource-management.md`, `src/docs/spec/diagnostics/01_rule-codes.md`, `tests/syntax/resources/resource-record-field-invalid/golden/build.log`, `tests/syntax/tls/listen_invalid/golden/build.log` |
| Golden `build.log`s that must change | 2 | the two `tests/…/golden/build.log` above |
| `tests/syntax/resources` fixtures | 49 | `ls tests/syntax/resources \| wc -l` → `49` |
| `tests/rt-behavior/resources` fixtures | 39 | `ls tests/rt-behavior/resources \| wc -l` → `39` |
| `TYPE_RESOURCE_REQUIRES_RES` emission sites | 2 | `grep -rn "TYPE_RESOURCE_REQUIRES_RES" src/ --include='*.rs' \| grep -v tests` → `src/ir/verify/resources.rs:941`, `src/ir/verify/ops.rs:214` |

### Verified properties

- **Comparability already handles a `RES` field correctly — no work needed.** Read
  `is_comparable_seen` (`src/ir/verify/values.rs:1023-1070`): the record arm at
  `:1061-1066` maps every field through `resource_base_type(ft)` before recursing,
  which strips the `RES ` marker, and `is_resource_name` then returns `false` for
  the resource at `:1043`. So a record with a `RES fs::File` field is not
  comparable, and by the same path cannot be a `Map`/`Set` key
  (`check_map_key_comparable`, `:928`). This letter **proves** that with fixtures;
  it does not build it.
- **Thread rejection already handles a `RES` field — no work needed.** Read
  `record_fields_sendable` (`src/ir/verify/resources.rs:474-482`): it recurses
  through `is_thread_sendable` on every field type, and `ParameterType::Res(_) => false`
  (`:439`). Letter A turns that into `2-203-0138`. The existing test
  `rejects_unsendable_resource_plane_state_payload` (`src/ir/verify/tests.rs:7975`)
  is the precedent — it already refuses a record whose field is `List OF RES fs.File`.
- **There is no print/serialize surface to gate.** Built a scratch project calling
  `toString(h)` on a plain record: `2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH — Call to
  `toString` has argument type(s) (Holder), expected Integer, Float[, Byte], Fixed[,
  Byte], Boolean, String, Byte, Scalar, or List OF Byte.` Records have never been
  printable, and `mfb man json` documents `json::stringify` as accepting only the
  `Json` union or one of its six member types. Nothing to add.
- **`is_copyable` already returns true for a `RES` field.** Read
  `src/ir/verify/resources.rs:342` — `ParameterType::Res(_) | ParameterType::Func(..) => true` —
  and `record_fields_copyable` (`:379`) descends fields through it. So a record
  with a `RES` field stays a copyable data type, which is what makes it a legal
  `STATE` type and a legal collection element.
- **UNVERIFIED:** what a `STATE T` clause on a field type does after the grammar
  accepts `RES` — parse error, or parse-then-reject. Phase 1 measures it and Phase 2
  makes it a clean rejection deferred to letter E.

## 3. Design Overview

Three pieces:

1. **Grammar** — accept `RES <type>` in a record field-type position. The
   `ParameterType` already exists; this is about which positions the parser will
   read the marker in.
2. **Rules** — delete the `TYPE_RESOURCE_FIELD_FORBIDDEN` emission, reserve the
   code, and let the field walk fall through to the same `RES`-marker rules a
   collection element takes.
3. **Proof** — fixtures for every rejection that must survive (bare field, `RES` on
   a non-resource, comparability, map key, thread plane) and an rt-behavior fixture
   for the accept path with a close-count runtime proof.

**Correctness risk concentrates in (2)**, in a specific and non-obvious way: the
`RES`-marker rules for a collection element are emitted from two sites
(`src/ir/verify/resources.rs:941`, `src/ir/verify/ops.rs:214`), and neither may be
reached by a record field today. If the field walk simply *stops* emitting
`TYPE_RESOURCE_FIELD_FORBIDDEN` without routing to those rules, `handle AS fs::File`
becomes silently accepted — a bare resource field with no ownership marker, which
letter C's analysis will not float and which will leak. **A negative fixture for
the bare-field case is therefore not optional and not a nice-to-have; it is the
thing that catches this.** Phase 2 lands it before Phase 3 opens the accept path.

**Byte-identity is the WRONG gate for this letter.** New source shapes become
legal, so `.ncode` for the new fixtures is new output, and diagnostic goldens
change by design. The gates are: `test-accept.sh` for the diagnostic prose (the
artifact gate cannot see it), the rt-behavior close-count fixture for the runtime
contract, and `artifact-gate.sh all` → `diffs=0` **only** for the pre-existing
fixtures — a diff there means an existing program's codegen moved, which is a bug.

Rejected alternatives:

- *Keep `2-203-0084` and reword it to fire only for a bare field.* Rejected: it
  would duplicate `TYPE_RESOURCE_REQUIRES_RES`, which already says exactly that for
  a collection element, and users would get two different codes for one mistake
  depending on where they made it.
- *Delete `2-203-0084` from the table.* Rejected: `src/rules/table.rs:1109` and
  `:986` establish the convention — reserve, never recycle a code.

## 4. Detailed Design

### 4.1 Field-type grammar

A record field type becomes `[RES] <type> ` where the `RES` marker sets
`ParameterType::Res`. Locate the field-type parse site (Phase 1) and route it
through the same entry point the collection element uses — do not add a second
`RES`-marker parser. `ParameterType::parse` is the only type grammar
(`.ai/…`/`src/types.rs`); consumers match variants and never re-parse.

### 4.2 Rule routing

In `src/ir/verify/types.rs:25-35`, replace the `TYPE_RESOURCE_FIELD_FORBIDDEN`
emission with the collection-element pair:

- field type is a bare resource (no `Res` wrapper) → `TYPE_RESOURCE_REQUIRES_RES`,
  message shaped like the element message at `src/ir/verify/resources.rs:941`;
- field type is `Res(T)` where `T` is not a resource → `TYPE_RES_REQUIRES_RESOURCE`.

Reuse the existing emitters rather than writing new ones, so the two spellings
cannot drift.

In `src/rules/table.rs:975`, keep the entry and mark it reserved, following the
`2-203-0086` comment style at `:986`: state that plan-114 retired it, that a record
field is now governed by `2-203-0082`/`2-203-0083`, and that the code is reserved
rather than deleted so it is never recycled.

## Compatibility / Format Impact

- **Source-language surface grows**: `RES` is accepted in a record field type. This
  is additive — no previously-valid program changes meaning.
- **Diagnostics**: `2-203-0084` is reserved and no longer emitted; the same
  programs are still rejected, under `2-203-0082` / `2-203-0083`. Two golden
  `build.log`s change (§2).
- **`.mfp`**: a record field type may now render as `RES fs.File`. Exporting such a
  record across a package boundary is **letter E** — this letter does not enable it
  and Phase 3 adds a fixture asserting the current behavior, whatever it is, so
  letter E has a baseline.
- No ABI, layout, or runtime-format change.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work it describes; `- [~]` for partial with one line on what
> remains; `- [x] ~~text~~ — moot: <evidence>` for a dropped task. An unticked box
> means NOT DONE.

### Phase 1 — Locate the grammar site and measure the `STATE` case

- [ ] Find where a record field's type is parsed and where a collection element's
      `RES` marker is parsed; confirm they can share one entry point (§4.1). Record
      both sites in Corrections.
- [ ] With the grammar change stubbed in locally (do not commit), measure what
      `handle AS RES fs::File STATE Cursor` does — parse error, or parse-then-reject,
      and with which code. This resolves the §2 UNVERIFIED row and sets Phase 2's
      `STATE` task.

Acceptance: both sites named, and the `STATE`-clause behavior measured and written
into Corrections.
Commit: —

### Phase 2 — Rules first: reject the wrong shapes before accepting the right one

Landing the rejections **before** the grammar opens is what prevents the silent-accept
failure described in §3.

- [ ] Route the field walk in `src/ir/verify/types.rs:25-35` to
      `TYPE_RESOURCE_REQUIRES_RES` / `TYPE_RES_REQUIRES_RESOURCE` per §4.2, reusing
      the existing emitters.
- [ ] Reserve `2-203-0084` in `src/rules/table.rs:975` per §4.2, and update its row
      in `src/docs/spec/diagnostics/01_rule-codes.md:434` to say reserved/retired.
- [ ] Reject a `STATE` clause on a field type with a clear diagnostic, per Phase 1's
      measurement, noting in the message that it is not yet supported. (Letter E
      turns this into an accept; a bare "unsupported" is not acceptable as a
      permanent state, which is why letter E is in the same feature.)
- [ ] Update `tests/syntax/resources/resource-record-field-invalid/golden/build.log`
      — its source is `handle AS fs::File` (no `RES`), so it must now show
      `2-203-0082 TYPE_RESOURCE_REQUIRES_RES`. Read the source before regenerating.
- [ ] Update `tests/syntax/tls/listen_invalid/golden/build.log` the same way, after
      reading which shape it uses.
- [ ] New fixture `tests/syntax/resources/resource-record-field-res-required-invalid/`
      — `RES Integer` field → `2-203-0083`.
- [ ] Update the unit test `rejects_resource_field_in_record`
      (`src/ir/verify/tests.rs:3190`) to assert the new codes. Do not delete it.

Acceptance: both wrong shapes are rejected with the collection-mirroring codes;
`grep -rn "TYPE_RESOURCE_FIELD_FORBIDDEN" src/ --include='*.rs' | grep -v "rules/table.rs"`
returns nothing (no emission site remains).
Commit: —

### Phase 3 — Open the grammar and prove the accept path end to end (largest blast radius)

- [ ] Accept `RES` in the field-type position per §4.1.
- [ ] Rewrite `src/docs/spec/language/15_resource-management.md:40` — the ban
      sentence becomes the rule: a record field holds a copy of the one handle
      pointer, owns nothing, and is governed by the same `RES` marker and float
      rules as a collection slot (§15.6); cross-reference `2-203-0138`. Fix the
      `TYPE_RESOURCE_FIELD_FORBIDDEN` mention in
      `src/docs/spec/language/05_bindings-and-scope.md`.
- [ ] New fixture `tests/rt-behavior/resources/record-res-field-rt/` — the runtime
      proof: open a file into a `Holder`, copy the record, write through the copy's
      handle **after** the copy, print a marker, and loop the open/scope-exit 200
      times. Model it on `tests/rt-behavior/resources/res-rebind-alias-runtime/`,
      whose comments explain why the post-copy *use* is the assertion.
- [ ] **Added task (inherited from plan-114-C Correction C5).** letter C could not
      write the close-site-count assertions its Phase 3 asked for: no `CodeBuilder`
      is constructible in a test (`grep -rn "CodeBuilder {" src/codegen/ | grep -i
      test` → nothing), and the existing count harness
      (`tests/rt_native_resource_scope_drop.rs`) compiles MFBASIC source, which the
      then-standing ban made impossible for this shape. **This letter lifts that
      ban, so they become expressible — write them here.** Three properties, all
      double-close or leak bugs if wrong:
      (a) a floated record-carried handle closes **exactly once**, at the record
          binding's scope, and **not** at the resource's own scope;
      (b) a **returned** float-target record emits **zero** closes in the callee
          (the caller adopts and closes);
      (c) `RES g = h.handle` adds **no** close site.
      The 200-iteration loop in the fixture above covers (a) and (c) behaviourally
      — a missed drain exhausts fds, a double close raises `7-703-0004`. (b) needs
      its own fixture returning a `Holder` from a `FUNC` and closing in the caller.
      letter C pinned all three at their decision points
      (`is_resource_owning_container`, `record_res_field_types`,
      `value_aliases_live_resource`); this is the end-to-end half.
- [ ] New fixtures for the three rejections that must survive:
      `record-res-field-map-key-invalid` (record with a `RES` field as a `Map` key →
      `TYPE_REQUIRES_COMPARABLE`), `record-res-field-compare-invalid` (`=` on two
      such records), `record-res-field-thread-plane-invalid` (→ `2-203-0138`).
- [ ] Baseline fixture for the `.mfp` case: a package exporting a record with a
      `RES` field, with a golden capturing whatever happens today. Letter E owns
      making it work; this records the starting point so letter E's delta is
      provably its own.
- [ ] Regenerate goldens with `scripts/sync-goldens.sh` for the new fixtures, and
      generate the target-infixed `.ncodesum`/`.app.ncode` by hand where needed
      (`sync-goldens.sh` skips `syntax/**` and does not refresh those).

Acceptance: the rt-behavior fixture runs, prints its markers, and the 200-iteration
loop completes without `7-703-0004` or fd exhaustion; all three rejection fixtures
show their expected codes; `scripts/artifact-gate.sh target/release/mfb all` shows
diffs **only** in the newly added fixtures — a diff in a pre-existing fixture is a
bug to root-cause (objdump one fixture), not an expected cost.
Commit: —

## Validation Plan

- Tests: `src/ir/verify/tests.rs` (updated `rejects_resource_field_in_record`, plus
  comparability and map-key assertions for a `RES`-field record — these prove the
  §2 verified properties rather than assuming them); five new fixtures under
  `tests/syntax/resources/` and `tests/rt-behavior/resources/`.
- Coverage check: measure with `--bin mfb`. The new rule-routing branch in
  `src/ir/verify/types.rs` must be in the denominator; integration fixtures run in
  an uncaptured subprocess and do not count toward it
  (`.ai/coverage-measurement-mechanics`), so the unit assertions are what cover it.
- Runtime proof: `tests/rt-behavior/resources/record-res-field-rt/` — the
  post-copy write is the assertion that the copy aliased rather than duplicated,
  and the 200-iteration loop is the assertion that the drain neither leaks nor
  double-closes. A silent pass with no post-copy use proves nothing (that is the
  documented trap in `res-rebind-alias-runtime`'s header comment).
- Doc sync: `src/docs/spec/language/15_resource-management.md` §15 `:40` and §15.6;
  `src/docs/spec/language/05_bindings-and-scope.md`;
  `src/docs/spec/diagnostics/01_rule-codes.md:434`;
  `src/docs/spec/language/04_types.md` §4.2 (a record field may carry a `RES`
  handle). Per `.ai/specifications.md`, the embedded spec must stay current with
  every compiler change.
- Acceptance: `cargo test --no-fail-fast` (redirect to a file; check cargo's exit
  status); `cargo check --all-targets`;
  `scripts/test-accept.sh target/release/mfb /tmp/plan114d-scratch` (full — this is
  a diagnostics change); `scripts/artifact-gate.sh target/release/mfb all`;
  `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Does the bare-field case deserve its own message, or the element message
  verbatim?** Recommendation: the same rule code (`2-203-0082`) with a
  field-specific message naming the record and field, mirroring how the element
  message names the collection. Same code, different message — the precedent is
  `TYPE_REQUIRES_COMPARABLE`'s Set/Map split (`src/ir/verify/values.rs:938-943`).
- **Should a record with a `RES` field be a legal `STATE T` payload?**
  `is_copyable` says yes today (§2), and `TYPE_STATE_INVALID` requires copyable +
  defaultable. Recommendation: leave it legal — a `STATE` payload is deep-copied on
  `thread::transfer`, but letter A's `2-203-0138` already refuses that plane
  (`rejects_unsendable_resource_plane_state_payload` is the existing proof), so the
  dangerous path is closed and the in-thread case is sound. Add a fixture either
  way so the answer is recorded rather than latent.

## Corrections

<!-- Filled in during execution. -->

## Summary

The risk here is not the accept path — letters B and C built and tested that. It is
the rejection path: if the field walk stops emitting `2-203-0084` without routing to
`TYPE_RESOURCE_REQUIRES_RES`, a bare unmarked resource field is silently accepted
and leaks with no diagnostic. Phase 2 lands the rejections before Phase 3 opens the
grammar, specifically to make that failure impossible to ship. Three things measured
in §2 turned out to need **no work at all** — comparability, map keys, and thread
rejection are already correct through `resource_base_type` stripping — so this
letter proves them with fixtures instead of building them. Untouched: printing and
serialization (records were never printable), unions, and the `.mfp` boundary.
