# plan-63-D: Set documentation, spec, and golden/acceptance close-out

Last updated: 2026-07-25
Effort (Human): large (3h–1d)
Effort (AI): medium (1h–2h)
Depends on: plan-63-C (the full Set surface — type, native members, and algebra —
must be landed and green, because the docs describe behavior that must already be
true and the goldens capture emitted output that must already be stable)
Produces: the `Set OF T` type man page, the man pages for every new
`collections::` member, the spec updates (`language/04_types`,
`language/12_collections`, `memory/05_collections`, `architecture/type-name-encoding`,
`language/19_grammar`), the man-package registration, worked examples, and the
final acceptance/golden pass that proves the whole feature is stable. This is the
last sub-plan; after D, plan-63 is complete and its four files archive to
`planning/old-plans/`.

Prerequisites: see plan-63-A §Prerequisites. D adds: **plan-63-C is complete** —
`grep -c '^FUNC __collections_union\|^FUNC __collections_toSet' src/builtins/collections_package.mfb`
> 0 and the C fixtures are green. If C is not complete, D cannot start, full
stop. D changes no behavior — it documents and locks behavior A/B/C shipped.

References (read first, they are the authoring contract):

- `.ai/man_template.md` (per-function page), `.ai/man_type_template.md` (a type
  topic like `List`/`Map`), `.ai/man_package_template.md` (package overview) —
  the templates AGENTS.md mandates following exactly.
- `scripts/update_man.sh` (function/type pages) and `scripts/update_man_package.sh`
  (package overviews) — the driver scripts holding the authoring rules.
- `.ai/specifications.md` — the obligation to keep `mfb spec` current with every
  compiler change.
- `src/docs/man/types/list.md`, `src/docs/man/types/map.md` — the two existing
  type topics `set.md` mirrors.
- `src/docs/man/mod.rs:29-45` (`PACKAGE_ORDER`) and the man-tree build
  (`man/mod.rs:6,24-28`) — registration.

## 1. Goal

- `mfb man types set` renders a complete `Set OF T` type topic (synopsis,
  literal, storage, comparability/defaultability, mutation, iteration) matching
  `.ai/man_type_template.md`.
- `mfb man collections add` (and `remove`, `toList`, `union`, `intersection`,
  `difference`, `symmetricDifference`, `isSubset`, `isSuperset`, `isDisjoint`,
  and the extended `contains`) each render a complete function page.
- `mfb spec language types`, `mfb spec language collections`,
  `mfb spec memory collections`, `mfb spec architecture type-name-encoding`, and
  `mfb spec language grammar` all describe `Set OF T` accurately and consistently
  with the shipped behavior.
- The man citation tests and spec citation tests pass (symbol-level and
  file-level), and the full acceptance/golden suite is green with the Set
  fixtures' goldens seeded.

### Non-goals (explicit constraints)

- **No behavior change.** D is documentation, registration, examples, and
  goldens. If writing a doc reveals a behavior that is wrong or under-specified,
  that is an A/B/C correction (fix there, note it in that plan's Corrections),
  not a doc that describes a bug as intended.
- **No re-baselining to hide churn.** Goldens are re-seeded only for fixtures
  that newly construct Sets. Any unrelated golden that churns is a real
  regression to investigate, per AGENTS.md "Never edit a test/golden to pass."

## 2. Current State

`List` and `Map` are documented as **type topics** under `src/docs/man/types/`
(`list.md`, `map.md`) — there is no `list`/`map` *package*; their operations live
in the `collections` builtins package (`src/docs/man/builtins/collections/`, one
`.md` per member + `package.md`). The man tree is compiled to `MAN_PACKAGES` by
`build.rs`; each package needs one `PACKAGE_ORDER` row (`man/mod.rs:29-45`) kept
in sync with the generated set (asserted `mod.rs:64-68`).

Set follows the identical split: a new `types/set.md` topic, plus new
`builtins/collections/<member>.md` pages for the Set members — **no new man
package** (Set operations join `collections`, per plan-63-B's namespace decision).
The `types` package already exists, so `types/set.md` needs no new
`PACKAGE_ORDER` row; the `collections` package already exists too, so the new
member pages need no new row either — D's registration work is adding pages to
two existing trees and re-running the sync/citation checks.

Spec source files to update (behavior already shipped in A/B/C):

- `src/docs/spec/language/04_types.md` — §4.7 collection type forms (add
  `Set OF T`), §4.10 defaults (Set defaultable ⇔ element defaultable), §4.11
  comparability (Set not comparable; element must be comparable).
- `src/docs/spec/language/12_collections.md` — the collection operations model;
  add the Set members and the `Set OF T { … }` literal, and the `FOR EACH x IN
  set → T` iteration note.
- `src/docs/spec/memory/05_collections.md` — the block layout; document
  `kind = 3` (Set), `valueType = none`, `valueLength = 0` entries, and that a Set
  carries a bucket index like a Map.
- `src/docs/spec/architecture/22_type-inference.md` and the type-name-encoding
  spec — add `Set OF T` to the canonical grammar table and the round-trip
  prefix-strip list.
- `src/docs/spec/language/19_grammar.md:131-244` — add `Set OF` to `templateType`
  and `Set OF type "{" [exprList] "}"` to the literal grammar.

### Measured populations

| What | Count | Command |
|---|---|---|
| New Set members needing a `collections/*.md` function page | 11 | add, remove, toList, contains(Set overload), union, intersection, difference, symmetricDifference, isSubset, isSuperset, isDisjoint |
| Existing type topics `set.md` mirrors | 2 | `ls src/docs/man/types/{list,map}.md → 2` |
| Spec source files to update | 5 | `04_types.md`, `12_collections.md`, `05_collections.md`, type-name-encoding, `19_grammar.md` |
| Citation test gates that must pass | UNMEASURED — **Task D0** | `grep -rn 'man_citations_resolve\|spec.*citation' src/ tests/` then list the exact test names |

D0 (name the citation gates) is first: per the "Splits must sweep man AND spec
citations" hazard, the man citation test is symbol-level (strict) and the spec
citation test is file-level (lenient), and they are distinct gates — D must know
both names before editing docs so it can run each after.

### Verified properties

- **Behavior D documents is already true.** D depends on C being complete and
  green; every claim a Set man/spec page makes is checked against the A/B/C
  fixtures, not asserted. If a page would need to describe unshipped behavior, the
  dependency gate was violated — stop and finish the prior letter.

## 3. Design Overview

D is mechanical breadth: one type topic, eleven function pages, five spec files,
registration, examples, and goldens — all against a frozen behavior. It lands
**last** because it churns the most generated/checked output (man tree, spec
render, acceptance goldens) and reviewing that churn is only meaningful once the
behavior is fixed. There is no design uncertainty and no correctness risk beyond
"a doc claims something false," which the citation tests and a read-back catch.

Ordered within D: docs+spec first (Phase 1), then registration+examples
(Phase 2), then the golden/acceptance close-out (Phase 3, largest churn last).

## 4. Detailed Design

### 4.1 Type topic `types/set.md`

Follow `.ai/man_type_template.md` exactly, mirroring `types/map.md`: synopsis
(`Set OF T`), literal (`Set OF T { 1, 2, 3 }`, empty `Set OF T { }`, dedup note),
storage (Map-shaped block with a hash index, elements as keys, no values),
comparability (element must be comparable; a Set is not itself comparable),
defaultability (empty set when element defaultable), mutation (`add`/`remove`,
`MUT` in-place idiom), iteration (`FOR EACH x IN set` yields `T` in stable
insertion order), and the `collections::` operation index.

### 4.2 Function pages

One page per §2 member, via `scripts/update_man.sh`, following
`.ai/man_template.md` (synopsis, params, returns, errors, examples). The
`contains` page gains a Set overload section beside its List overload.

### 4.3 Spec updates

Edit the five source files in §2 so `mfb spec` renders Set consistently. Keep the
type-name-encoding grammar table and the round-trip prefix-strip list in lockstep
(that doc is the canonical contract A implemented).

### 4.4 Examples

Add a worked Set example (mirror an existing collections example) demonstrating
construction, membership, and one algebra op — wherever the project keeps
runnable examples (`examples/` per the recent `hello_world` expansion commit).

## Phases

> Keep checkboxes current in the same commit as the work. Fill `Commit:` on land.

### Phase 1 — Docs + spec

One line: author the type topic, the eleven function pages, and the five spec
edits, all against the shipped behavior.

- [ ] D0: identify and record the exact man-citation and spec-citation test
      names (the two distinct gates).
- [ ] `src/docs/man/types/set.md` per `.ai/man_type_template.md` (§4.1).
- [ ] Eleven `src/docs/man/builtins/collections/*.md` pages (§4.2), including the
      `contains` Set overload.
- [ ] Five spec source edits (§2/§4.3), consistent with A/B/C behavior.

Acceptance: `mfb man types set` and `mfb man collections <each new member>`
render fully; `mfb spec` renders the updated Set sections; both citation gates
(from D0) pass.
Commit: —

### Phase 2 — Registration + examples

One line: wire the new pages into the man tree and add a runnable example.

- [ ] Confirm `types` and `collections` `PACKAGE_ORDER` rows already cover the new
      pages (no new package row expected — verify the `mod.rs:64-68` sync
      assertion passes after the tree grows).
- [ ] Add the worked Set example (§4.4) and any example index/registration it
      needs.
- [ ] Tests: the example compiles and runs to its expected output under the
      project's example harness.

Acceptance: the man-tree sync assertion passes; the example runs and prints its
expected output; `cargo test` green.
Commit: —

### Phase 3 — Golden + acceptance close-out (largest churn last)

One line: seed the goldens for every Set fixture/example and prove the full
suite green with no unrelated churn.

- [ ] Seed acceptance goldens for the A/B/C fixtures and the D example
      (`sync-goldens.sh` scoped to the set fixtures first, then the full pass),
      per the acceptance golden harness.
- [ ] Diff the golden set: every churned golden must be a Set fixture/example.
      Any unrelated churn is investigated, not re-baselined (AGENTS.md).
- [ ] Archive: move plan-63-A/B/C/D to `planning/old-plans/` once green.

Acceptance: the full acceptance/CI suite passes; the golden diff contains only
Set-related fixtures/examples; `cargo test` green; plan-63 files archived.
Commit: —

## Validation Plan

- Tests: man-citation gate + spec-citation gate (D0), the example harness, the
  man-tree sync assertion (`man/mod.rs:64-68`).
- Coverage check: the citation gates are the coverage here — a rendered page with
  a dangling symbol reference fails the symbol-level man gate; do not rely on a
  visual read alone.
- Runtime proof: the worked example running to expected output is the end-to-end
  proof that the documented surface behaves as written.
- Doc sync: this whole sub-plan *is* the doc-sync obligation for the feature
  (per `.ai/specifications.md` and AGENTS.md's man-template rules).
- Acceptance: full acceptance/CI (~15 min per the project baseline); confirm the
  golden diff is Set-only.

## Open Decisions

- **Example placement.** Recommend adding to `examples/` alongside the existing
  collections demos (matches the recent 38-language `hello_world` expansion). Use
  the project's canonical example location if it differs — verify before Phase 2.Decision: Add to the benchmark code, no example.

## Corrections

<Filled in during execution.>

## Summary

D is the documentation-and-lock close-out: a `Set` type topic, eleven
`collections` function pages, five spec files, a worked example, and the final
golden/acceptance pass. It lands last because it churns the most checked output
against a frozen behavior. The only failure mode is a doc that describes
something A/B/C did not ship — caught by depending on C's completion and by the
two citation gates.
