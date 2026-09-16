# plan-138-E: `xml` XPath oracles, final validation, and archive

Last updated: 2026-09-15
Effort: medium (1h–2h)
Depends on: plan-138-D (Prerequisites: `planning/plan-138-A-xml-tree-builder-and-reader.md`)

Extends the plan-138-C oracle with XPath on both peers, runs the whole feature's final gate, and
archives plan-138. Outcome: `node packages/xml/oracle/diff.mjs` exits 0 across every mode, including
`xpath` and `fuzz-xpath`, with the package, the Node oracle and the Rust oracle agreeing on every
expression in plan-138-D's subset.

References:

- plan-138-C §3 (layout, envelope, equal-peer rule) and plan-138-D §4–§5 (subset, result kinds).
- W3C XPath 1.0.

## Prerequisites

See plan-138-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-138-D complete | `ls planning/completed/plan-138-D-*` → one file | MET (re-measured 2026-09-15 → `planning/completed/plan-138-D-xml-xpath-subset.md`; archived in `d5df8a6c7`) |

## 1. Goal

- `node packages/xml/oracle/diff.mjs` → exit 0 for all modes; `target/release/mfb test packages/xml` →
  exit 0; `packages/xml/check-doc-examples.sh` → exit 0; plan-138-A…E archived.

### Non-goals (explicit constraints)

- No change to plan-138-D's subset or API. A disagreement is resolved by fixing the side that is
  wrong per XPath 1.0 (a package fix gets a TESTING case), or by a `divergences.json` entry citing the
  spec section — never by narrowing the corpus.
- No namespace-URI matching. Oracles are given a resolver so their prefix tests line up with the
  package's literal-prefix rule (§3).

## 2. Current State

- plan-138-C's oracle has no XPath.

### Measured populations

| What | Value | Command |
|---|---|---|
| npm `xpath` | 0.0.34 | `npm view xpath version` |
| `sxd-xpath` last release | 0.4.2, updated 2018-10-31 | `curl -s https://crates.io/api/v1/crates/sxd-xpath` → `updated_at` |
| `xpath-eval` | 0.2.2, MIT, "generic over the document model" | `cargo info xpath-eval` |
| `xrust` | 2.2.0, updated 2026-07-07 | `cargo info xrust`; crates.io API `updated_at` |

### Verified properties

- **`xpath-eval` is modeled on `roxmltree::Node`** (`xpath-eval-0.2.2/src/document.rs:31`, doc
  comment "handles — modeled after `roxmltree::Node<'a>`"). It ships no roxmltree adapter
  (`grep -rn roxmltree` over the crate finds only that comment). **Whether a roxmltree adapter
  compiles and evaluates correctly is UNVERIFIED** — Phase 1.
- **`xrust` claims XPath 1.0 functional equivalence** (`xrust-2.2.0/README.md`: "achieved the
  functional equivalent of XPath 1.0"). Its goal is XPath 3.1, so 1.0-vs-3.1 semantic differences
  (e.g. number formatting) are a risk if it is chosen.

## 3. Design Overview

- **Rust:** `xmloracle xpath <job.json>` — parse with the same roxmltree options as `read`, evaluate
  each expression with `xpath-eval` through a `Document` trait adapter over `roxmltree::Node`.
- **Node:** `oracle.mjs xpath` — build an xmldom `Document` from saxes events (never xmldom's own
  parser), evaluate with npm `xpath`.
- **Prefix alignment:** the package matches `p:name` literally. Each oracle gets a resolver mapping
  every prefix to the URI declared for it in the document. The XPath corpus and the fuzz generator
  never bind one prefix to two different URIs within a document, so literal and resolved matching
  select the same nodes. A document that breaks this rule is outside the mode.
- **Envelope:**
  `{"ok":true,"kind":"nodes","value":[<content node>...]}` (plan-138-C projection per node, document
  order); `"attributes"` → `[[name,value],...]` in document order; `"string"` → text; `"number"` →
  the XPath `string()` of the number (so all three format per §4.2 and exact float text is compared);
  `"boolean"`; or `{"ok":false,...}`. Both refusing agrees, as in C.

**Risk:** the Rust oracle choice (Phase 1, first). Rejected: `sxd-xpath`, with no release since
2018-10-31 (table above). Equal peers with a stale engine would make every disagreement suspect.

**Gate class:** new tooling plus fixes. The gate is exit status.

## Compatibility / Format Impact

Tooling only, plus package fixes behind TESTING cases.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the work; `- [~]`
> partial; moot tasks struck through with evidence; fill `Commit:`. **Unticked means NOT DONE.**

### Phase 1 — Rust XPath engine

- [x] Add `xpath-eval = "0.2.2"` to `packages/xml/oracle/rust/Cargo.toml`; implement its document
      trait over `roxmltree::Node`; `xmloracle xpath` evaluating `count(//a)`, `//a[@x='1']`,
      `string(1 div 0)` on a fixture.
- [x] If the adapter cannot express the plan-138-D subset (missing node kinds or functions), record the
      evidence under Corrections, resolve the Open Decision below, and implement the chosen engine
      instead. Nothing ships with two engines.
      **Not needed — the adapter works, and the Open Decision resolves to `xpath-eval`.** It compiled
      on the first attempt and answered every probe correctly. `xrust` is not used.

Acceptance: the Rust oracle answers the three probe expressions with XPath 1.0 results.
  Check: `cargo run --release --manifest-path packages/xml/oracle/rust/Cargo.toml -- xpath <fixture job>`
  → `3`-style exact expected envelope for each (est. 3 min).
  MET. `cargo build --release --manifest-path packages/xml/oracle/rust/Cargo.toml` → `Finished`, then
  `./packages/xml/oracle/rust/target/release/xmloracle xpath /tmp/xpath-job.json` →
  `count(//a)` → `{"kind":"number","value":"2"}`;
  `//a[@x='1']` → `{"kind":"nodes","value":[["e","a",[["x","1"]],[["t","one"]]]]}`;
  `string(1 div 0)` → `{"kind":"string","value":"Infinity"}`;
  and two more asked at the same time — `//a/@x` → `{"kind":"attributes","value":[["x","1"],["x","2"]]}`
  and `string(//a)` → `{"kind":"string","value":"hi"}`.
Commit: —

### Phase 2 — Node XPath and `xpath` mode

- [x] `packages/xml/oracle/package.json` — add `xpath ^0.0.34`; refresh `package-lock.json`.
- [x] `oracle.mjs xpath` per §3; `probe/src/main.mfb xpath` calling `xml::evaluate`.
- [x] `diff.mjs xpath` over `corpus/xpath/*.json` (`{"doc": "<file in corpus/>", "exprs": [...]}`)
      with at least one expression per plan-138-D §4 row, plus the Phase 3 evaluator edge cases from
      plan-138-D. Four job files over a new namespace-free `corpus/catalog.xml` — `paths.json` (33
      expressions), `predicates.json` (33), `operators.json` (42), `functions.json` (40) — 148 in all.
- [x] Fix every package defect with a failing case first in `packages/xml/src/test_xpath_eval.mfb`;
      add rows to the oracle README "What it found". Two package defects found and fixed (see
      Corrections); the package suite went 249 → 252.

Acceptance: the XPath corpus agrees three-way.
  Check: `node packages/xml/oracle/diff.mjs xpath` → exit 0 (est. 1 min).
  MET: `node packages/xml/oracle/diff.mjs xpath` → `ok   xpath: 148 case(s) agreed three ways,
  2 declared divergent`, exit 0. The first run disagreed on 6; four were defects, now fixed, and the
  remaining two are one declared divergence recorded with its measurement. This is also the first
  time the divergence machinery has been used at all — letters C and D resolved every disagreement
  by fixing a side.
Commit: —

### Phase 3 — `fuzz-xpath`

- [ ] `diff.mjs fuzz-xpath` — for each random tree from plan-138-C's generator, generate expressions
      from names, attribute values and text actually present (paths, predicates, comparisons,
      functions from plan-138-D §4), seeded and replayable.
- [ ] Fix defects as in Phase 2.

Acceptance: random queries agree exactly.
  Check: `node packages/xml/oracle/diff.mjs fuzz-xpath --count 2000` → exit 0 (est. 3 min).
Commit: —

### Phase 4 — final gate and archive

- [ ] Run the Validation Plan's final gate; record results here.
- [ ] `packages/xml/oracle/README.md` — XPath section and final "What it found".
- [ ] `git mv planning/plan-138-A-xml-tree-builder-and-reader.md planning/plan-138-B-xml-serializer-and-docs.md planning/plan-138-C-xml-node-and-rust-oracles.md planning/plan-138-D-xml-xpath-subset.md planning/plan-138-E-xml-xpath-oracles-and-validation.md planning/completed/`
      (each earlier letter is archived as it completes. This task covers E, plus any letter not yet
      moved.)

Acceptance: the whole feature's gate passes.
  Check: the final gate below → all exit 0.
Commit: —

## Validation Plan

- Tests: `packages/xml/src/test_*.mfb` (every letter's).
- Coverage check: `target/release/mfb test --coverage packages/xml` → `coverage.html`; no reader,
  writer or XPath refusal branch is unexercised.
- Runtime proof: `node packages/xml/oracle/diff.mjs` (all modes) with both oracles as equal peers.
- Doc sync: `packages/xml/README.md`, `packages/xml/doc.html`, `packages/xml/oracle/README.md`,
  `planning/todo.md` row 6.
- Final gate (run ONCE, here):
  1. `cargo build --release` → `Finished` (est. 3 min; the probe must be built by the current compiler).
  2. `target/release/mfb test packages/xml` → exit 0 (est. 1 min).
  3. `packages/xml/check-doc-examples.sh` → exit 0 (est. 3 min).
  4. `packages/xml/oracle/fetch-xmlconf.sh && node packages/xml/oracle/diff.mjs` → exit 0 over
     `corpus xmlconf fuzz-read fuzz-write roundtrip mutate perf xpath fuzz-xpath` (est. 15 min; this
     is the feature's end-to-end proof and runs once).
  5. `cargo test` — **only if** any letter changed `src/` through a bug fix; otherwise not run, because
     no compiler code changed (est. per `.ai/testing-gates.md`).

## Open Decisions

- Rust XPath engine — **`xpath-eval` over roxmltree** (XPath 1.0-specific, small adapter, same parser
  as the `read` oracle) vs. **`xrust`** (maintained, claims XPath 1.0 equivalence, but targets 3.1 and
  brings its own tree and parser). Recommend `xpath-eval`; Phase 1 proves or disproves it. (§3)

## Corrections

**Phase 1 — the Open Decision resolves to `xpath-eval`, and the adapter was easier than feared.**
§3 listed as UNVERIFIED whether a roxmltree adapter compiles and evaluates correctly. It does, and it
compiled on the first attempt. One thing the plan's "modeled on `roxmltree::Node`" note hides: the
adapter cannot BE a `roxmltree::Node`. XPath's data model (§5) makes attributes and namespaces nodes
of the same kind as elements, while roxmltree keeps attributes in a separate type and namespaces in a
scope list — so a handle is a `Copy + Eq` enum of "tree node", "attribute of an element by index" and
"namespace of an element by index", with `document_order` keyed on `(node id, class, index)` so an
element sorts before its namespaces, then its attributes, then its children. `xrust` is not used, and
nothing ships with two engines.

**Phase 2 — the XPath corpus is namespace-free, and that is a scope decision with a reason.** §3
proposed giving each oracle a resolver so prefix tests line up with the package's literal matching.
That bridges a PREFIXED name, but it cannot bridge a DEFAULT namespace: XPath 1.0's unprefixed name
test matches only no-namespace elements, so on a document with `xmlns="urn:x"` both oracles correctly
select nothing for `//book` while this package — which never resolves a prefix — matches the literal
name and selects every book. That is a difference of design, not a defect on either side, and no
resolver removes it. The mode therefore runs over documents with no namespace declarations, as §3's
own "a document that breaks this rule is outside the mode" anticipates.

**Phase 2 — two package defects, both about unions of attributes.** The oracle found them together:

- `//book/@id | //book/@year` was refused as "a union needs a node-set". Two attribute sets union like
  any others; only a union that MIXES attributes with elements is refused, which the package still
  does.
- Once it answered, it answered in the wrong ORDER — all the `id`s, then all the `year`s — because it
  appended one set after the other. A union yields a node-set, and a node-set is in document order,
  so the two interleave. Fixed with ordered insertion keyed on (owner element, attribute position),
  and pinned by a case that asserts the order rather than the count.

**Phase 2 — two harness defects.** The Node oracle's DOM kept whitespace text nodes from OUTSIDE the
document element, so `//text()` and `//node()` returned two more nodes than the other two sides;
XPath 1.0 §5.1 gives the root node no text children. And selecting the document node itself rendered
three different ways although all three had selected the same node — now reported as the document
element everywhere, which is what the package does.

**Phase 2 — the feature's only declared divergence: MFBASIC prints a Float with two decimals.**
XPath 1.0 §4.2 asks for as many digits as distinguish a double from its neighbours, so
`sum(//price)` over 9.99, 12.50 and 8.25 is `30.740000000000002` — which both oracles produce. The
package says `30.74`. Measured directly rather than inferred: a program printing
`toString(30.740000000000002)` outputs `30.74`, and `toString(1.0/3.0)` outputs `0.33`; there is no
float-formatting function with a precision argument (`mfb man strings` has only padding, `mfb man
math` only `round`/`ceil`/`floor`). The arithmetic agrees — only the text form differs — so this is a
limit of the language's number-to-text conversion. Recorded in `divergences.json` with that evidence,
and in `packages/xml/README.md` so a caller meets it in the documentation rather than in a surprise.

## Summary

The risk is the Rust XPath engine (settled first) and oracle disagreement on XPath edge semantics,
resolved by citing the spec rather than a vote. Final validation and archive close plan-138.
