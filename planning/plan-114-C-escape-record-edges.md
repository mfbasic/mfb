# plan-114-C: Ownership floats through a record

Last updated: 2026-08-30
Effort: medium (1h–2h)
Depends on: plan-114-B

Resource escape analysis (`src/ir/resource_escape.rs`) decides, per `RES` binding,
where its close obligation is discharged: `ResOwner::Local` (its own producing
scope) or `ResOwner::Float(container)` (an outer container's scope drains it).
Today the only containers it models are collections — its float edges are list and
map literals and the eight collection insertion builtins (`is_insertion_builtin`,
`:501-516`).

A record is the same kind of container: a value that holds handle pointers and
whose binding has a scope. This letter adds the record edges so a resource placed
into a record floats to that record's binding scope, is drained once there, and
transfers to the caller when the record is `RETURN`ed — with the bug-291 ordering
rule extended so the declared-after-the-resource case is rejected rather than
silently miscompiled.

Behavioral outcome: for a function that puts a `RES` binding into a record,
`analyze_function` returns `ResOwner::Float(<record binding>)` for that binding,
and codegen drains the record binding's owned-list on every exit path — the same
mechanism `List OF RES` already uses, unchanged.

References:

- `src/docs/spec/language/15_resource-management.md` §15.6 — "ownership floats up",
  the return-transfer rule, `TYPE_RESOURCE_RETURN_ORDER`.
- `./mfb spec architecture escape-analysis` — the decision procedure this extends.
- `.ai/resources-packages.md` — "TRAP desugar hides producers as locals",
  "Owned-list union drain", "Scope-drop frees owned flat values".
- `src/ir/resource_escape.rs:1-49` — the module's own soundness argument.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-114-B complete and landed | `ls planning/completed/plan-114-B-*` → one match | NOT MET |
| Working tree clean; release `mfb` built | `git status --porcelain` → empty | MET (2026-08-30) |
| No other artifact-gate / test-accept running | `pgrep -f '[a]rtifact-gate\|[t]est-accept'` → no output | MET (2026-08-30) |

If plan-114-B is not complete, this letter cannot start, full stop. Everything
below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `analyze_function` treats a record `Constructor` and a `WithUpdate` as container
  expressions: a `RES` binding named in an argument or update position is an
  element insertion into the target binding, exactly as a list literal's element is.
- A `RES` binding that flows into a record declared in an outer scope gets
  `ResOwner::Float(<record binding>)`; one that flows into a same- or lower-scope
  record stays `ResOwner::Local`.
- A record binding that is a float target gets a runtime owned-list, drained on
  every exit path; a `RETURN` of that record transfers the list to the caller
  instead of draining it.
- The bug-291 ordering case — a returned record declared *after* the resource it
  carries — yields `ResOwner::FloatBlocked` and is rejected with
  `TYPE_RESOURCE_RETURN_ORDER` naming both bindings.
- `RES g = t.handle` (reading a handle back out of a record field) is an **alias**:
  it registers no close obligation of its own.

### Non-goals (explicit constraints)

- **The front-end ban stays up.** No MFBASIC source can build such a record until
  letter D; this letter is verified by HIR-level unit tests and codegen unit tests.
- **No new `ResOwner` variant and no `.ir` format change.** `Float(String)` and
  `FloatBlocked(String)` already carry a binding name; a record binding name fits.
  The tag bytes in `src/ir/binary.rs:916-931` do not change.
- **No change to the owned-list node layout** (`{record ptr@0, next@8}`, 16 bytes,
  `src/codegen/cleanup/owned/builder_owned_cleanup.rs:70`).
- **No change to collection behavior.** Every existing `ResOwner` decision for a
  program with no record-carried resource must be identical.
- Nesting a resource-carrying record inside a collection (`List OF Holder` where
  `Holder` has a `RES` field) is **in scope for the analysis** (the scan descends
  into a constructor inside a list literal) but see Open Decisions for the
  ordering-rule interaction.

## 2. Current State

### The analysis

`analyze_function(function)` (`src/ir/resource_escape.rs:141`) walks the HIR body,
recording `Routing { target: Var(name) | Returned, res_elems, src_collections }`
facts, then `solve()`s them to a `HashMap<String, ResOwner>`. The scan that
produces a routing is `scan_collection_expr` (`:~265-322`), whose container arms
are, read in full:

- `HirExpression::Identifier(name)` — a non-`RES` name is a plain collection copy.
- `HirExpression::ListLiteral(values)` — each value scanned as an element.
- `HirExpression::MapLiteral { entries }` — each value scanned as an element.
- `HirExpression::Call { callee, arguments }` where `is_insertion_builtin(callee)`
  (`:501`; the set at `:515` is `append | prepend | insert | set | mid | removeAt |
  filter | reduce`) — argument 0 is the container, the rest are elements.
- `HirExpression::Trapped { expression, handler }` — both arms flow to the same
  target (bug-290).
- `_ => {}` — everything else contributes nothing.

`HirExpression` has exactly **11** variants (`src/hir/mod.rs:413`); the two record
ones are `Constructor` (`:442`) and `WithUpdate` (`:447`). Both fall into the
`_ => {}` arm today. Record construction has no third spelling: the spec gives
`TypeName[...]` positional, `TypeName[field := ...]` by-field, and
`WITH v { field := expr }` (`src/docs/spec/language/04_types.md:104-124`).

### The codegen half already generalizes

This is the key de-risking fact for this letter. Read
`src/codegen/cleanup/owned/builder_owned_cleanup.rs:61-95`: `emit_owned_list_push`
looks the container up **by binding name** in `owned_list_heads`, allocates a
16-byte `{record ptr, next}` node, and links it onto a head held in a stack slot.
Nothing in it reads the container's own representation. The registration site
(`src/codegen/engine/control/builder_control.rs:626`) is likewise name-keyed:
`if self.owner_collections.contains(name) { self.setup_owned_list(name, type_)? }`,
and `owner_collections` is built generically from the `ResOwner::Float(name)`
values (`src/codegen/engine/function/function_lowering.rs:879-886`).

So a record binding can carry an owned-list with no new runtime structure. The
open question is whether `setup_owned_list(name, type_)` and the drain read
`type_` for anything collection-specific — a Phase 1 task, not an assumption.

### Measured populations

| What | Count | Command |
|---|---|---|
| `HirExpression` variants | 11 | `sed -n '413,520p' src/hir/mod.rs \| grep -cE "^    [A-Z][A-Za-z]*"` → `11` (of which `Constructor`, `WithUpdate` are the record forms) |
| Container arms in `scan_collection_expr` today | 5 | read of `src/ir/resource_escape.rs:265-322` (Identifier, ListLiteral, MapLiteral, insertion Call, Trapped) |
| Insertion builtins in the float set | 8 | `src/ir/resource_escape.rs:515` — `append prepend insert set mid removeAt filter reduce` |
| `ResOwner` consumer files outside the module | 15 | `grep -rln "ResOwner" src/ --include='*.rs' \| grep -v resource_escape.rs \| wc -l` → `15` |
| `src/ir/resource_escape.rs` LOC | 695 | `wc -l src/ir/resource_escape.rs` → `695` |

### Verified properties

- **The owned-list is representation-independent.** Read
  `emit_owned_list_push:61-95` — it uses only `owned_list_heads[name]` (a stack
  slot) and the resource's own slot. No collection header, entry table, or element
  type is touched.
- **`ResOwner::Float` already deactivates an aliasing producer.** Read
  `builder_control.rs:663-679`: on a float bind whose initializer is
  `NirValue::Local(src)`, it calls `deactivate_resource_cleanup(src)` so the
  TRAP-desugared `$trap_valN` temp does not close the record at its inner scope.
  A record float target reaches the same branch unchanged.
- **UNVERIFIED:** whether `setup_owned_list` and the exit drain read `type_` for
  anything collection-specific. Phase 1 task.
- **UNVERIFIED:** what shape `RES g = t.handle` lowers to at NIR (a
  `NirValue::MemberAccess` initializer on a resource-typed bind), and which arm of
  the `builder_control.rs:640-700` bind chain it currently falls into. Phase 3
  task — this decides whether the alias case needs a new arm or is already covered
  by `aliases_live_resource`.

## 3. Design Overview

Three pieces, layered:

1. **Scan** — two new arms in `scan_collection_expr` (`Constructor`, `WithUpdate`),
   plus `scan_element` descending into them. Pure HIR, unit-testable, no codegen.
2. **Solve/ordering** — the existing `solve()` needs the record analogue of the
   bug-291 gate. Its `decl_type` map is used only to avoid piling `FloatBlocked`
   onto a program already rejected for a missing `RES` marker
   (`src/ir/resource_escape.rs:130-135`), and it cannot tell whether `Named("Holder")`
   has a resource field without a type table. See Open Decisions.
3. **Codegen** — register a float-target record binding's owned-list and drain it;
   route the `RETURN` transfer. Expected to be a name/type generalization of
   existing code, not new machinery (§2 "Verified properties").

**Design uncertainty concentrates in (2)** — whether the ordering rule needs a type
table threaded into `analyze_function` — so it is scheduled early, as the cheapest
question that changes the shape of the work.

**Correctness risk concentrates in (3)**: a missed drain is a leaked handle with no
diagnostic, and a double drain is `7-703-0004` / a double free. It is scheduled
last, behind the Phase 1–2 tests.

**Byte-identity is NOT this letter's premise, but it IS a usable gate here**,
because the front-end ban means no source program has a record-carried resource:
every existing program's `ResOwner` map must be byte-for-byte what it is today.
`scripts/artifact-gate.sh target/release/mfb all` → `diffs=0`. A diff means the new
scan arms changed an existing decision — root-cause it (the `.ir` dump carries
`resource_owners`, so diff that first, not the `.ncode`), fix, re-run.

Rejected alternatives:

- *A new `ResOwner::FloatRecord(String)` variant.* Rejected: it would change the
  `.ir` binary tag encoding (`src/ir/binary.rs:916-931`) and every match site, to
  express a distinction codegen does not need — the drain is identical.
- *Treat a record field write as a mutation edge rather than a construction edge.*
  Rejected: records have no field assignment. `WITH` produces a new value, which is
  exactly the construction edge already being added.

## 4. Detailed Design

### 4.1 Scan arms

```
HirExpression::Constructor { arguments, .. } =>
    for each argument: self.scan_element(value, res_elems, src_collections)

HirExpression::WithUpdate { target, updates } => {
    // The updated value flows into the result, exactly like insertion arg 0.
    self.scan_collection_expr(target, res_elems, src_collections);
    for (_, value) in updates: self.scan_element(value, res_elems, src_collections)
}
```

`scan_element` (`:325`) already returns a `RES` identifier as a direct element and
otherwise recurses into `scan_collection_expr`, so a constructor nested in a list
literal (`[Holder[handle := f]]`) routes `f` to the list — no extra arm needed.

Read `HirExpression::Constructor`'s and `WithUpdate`'s exact field names at
`src/hir/mod.rs:442-450` before writing the arms; the shapes above are the
intent, not a transcription.

### 4.2 Alias out of a record field

`RES g = t.handle` must register no close obligation: the record's scope owns the
handle. In the analysis, `t.handle` is a `MemberAccess`, which `scan_element` will
route through `scan_collection_expr` and which contributes nothing — correct, since
reading a handle out is not an insertion.

The obligation-side decision is in codegen's bind chain
(`builder_control.rs:640-700`), where `aliases_live_resource` already suppresses the
cleanup for `RES g = f` (bug-375, `.ai/resources-packages.md`). Phase 3 determines
whether a `MemberAccess` initializer reaches that predicate; if not, extend
`value_aliases_live_resource` (`src/codegen/engine/value/builder_values.rs`) to
include a resource-typed `MemberAccess` on a record. Do **not** widen it to every
`MemberAccess` — the gate must stay on the resource-typed bind, per the existing
comment at `builder_control.rs:684-694`.

## Compatibility / Format Impact

- No `.ir` format change: `ResOwner` variants and their tag bytes are unchanged.
- The `.ir` dump's `resource_owners` map may gain record-binding names — but only
  for programs that cannot yet be written (the ban is still up), so no golden moves.
- No runtime structure change: the owned-list node stays `{record ptr@0, next@8}`.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work it describes; `- [~]` for partial with one line on what
> remains; `- [x] ~~text~~ — moot: <evidence>` for a dropped task. An unticked box
> means NOT DONE.

### Phase 1 — Resolve the two UNVERIFIED questions

Cheapest work that can change the shape of Phases 2–3. Do it before writing code.

- [ ] Read `setup_owned_list` and the exit drain
      (`src/codegen/cleanup/owned/builder_owned_cleanup.rs`,
      `src/codegen/resource/cleanup/builder_resource_cleanup.rs`) and record in
      Corrections exactly what they read from `type_`. If either is
      collection-specific, that becomes a Phase 3 task with named scope.
- [ ] Read the `solve()` ordering gate (`src/ir/resource_escape.rs:~440-480`,
      around the `FloatBlocked` insertion at `:473`) and `decl_type`'s use
      (`:~128-134`), and decide the Open Decision below: does the record ordering
      rule need a resource-record-type set threaded into `analyze_function`?
- [ ] Record both answers in Corrections. Do not proceed on an assumption.

Acceptance: both UNVERIFIED rows in §2 are answered with the function read and the
answer written down; the Open Decision is closed.
Commit: —

### Phase 2 — Scan arms and ordering (analysis only)

- [ ] Add the `Constructor` and `WithUpdate` arms to `scan_collection_expr` per §4.1.
- [ ] Extend the ordering gate so a returned record declared after the resource it
      carries yields `ResOwner::FloatBlocked(<record>)`, per Phase 1's answer.
- [ ] Verify `TYPE_RESOURCE_RETURN_ORDER`'s message (`src/ir/verify/mod.rs:336-340`)
      reads correctly when the container is a record; adjust the wording if it says
      "collection", and update `src/rules/table.rs:1027`'s message only if it does.
- [ ] Tests in `src/ir/resource_escape.rs`'s test module, mirroring the existing
      collection cases at `:615` and `:638`:
      - `MUT h AS Holder = …; WHILE { RES f = …; h = WITH h { handle := f } }` → `f`
        floats to `h`;
      - `RES f = …; LET h = Holder[handle := f]` at the same depth → `f` stays `Local`;
      - `LET h = Holder[handle := f]; RETURN h` with `h` declared *after* `f` →
        `FloatBlocked("h")`;
      - `LET xs = [Holder[handle := f]]` → `f` floats to `xs` (nesting via `scan_element`);
      - a record with no resource argument → no routing at all (regression guard).

Acceptance: the five analysis tests pass; every pre-existing test in
`src/ir/resource_escape.rs` and `src/ir/tests.rs` passes **unmodified**;
`scripts/artifact-gate.sh target/release/mfb all` → `diffs=0`.
Commit: —

### Phase 3 — Codegen: owned-list on a record binding, drain, return transfer, alias (largest blast radius)

- [ ] Generalize `owner_collections` → the float-target set is already built
      generically (`function_lowering.rs:879`); confirm and rename it to
      `owner_containers` so the next reader is not misled. Update
      `builder_control.rs:626` and `:1770` and `builder/mod.rs:271` accordingly.
- [ ] Make `setup_owned_list` and the exit drain work for a record binding, per
      Phase 1's finding.
- [ ] Route the `RETURN` transfer: returning a float-target record deactivates its
      owned-list instead of draining it, mirroring `deactivate_owned_list` for a
      returned `List OF RES` (`src/codegen/engine/exits/builder_exits.rs`).
- [ ] Alias case: confirm `RES g = t.handle` registers no cleanup; extend
      `value_aliases_live_resource` for a resource-typed record `MemberAccess` if
      Phase 1/§4.2 shows it does not already, keeping the resource-typed-bind gate.
- [ ] Tests (codegen unit tests over a hand-built NIR module — the source ban is
      still up): count close/reclaim sites in the emitted body, as
      `tests/native_resource_scope_drop.rs` does, asserting
      (a) a floated record-carried handle emits exactly **one** close at the record
      binding's scope and **none** at the resource's own scope;
      (b) a returned float-target record emits **zero** closes in the callee;
      (c) `RES g = t.handle` adds no close site.

Acceptance: the three close-site counts hold; `cargo test --no-fail-fast` green;
`scripts/artifact-gate.sh target/release/mfb all` → `diffs=0`.
Commit: —

## Validation Plan

- Tests: analysis unit tests in `src/ir/resource_escape.rs` (Phase 2); codegen
  close-site-count tests in the `tests/` integration harness, modelled on
  `tests/native_resource_scope_drop.rs` (Phase 3).
- Coverage check: measure with `--bin mfb`; confirm the two new scan arms and the
  record branch of the ordering gate are in the denominator. A green gate over
  code no test reaches proves nothing.
- Runtime proof: **not possible in this letter** — the ban is still up. The
  end-to-end runtime proof (open a file into a record, use it after the record is
  copied, confirm one close via a 200-iteration loop that would exhaust fds if the
  drain leaked) lands in letter D and is named there. Do not claim runtime
  verification here.
- Doc sync: `./mfb spec architecture escape-analysis` must gain the record edges —
  it is cited by §15.6 as the authority for the decision procedure, so leaving it
  collection-only makes the spec wrong. `.ai/resources-packages.md` gains the
  "owned-list is representation-independent" fact.
- Acceptance: `cargo test --no-fail-fast` (redirect to a file; check cargo's exit
  status, not a piped `tail`'s); `cargo check --all-targets`;
  `scripts/artifact-gate.sh target/release/mfb all` → `diffs=0`;
  `scripts/test-accept.sh target/release/mfb /tmp/plan114c-scratch`;
  `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Does the ordering rule need a type table?** The bug-291 gate consults
  `decl_type` to skip programs already rejected for a missing `RES` marker
  (`src/ir/resource_escape.rs:130-135`). `Named("Holder")` does not reveal whether
  `Holder` has a resource field. **Recommendation:** thread an optional
  `&HashSet<ParameterType>` of record types carrying a `RES` field into
  `analyze_function`, populated at its one production call site
  (`src/ir/lower.rs:537`), and default it to empty for the `#[cfg(test)]` callers.
  Alternative — key the gate purely on "this routing carried a `RES` element", which
  needs no table but cannot distinguish "already rejected elsewhere". Phase 1 closes
  this.
- **`List OF Holder` where `Holder` carries a resource.** The scan handles it (a
  constructor inside a list literal routes to the list). Whether the *drain* should
  be the list's or the record's is the same question `solve()` already answers for
  nested collections — outermost wins. **Recommendation:** rely on the existing
  outermost-wins fixpoint and add the nesting case as a Phase 2 test rather than new
  logic; if the test shows the fixpoint does not reach it, that is a Phase 2 task,
  not a reason to restrict the feature.

## Corrections

<!-- Filled in during execution. -->

## Summary

The engineering risk is entirely in Phase 3: a missed drain leaks a handle with no
diagnostic. It is scheduled last, behind analysis tests, and gated on close-site
counts rather than exit codes — which is the only thing that can see a leak.
The pleasant surprise measured in §2 is that the owned-list is keyed by binding
name and reads nothing about the container's representation, so a record binding
needs no new runtime structure. Untouched: the `.ir` format, the `ResOwner` enum,
collection behavior, and the front-end ban.
