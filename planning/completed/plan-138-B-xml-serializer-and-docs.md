# plan-138-B: `xml` package — `xml::stringify` (compact and pretty) and documentation

Last updated: 2026-09-15
Effort: large (3h–1d)
Depends on: plan-138-A (Prerequisites: `planning/plan-138-A-xml-tree-builder-and-reader.md`)

Adds the writer. The outcome: for every `Document` or `Node` the package can build,
`xml::stringify` produces well-formed XML whose **content** (plan-138-A §4) equals the input's, in
compact form or pretty-printed; pretty-printing changes formatting only. Also lands the package's
user-facing documentation.

References:

- plan-138-A §4 (content vs. formatting) and §5 (tree model) — the definitions used here.
- `src/docs/spec/stdlib/04_json.md`, "Stringify output form" — the overload shape and clamping
  rules mirrored here.
- `packages/yaml/README.md`, `packages/yaml/src/lib.mfb` `DOC` blocks,
  `packages/mustache/check-doc-examples.sh` — the documentation precedents.

## Prerequisites

See plan-138-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-138-A complete | `ls planning/completed/plan-138-A-*` → one file | MET (re-measured 2026-09-15 → `planning/completed/plan-138-A-xml-tree-builder-and-reader.md`; archived in `2281f1932`) |
| bug-631 fixed: an imported overloaded call resolves when its argument is a field of an imported record | `ls bugs/completed/bug-631-*` → one file; and a consumer of `packages/xml` with `FOR EACH n IN doc.children` / `io::print(xml::stringify(n))` → `mfb build` prints `Wrote executable` | MET (re-measured 2026-09-15, both halves). `ls bugs/completed/bug-631-*` → `bugs/completed/bug-631-imported-overload-ambiguous-on-imported-record-field-argument.md`. The deferred consumer check now runs against the real package (`/tmp/xml-consumer`, importing the built `xml.mfp`): `mfb build app` → `Wrote executable to app/build/xmlconsumer.out`, and running it prints `<!--note-->`, `<a><b id="1"/></a>`, `<?p d?>` for `FOR EACH n IN doc.children` / `xml::stringify(n)`, the pretty forms for `xml::stringify(n, 2)`, and the whole document for `xml::stringify(doc)`. So the untyped loop variable bound to a field of an imported record resolves against both imported overloads, which is the bug-631 shape. |

## 1. Goal

- `mfb test packages/xml` passes with serializer tests in which, for every fixture document `d`,
  `content(parse(stringify(d))) = content(d)` and `content(parse(stringify(d, 2))) = content(d)`;
  and `packages/xml/check-doc-examples.sh` compiles and runs every `DOC` example.

### Non-goals (explicit constraints)

- No change to plan-138-A's tree model or reader behavior.
- Output is always UTF-8. No encoding option, no XML 1.1.
- No canonical XML (C14N) mode. Attribute order is written in document order, not sorted.
- Comments and PIs are written when present, but no test requires them to survive (they are not
  content).

## 2. Current State

- plan-138-A lands the tree and reader; nothing writes XML.
- `json::stringify` has `stringify(value)` (compact), `stringify(value, count)` and
  `stringify(value, indent)`; "a count clamps to `0..=10` and a string indent is truncated to its
  first 10 characters; `0` and `""` mean compact and are byte-identical to the one-argument form"
  (`src/docs/spec/stdlib/04_json.md`, "Stringify output form").
- A package can export same-named overloads (plan-138-A Verified properties, `xmlcheck` probe). The
  API is `stringify(doc AS Document)`, `stringify(doc AS Document, count AS Integer)`,
  `stringify(doc AS Document, indent AS String)`, and the same three over `n AS Node`. A consumer
  calling one on an untyped `FOR EACH` variable hits the imported-overload ambiguity bug, which is
  a prerequisite below.
- Doc tooling: `mfb pkg doc <pkg>.mfp --out <file>` writes the HTML reference
  (`src/cli/help.rs`: "--out <file> Path to the generated HTML file (default: doc.html)");
  `packages/mustache/check-doc-examples.sh` extracts and runs each `EXAMPLE … END EXAMPLE` from
  `src/lib.mfb`.

### Measured populations

| What | Value | Command |
|---|---|---|
| packages with a `check-doc-examples.sh` | 4 (cli, json_schema, logger, mustache) | `ls packages/*/check-doc-examples.sh` |
| packages with a tracked `doc.html` | **0** — the plan's "6" counted files on disk, not tracked ones | `git ls-files packages/*/doc.html` → (no output). `ls packages/*/doc.html \| wc -l` counts generated artifacts, which is how the 6 was arrived at. |

### Verified properties

- **Why escaping must go beyond `&` and `<`.** A reader applies §2.11 line-end normalization and
  §3.3.3 attribute-value normalization. A literal CR in text, or a literal tab/LF/CR in an attribute
  value, would come back changed. Writing them as character references keeps them (character
  references are not normalized). This comes from the spec text, not a measurement; the Phase 1
  tests prove it.

## 3. Design Overview

One serializer walk (`src/write.mfb`) with a formatting mode: `compact` or `indent(String)`. It
recurses over the package's own `Node` union, which is allowed (plan-138-A Verified properties).
Depth is bounded by `DEPTH_LIMIT` for parsed trees. A hand-built tree deeper than `DEPTH_LIMIT` is
refused with `ErrDepthExceeded` before writing, so the writer never recurses past it.

**Correctness risk:** pretty-printing inside mixed content. That is the one place a formatter can
silently change content. It is pinned by the content-equality tests and, in plan-138-C, by the
`fuzz-write` and `roundtrip` modes against both oracles.

**Gate class:** new behavior. The gates are tests and the doc-example script.

Rejected: emitting canonical XML (sorted attributes) by default. It changes the user's attribute
order for no content benefit.

## 4. Escaping and refusals

| Where | Written as |
|---|---|
| Text `&`, `<`, `>` | `&amp;` `&lt;` `&gt;` (`>` always, so `]]>` can never appear) |
| Text `#xD` | `&#13;` |
| Attribute value `&`, `<`, `"` | `&amp;` `&lt;` `&quot;` (values always double-quoted) |
| Attribute value `#x9` `#xA` `#xD` | `&#9;` `&#10;` `&#13;` |
| Empty element | `<name/>` |
| Document | `<?xml version="1.0" encoding="UTF-8"?>` then the children; bare `Node` gets no declaration |

Refused with `ErrInvalidArgument` (`77050002`), because the XML could not be read back with the same
content:

- an element or attribute name that is not a valid `Name` / has more than one `:`;
- duplicate attribute names;
- any string containing a non-`Char` scalar;
- a comment containing `--` or ending in `-`;
- a PI whose target matches `[Xx][Mm][Ll]` or whose data contains `?>`;
- a `Document` without exactly one `Element` child, or with a `Text` child.

The writer does not check namespace-prefix declarations: a tree the user built may use a prefix
declared nowhere. The reader would refuse the output. **Open Decision** below.

## 5. Pretty-printing

With `indent` non-empty, an element's children are written on their own lines, one indent deeper,
**only when the element has no data text child.** "Data text" means any text node that is not
layout whitespace (plan-138-A §4 step 3). In that case, the element's own whitespace-only text
children are layout, so they are dropped and replaced by the newline+indent.

An element that has a data text child is written exactly as compact output, children included, with
no whitespace added or removed inside it. That covers `<a>text</a>`, `<p>Hello <b>x</b></p>`, and an
element whose only child is whitespace (`<a>  </a>`). The content is identical; only the formatting
around it changed.

Comments and PIs in element-only content each get their own line. The declaration and each
top-level `Document` child get their own line. No trailing newline, matching `json::stringify`.
Clamping follows `json::stringify` exactly: count → `0..=10` spaces; string → first 10 characters;
`0`/`""` → byte-identical to compact.

## Compatibility / Format Impact

Adds the six `xml::stringify` overloads (three over `Document`, three over `Node`). No change to
anything plan-138-A shipped.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the work; `- [~]`
> partial; moot tasks struck through with evidence; fill `Commit:`. **Unticked means NOT DONE.**

### Phase 1 — compact writer

- [x] `packages/xml/src/write.mfb` — compact walk with §4 escaping and refusals.
- [x] `packages/xml/src/lib.mfb` — `EXPORT FUNC stringify(doc AS Document) AS String` and
      `EXPORT FUNC stringify(n AS Node) AS String`.
- [x] `packages/xml/src/content.mfb` — `PUBLIC FUNC contentKey(doc AS Document) AS String`, a
      canonical string of plan-138-A §4's projection, for tests only (package-internal).
- [x] Tests: `packages/xml/src/test_write.mfb` — each §4 escaping row, with the exact output string;
      each refusal with its code; for every `test_read.mfb` accepting fixture,
      `contentKey(parse(stringify(parse(x)))) = contentKey(parse(x))`.
- [x] Added task: a "writing twice is stable" case over every fixture
      (`stringify(parse(stringify(parse(x)))) = stringify(parse(x))`), and cases pinning the content
      projection itself — that comments/PIs are not content, that text split by a comment merges,
      that layout whitespace is dropped but sole whitespace is kept, and that attribute ORDER is not
      content while attribute VALUES are. Without these the round-trip assertions could pass
      vacuously on a projection that hides differences.

Acceptance: compact output is exact and content-preserving on every reader fixture.
  Check: `target/release/mfb test packages/xml` → all pass (est. 1 min).
  MET: `./target/release/mfb test packages/xml` → `Tests: 141  Pass: 141  Fail: 0`, exit 0 (27 added
  here, over the 114 letter A left).
Commit: `904071bea`

### Phase 2 — pretty writer

- [x] `write.mfb` — §5 formatting; `stringify(doc, count AS Integer)` and
      `stringify(doc, indent AS String)` (and Node forms).
- [x] Tests: in `test_write.mfb` — exact expected output for element-only nesting, mixed content
      left inline, `<a>  </a>` left inline, comments/PIs on their own lines; the clamp rules
      (`11` → 10 spaces, `"abcdefghijkl"` → first 10, `0`/`""` byte-equal to compact); content
      equality for every reader fixture at indent `2` and `"\t"`.
- [x] Added task: a case pinning an element whose children are ONLY comments or processing
      instructions as inline at any indent — the Corrections entry below is a content bug the fixture
      loop caught, and this pins it directly rather than leaving it to one fixture inside a loop.

Acceptance: pretty output matches the exact expectations and preserves content on every fixture.
  Check: `target/release/mfb test packages/xml` → all pass (est. 1 min).
  MET: `./target/release/mfb test packages/xml` → `Tests: 154  Pass: 154  Fail: 0`, exit 0 (13 added
  here). The run before the §5 correction was `Pass: 153  Fail: 1`, failing exactly on content.
  With the added inline-comments case the suite is `Tests: 155  Pass: 155  Fail: 0`.
Commit: `ea4d79040`

### Phase 3 — documentation

- [x] `packages/xml/src/lib.mfb` — full `DOC` blocks (`PACKAGE` intro like yaml's, with the
      compatibility policy as `INFO` paragraphs: no DTD, prefixes not resolved, UTF-8 / 1.0 only,
      limits, what content means), an `EXAMPLE` on `parse`, `root`, `attr`, `stringify`.
- [x] `packages/xml/README.md` — mirror `packages/yaml/README.md`: usage, "Why this is a package",
      what it reads, a policy table (decision / behaviour / code), content vs. formatting. Also
      states the `root(doc)`-widening rule from Corrections, so the first user does not hit it cold.
- [x] `packages/xml/check-doc-examples.sh` — copy `packages/mustache/check-doc-examples.sh`, adjust
      the package name.
- [x] `packages/xml/doc.html` — `target/release/mfb build packages/xml && target/release/mfb pkg doc packages/xml/xml.mfp --out packages/xml/doc.html`
      → `Wrote documentation to packages/xml/doc.html` (25,479 bytes). Generated and verified, but
      **not committed**: `doc.html` is git-ignored tree-wide and no package tracks one. See
      Corrections.
- [x] `planning/todo.md` — row 6 and the "XML as pure MFB" note point at plan-138.
- [x] `examples/browser/README.md` — correct the two stale claims plan-138-A measured (arena
      free-list "known open issue"; imported-union recursion), citing the plan-138-A measurements.
      The section also now states what IS still true and why `dom` keeps its work-stack: rebuilding a
      shared list of subtrees is quadratic (bug-652), which is a different claim from either stale one.

Acceptance: every documented example compiles and runs.
  Check: `packages/xml/check-doc-examples.sh` → exit 0 (est. 3 min).
  MET: `./packages/xml/check-doc-examples.sh` → `all 5 example(s) built and ran`, exit 0. Each
  example's printed output matches the expectation written beside it in its `DOC` block: `feed`;
  `catalog holds 1 child` then `refused: 77050003`; the three `stringify` forms; `catalog`; and
  `page` then `none`.
Commit: `809c3a979` (also carries the coverage follow-up: the empty-prefix writer bug, the two dead
  guards, and `Tests: 157  Pass: 157  Fail: 0`)

## Validation Plan

- Tests: `packages/xml/src/test_write.mfb` (escaping, refusals, pretty rules, content equality).
- Coverage check: `target/release/mfb test --coverage packages/xml`; every refusal branch in
  `write.mfb` is hit.
  DONE: `Tests: 157  Pass: 157  Fail: 0`; slot coverage (`coverage.covmap.json` paired with
  `coverage.covdata`) `write.mfb` **173/173**, `content.mfb` 46/47, `chars.mfb` 113/113,
  `lib.mfb` 34/34, `scan.mfb` 89/89, `core.mfb` 4/4, `read.mfb` 384/389. `content.mfb`'s one slot is
  the `CASE ELSE` a `MATCH` over the four-variant union needs for exhaustiveness, and `read.mfb`'s
  five are letter A's `RETURN`s after a `FAIL` — all unreachable by construction.

  **The coverage read found a writer bug, not just untested lines.** `requireWritableName` returned
  early whenever the prefix was empty, so `stringify(element(":a", …))` emitted `<:a/>` — which this
  package's own reader refuses (`parse("<:a/>")` → `ErrInvalidFormat`). That breaks the writer's
  contract of never emitting XML its own reader rejects. A colon is a `NameChar`, so `isName` accepts
  `:a`, `a:` and `a:b:c` alike; the QName rules belong in the writer's own check. Fixed, with cases
  for all four spellings over both element and attribute names.

  Two further branches were dead and were deleted rather than tested: `isLayoutText`'s and
  `isAllSpace`'s empty-string guards, whose callers already skip empty text. That fix also corrected
  a real classification: empty text is **not** data, so it no longer forces an element inline.
- Runtime proof: `check-doc-examples.sh` runs the `stringify` example end to end.
- Doc sync: `packages/xml/README.md`, `packages/xml/doc.html`, `planning/todo.md`,
  `examples/browser/README.md`.
- Final gate: runs once at the end of plan-138-E.

## Open Decisions

- Writer and undeclared prefixes — **write them unchecked** (the writer serializes what it is given;
  a user building namespaced XML declares `xmlns:p` themselves) vs. refuse with
  `ErrInvalidArgument`. Recommend unchecked. The oracle fuzz trees in C always declare their
  prefixes, so this does not affect agreement. (§4)

## Corrections

**Phase 1 — same-named overloads share ONE `DOC` block.** A `DOC` block per overload is rejected:
`error[2-205-0003 DOC_DUPLICATE]: two DOC blocks name the same declaration`. The block attaches to
the first `EXPORT FUNC` and carries the union of the `ARG` lines, which is what `packages/timezones`
already does for its two `toIso` overloads (`packages/timezones/src/lib.mfb:189-224`: one block,
`ARG dt` / `ARG digits` / `ARG name`). §8's table lists the six `stringify` forms as separate rows;
that is the API surface, not six doc blocks.

**Phase 3 — `doc.html` is generated, never committed; the plan's "6 tracked" was miscounted.** §2's
Measured populations row claimed six packages carry a tracked `doc.html`, and Phase 3 listed
`packages/xml/doc.html` as a deliverable. Both rest on `ls packages/*/doc.html | wc -l`, which counts
files on disk. The tracked count is **zero**: `git ls-files packages/*/doc.html` prints nothing, and
`.gitignore:83` ignores `doc.html` tree-wide — its own comment says so, "no doc.html is a tracked
file anywhere in the tree" (`git check-ignore -v packages/xml/doc.html` → `.gitignore:83:doc.html`).
Committing mine would need `git add -f` and would make `packages/xml` the only package in the tree
carrying one. The file is therefore generated and verified as the task asks, and left untracked like
every other package's. The command stays in the README and in this plan, which is what a reader
needs.

**Phase 2 — §5's indent condition was incomplete: it must also require an ELEMENT child.** §5 says an
element's children go on their own lines "only when the element has no data text child". That is not
sufficient. An element whose children are only comments or processing instructions has no data text
child, so the rule as written indents it — but it also has no *element* child, and plan-138-A §4
step 3 counts whitespace-only text as layout only when the parent **also has an element child**. The
newlines the writer added therefore read back as DATA. Caught by the Phase 2 content test on the
`<a><!-- note --></a>` fixture: `expected e(a[]), got e(a[t(` — the content projection saw the
writer's own indentation as the element's text. The condition is now "has an element child, and no
data text child"; `<a><!--c--><b/></a>` still indents, because it has one.

**Phase 1 — `root(doc)` returns an `Element` record, so `stringify(root(doc))` matches no overload.**
`stringify` is declared over `Document` and `Node`; `Element` is a variant record of `Node`, and the
call fails with `Callable \`stringify\` is not a top-level function`. A caller widens through a typed
binding first (`LET n AS xml::Node = root(doc)`), which is the same pattern the constructors use
internally. This is not the bug-631 imported-overload ambiguity — it reproduces inside the package,
where no import is involved. Worth stating in the README (Phase 3) so the first user does not hit it
cold.

## Summary

The risk is pretty-printing mixed content without changing content; the rule "indent only where
there is no data text" keeps it structural, and C re-checks it against both oracles. Reader and
tree are untouched.
