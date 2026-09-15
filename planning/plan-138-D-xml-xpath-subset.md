# plan-138-D: `xml` package — XPath 1.0 subset

Last updated: 2026-09-15
Effort: large (3h–1d)
Depends on: plan-138-C (Prerequisites: `planning/plan-138-A-xml-tree-builder-and-reader.md`)

Adds XPath to `packages/xml`. The only query language is an XPath 1.0 subset. Outcome: every
expression in the §4 grammar evaluates per XPath 1.0 semantics against a `Document`. Anything
outside the subset fails with `ErrUnsupported` rather than being misread, and a query over a
100k-node document stays within the budget measured in Phase 1.

References:

- W3C XML Path Language (XPath) Version 1.0 — §2 location paths, §2.4 predicates, §3.4 comparisons
  (node-set existential semantics), §4 core functions, §4.2 `string()` of numbers.
- plan-138-A §5 (tree model) and Verified properties (copy-on-`get`; consumer recursion works).

## Prerequisites

See plan-138-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-138-C complete | `ls planning/completed/plan-138-C-*` → one file | NOT MET |

## 1. Goal

- `mfb test packages/xml` passes with an XPath test file covering every §4 grammar row and function,
  plus refusal cases; and `xml::select(doc, "//item[@id='49999']")` on the 100k flat shape returns
  the one element in ≤ 3 s including parse.

### Non-goals (explicit constraints)

- No XPath 2.0/3.x, no variables (`$x`), no namespace axis, no unabbreviated axis syntax
  (`child::`, `following-sibling::` …), no `id()`, no `lang()`.
- No namespace resolution: `p:name` matches the literal name `p:name` (plan-138-A §1).
- No change to the tree model or the reader/writer.

## 2. Current State

- No query support. Consumers can recurse over `xml::Node` themselves (plan-138-A Verified
  properties).
- The tree has no parent pointers. Records are immutable values, and `collections::get` returns a
  deep copy (`.ai/collections.md`, bug-538). So `..`, document order across branches, and returning
  the matched subtrees all need a design that avoids copying the tree per step.

### Measured populations

| What | Value | Command |
|---|---|---|
| Evaluation-index build and node materialization cost at 100k nodes | UNMEASURED | Phase 1 measures it before any other phase |

### Verified properties

- **`XNumber.value` is `Float`**, the type `json::JsonNum.value` carries (`mfb man json types` →
  `value │ Float │ A JSON number, held as a double-precision float.`).
- **The `XPathValue` union and a consumer's `CASE XNodes(x)` compile across the package boundary**
  (plan-138-A Verified properties, `xmlcheck` probe → `xnodes 3`).

## 3. Design Overview

Three pieces:

1. **Parser** (`src/xpath_parse.mfb`) — expression text → AST records. Syntax errors →
   `ErrInvalidFormat`; recognized-but-excluded syntax (axes `::`, `$var`, excluded functions) →
   `ErrUnsupported`.
2. **Index** (`src/xpath_index.mfb`) — one iterative pass flattens the `Document` into
   `List OF IndexedNode` records in document order: `kind`, `name`, `value`, `parent AS Integer`,
   `firstChild`, `nextSibling`, `depth`, `attrStart`/`attrCount` into a parallel
   `List OF IndexedAttr`. No subtrees are stored. Document order is index order, so sorting and
   de-duplicating a node-set is sorting integers.
3. **Evaluator** (`src/xpath_eval.mfb`) — node-sets are `List OF Integer`. Only the final result is
   materialized back into `Node` values.

**Design uncertainty concentrates in materialization.** Turning index `i` back into a `Node`
without copying the whole document per result is unproven. Phase 1 measures candidates:
(a) rebuild each result subtree from the flat index, which copies only what is returned;
(b) walk down from the root by child ordinals, which copies the root subtree on each `get`, so it
is expected to be slow and is measured to confirm. The cheapest one meeting the budget is chosen.

**Correctness risk concentrates in semantics:** position predicates are evaluated per step
(`//a[1]` vs `(//a)[1]`), comparisons between node-sets are existential, and `string()` must format
numbers per §4.2. plan-138-E's three-way oracle pins them.

**Gate class:** new behavior. The gates are tests and the measured budget.

Rejected: a tree walker over the `Node` union with parent stacks — every `..` and every cross-branch
document-order sort would re-walk from the root, and collecting results by `get` copies subtrees.

## 4. Supported subset

| Construct | Examples |
|---|---|
| Absolute / relative paths | `/a/b`, `a/b`, `.`, `..` |
| Descendant-or-self abbreviation | `//a`, `a//b`, `.//b` |
| Name tests | `name`, `p:name` (literal), `*` |
| Node-type tests | `text()`, `node()`, `comment()`, `processing-instruction()` |
| Attributes | `@name`, `@*` |
| Predicates | `[1]`, `[last()]`, `[position() < 3]`, `[@a]`, `[@a='v']`, `[b]`, `[b='v']`, nested and chained `[..][..]` |
| Filter expression | `(//a)[1]` |
| Operators | `or`, `and`, `=`, `!=`, `<`, `<=`, `>`, `>=`, `+`, `-`, `*`, `div`, `mod`, unary `-`, `|` |
| Literals | `'…'`, `"…"`, numbers |
| Functions | `count()`, `contains()`, `starts-with()`, `string()`, `normalize-space()`, `not()`, `position()`, `last()`, `name()`, `local-name()` (text after the `:`), `concat()`, `string-length()`, `number()`, `sum()`, `true()`, `false()`, `boolean()` |

`count`, `contains`, `starts-with`, `string`, `normalize-space`, `not` and `last` were agreed in
discussion. The rest are listed because the agreed predicates need them (`position()` for `[1]`,
`number()` and `boolean()` for §3.4 comparisons, `name()`/`local-name()` for prefix-literal
matching). `concat`, `string-length`, `sum`, `true`/`false` are cheap and appear constantly in real
queries. **Open Decision** if any should be cut.

## 5. API

```
EXPORT TYPE XNodes      nodes AS List OF Node
EXPORT TYPE XAttributes attributes AS List OF Attribute
EXPORT TYPE XString     value AS String
EXPORT TYPE XNumber     value AS Float
EXPORT TYPE XBoolean    value AS Boolean
EXPORT UNION XPathValue XNodes XAttributes XString XNumber XBoolean

xml::evaluate(doc AS Document, expr AS String) AS XPathValue
xml::select(doc AS Document, expr AS String) AS List OF Node              ' fails ErrInvalidArgument if not an element/text/comment/PI node-set
xml::selectAttributes(doc AS Document, expr AS String) AS List OF Attribute ' fails ErrInvalidArgument if not an attribute node-set
xml::valueOf(doc AS Document, expr AS String) AS String                  ' XPath string() of any result
```

A node-set mixing attributes with other nodes (`@a | b`) → `ErrUnsupported`. The context node is
always the document node.

## Compatibility / Format Impact

Adds §5. Nothing earlier changes.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the work; `- [~]`
> partial; moot tasks struck through with evidence; fill `Commit:`. **Unticked means NOT DONE.**

### Phase 1 — index and materialization spike

- [ ] In a `/tmp` consumer of `packages/xml/xml.mfp`, build the §3 index iteratively for the three
      plan-138-A Phase 1 shapes at 100k nodes; measure build time (3 runs).
- [ ] Measure materialization candidates (a) and (b) for: one deep node; 1,000 scattered elements;
      all 50,000 `item` elements.
- [ ] Record numbers and the chosen candidate in Corrections; replace the UNMEASURED row above. If
      none meets "parse + index + materialize one element ≤ 3 s", localize the copying operation with
      `mfb build --debug`'s arena report and fix the compiler defect under its own bug document
      before Phase 2 (AGENTS.md).

Acceptance: a materialization design is chosen with its measured cost recorded.
  Check: the `/tmp` consumer's output for all measured cases, 3 runs each, with the chosen design ≤ 3.00 s
  for parse + index + one-element materialization (est. 5 min).
Commit: — (plan update only)

### Phase 2 — parser

- [ ] `packages/xml/src/xpath_parse.mfb` — tokenizer (XPath 1.0 §3.7 lexical rules, including the
      `*`/`div`/`mod` operator-vs-name disambiguation) and a precedence-climbing parser for §4 →
      AST records.
- [ ] Tests: `packages/xml/src/test_xpath_parse.mfb` — each §4 row parses to the expected AST; syntax
      errors → `ErrInvalidFormat`; `child::a`, `$x`, `id('x')` → `ErrUnsupported`.

Acceptance: the grammar and its refusals are pinned.
  Check: `target/release/mfb test packages/xml` → all pass (est. 1 min).
Commit: —

### Phase 3 — index and evaluator

- [ ] `packages/xml/src/xpath_index.mfb` — the §3 index.
- [ ] `packages/xml/src/xpath_eval.mfb` — location steps over indices, per-step predicate position
      (reverse order for `..`'s parent step is trivially a single node), filter expressions, union in
      document order, §3.4 comparisons, §4 functions, §4.2 number-to-string.
- [ ] `packages/xml/src/lib.mfb` — §5 API with `DOC` blocks and one `EXAMPLE` each for `select` and
      `valueOf`.
- [ ] Tests: `packages/xml/src/test_xpath_eval.mfb` — every §4 row against a fixture document with
      the exact expected result; `//a[1]` vs `(//a)[1]`; `@a = 'x'` over several `a` elements
      (existential); `string(1 div 0)` = `Infinity`, `string(0 div 0)` = `NaN`, `string(2.0)` = `2`;
      mixed node-set → `ErrUnsupported`; `select` over a string result → `ErrInvalidArgument`.

Acceptance: every subset row evaluates to its XPath 1.0 result.
  Check: `target/release/mfb test packages/xml` → all pass (est. 1 min).
Commit: —

### Phase 4 — budget and docs

- [ ] Measure `xml::select(doc, "//item[@id='49999']")` and `xml::valueOf(doc, "count(//item)")` on
      the 100k flat shape through a `/tmp` consumer; record here.
- [ ] `packages/xml/README.md` — XPath section: the §4 table, the §5 API, the literal-prefix rule.
- [ ] `packages/xml/doc.html` — regenerate with `target/release/mfb pkg doc packages/xml/xml.mfp --out packages/xml/doc.html`.

Acceptance: the query budget holds and the docs describe the subset.
  Check: `/tmp` consumer → each query, parse included, ≤ 3.00 s over 3 runs;
  `packages/xml/check-doc-examples.sh` → exit 0 (est. 4 min).
Commit: —

## Validation Plan

- Tests: `test_xpath_parse.mfb`, `test_xpath_eval.mfb`.
- Coverage check: `target/release/mfb test --coverage packages/xml`; every function and operator
  branch in `xpath_eval.mfb` is hit.
- Runtime proof: the Phase 4 `/tmp` consumer on 100k nodes.
- Doc sync: `packages/xml/README.md`, `packages/xml/doc.html`, `DOC` blocks.
- Final gate: runs once at the end of plan-138-E.

## Open Decisions

- Function list beyond the agreed seven — **keep the §4 list** vs. cut to exactly the agreed seven
  plus the ones predicates cannot work without (`position`, `number`, `boolean`). Recommend keep.
  Each is a few lines and all are in XPath 1.0 §4. (§4)
- Attribute results — **separate `selectAttributes`** vs. adding an `Attribute` variant to
  `xml::Node`. Recommend separate: an `Attribute` in `Node` would let a user put one in `children`,
  which has no XML meaning. (§5)

## Corrections

<Filled in during execution.>

## Summary

The risk is materializing results from an immutable, copy-on-read tree fast enough (measured first)
and exact XPath 1.0 semantics (pinned by tests here and three-way in plan-138-E). The reader, writer
and tree are untouched.
