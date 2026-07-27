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

- [x] D0: the two gates are `man_citations_resolve` (`src/docs/man/mod.rs`,
      symbol-level, strict) and `spec_citations_resolve` (`src/docs/spec/mod.rs`,
      file-level, lenient). Both pass.
- [x] `src/docs/man/types/set.md` — mirrors `types/map.md` (the bespoke type-topic
      shape, NOT the record `man_type_template.md`): Synopsis/Description/Literals/
      Elements/storage/Copying/Mutation/Iteration/See-also.
- [x] Twelve `src/docs/man/builtins/collections/*.md` pages: add, remove, toList,
      **toSet** (added for parity — see Corrections), union, intersection,
      difference, symmetricDifference, isSubset, isSuperset, isDisjoint, plus the
      `contains` Set overload. All citations grep-verified; the symbol-level gate
      passes.
- [x] Five spec edits: `language/04_types` (§4.7/4.10/4.11),
      `language/12_collections`, `memory/05_collections` (kind 3),
      `architecture/21_type-name-encoding` (round-trip prefix list),
      `language/19_grammar` (`templateType` + `setLit`).

Acceptance: `mfb man types set` and `mfb man collections <each new member>`
render fully; `mfb spec` renders the updated Set sections; both citation gates
(from D0) pass.
Commit: 133132f22

### Phase 2 — Registration + examples

One line: wire the new pages into the man tree and add a runnable example.

- [x] `types` and `collections` `PACKAGE_ORDER` rows already cover the new pages —
      no new package added, so the `man/mod.rs` sync assertion holds; the docs
      render/sync tests (27) pass.
- [x] Set demonstrated in the **benchmark code** (D decision, not `examples/`):
      `benchmark/mfb/src/setops.mfb` — `test_set_build` (add + contains hash path)
      and `test_set_ops` (union/intersection/difference/symmetricDifference/
      isSubset/isSuperset/isDisjoint/remove/toSet/toList), wired into `main.mfb`.
- [x] The benchmark builds and both rows run correctly (`set_build=20000`,
      `set_ops=6006`).

Acceptance: the man-tree sync assertion passes; the example runs and prints its
expected output; `cargo test` green.
Commit: 133132f22

### Phase 3 — Golden + acceptance close-out (largest churn last)

One line: seed the goldens for every Set fixture/example and prove the full
suite green with no unrelated churn.

- [x] Goldens seeded: the two Set rt-behavior fixtures (`set-behavior-rt`,
      `set-algebra-rt`) with release; they PASS `test-accept.sh <release>`. Five
      compile-only syntax fixtures that pin the two diagnostic messages I widened
      (FOR EACH; len/isEmpty/isNotEmpty) re-seeded.
- [x] Golden diff investigated. Two churn classes, both explained:
      (1) my 5 message fixtures + 2 Set fixtures — expected, re-seeded/pass;
      (2) ~51 pre-existing plan-67 perf-table mismatches — NOT plan-63 (see
      Corrections). `cargo test` full suite green; `artifact-gate.sh` (codegen
      byte-identity) run as the deterministic codegen gate.
- [x] Archive: plan-63-A/B/C/D moved to `planning/old-plans/` (this close-out commit).

Acceptance: the full acceptance/CI suite passes; the golden diff contains only
Set-related fixtures/examples; `cargo test` green; plan-63 files archived.
Commit: 8f7e84925 (re-seed) + the archive commit

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

- **`toSet` was missing from the 11-page list.** §2's "11 New Set members" table
  and Phase 1 both omitted `toSet`, yet it is a shipped C source generic
  (`__collections_toSet`) and `mfb man collections toSet` would 404. Every sibling
  generic (`sort`, `distinct`, …) has a page, so a 12th page (`toSet.md`) was
  authored for parity. The real count is **12** collections pages (+ the `contains`
  overload edit), not 11.
- **The type topic follows `types/map.md`, not `.ai/man_type_template.md`.** Phase 1
  cited the record `man_type_template.md`, but `list.md`/`map.md` are a distinct
  bespoke type-topic shape (Synopsis/Literals/storage/Copying/Mutation/…), which is
  what a `Set` topic needs. `set.md` mirrors `map.md`.
- **The "full acceptance = 0 test-accept mismatches" gate is not literally
  attainable at baseline — plan-67 perf goldens are non-deterministic.** Running
  `scripts/test-accept.sh <release>` reports ~51 mismatches in fixtures UNRELATED
  to plan-63 (map/list codegen `.ncode`, control-flow, project-entry, app-mode,
  parser-hello-world, …). Root cause: plan-67 injects a debug-gated
  `_mfb_rt_perf_*` table; those fixtures' goldens were seeded with a *debug* mfb,
  so their `.ncode` dumps carry perf symbols and their `build.log`s carry a perf
  table with **run-varying nanosecond timings** (e.g. `program 1 37000` in the
  golden vs `29000` on rerun). A release mfb strips the perf table entirely; a
  debug mfb reproduces it with different timings — so neither profile makes those
  goldens diff clean. This reproduces verbatim at the fork base `b2227871a` and is
  present identically on `main` (`git show` both), so it is a pre-existing plan-67
  golden-hygiene issue, **not** a plan-63 regression, and re-seeding 51 unrelated
  goldens is out of scope. The plan-63 acceptance evidence is instead: (a) full
  `cargo test` green (behavior + IR + citation gates), (b) `artifact-gate.sh`
  codegen byte-identity green, (c) both Set rt-behavior fixtures pass
  `test-accept.sh <release>` (their release goldens are deterministic — no perf
  table). The Set fixtures were seeded with release precisely so they stay
  deterministic, unlike the pre-existing perf-bearing goldens.
- **6 pre-existing byte-identity DIFFs inherited from `main` (not plan-63).**
  After merging `main` (advanced from `b2227871a` to `c3eb10afe`), `artifact-gate.sh`
  reports 6 DIFFs: `{audio,http,json,net,regex,strings}_codegen_cover_rt.macos-aarch64.ncode`
  (sha256). Verified pre-existing: a detached worktree at pure `main` `c3eb10afe`
  (no plan-63) builds `strings_codegen_cover_rt` to the SAME sha
  `a51a6419…838…559f`, which ALSO differs from the committed golden `72bda985…739d`.
  So main's `codegen_cover` `.ncodesum` goldens are stale on this macOS host
  (regen'd elsewhere / different profile), independent of plan-63. Plan-63's own
  gate was 0 diffs against the fork-base goldens pre-merge, and the merged codegen
  for these non-Set fixtures is byte-identical to pure-main. Not a plan-63
  regression; flagged for the main baseline.
- **No goldens needed re-seeding for A (front-end only).** Phase 3 says "seed
  goldens for the A/B/C fixtures"; A shipped no fixtures (unit tests only), and
  B/C's goldens were seeded in their own sub-plans. Phase 3 here is the full-suite
  green + Set-only-churn check + archive.

## Summary

D is the documentation-and-lock close-out: a `Set` type topic, eleven
`collections` function pages, five spec files, a worked example, and the final
golden/acceptance pass. It lands last because it churns the most checked output
against a frozen behavior. The only failure mode is a doc that describes
something A/B/C did not ship — caught by depending on C's completion and by the
two citation gates.
