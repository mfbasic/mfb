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
| bug-631 fixed: an imported overloaded call resolves when its argument is a field of an imported record | `ls bugs/completed/bug-631-*` → one file; and a consumer of `packages/xml` with `FOR EACH n IN doc.children` / `io::print(xml::stringify(n))` → `mfb build` prints `Wrote executable` | PARTIALLY MET (2026-09-15): bug-631 fixed on `worktree-B-631` (`73ab94edc`); every ✗ reproduction row now builds and prints its expected output. The `packages/xml` consumer check waits on plan-138-A; re-run it then. |

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
| packages with a tracked `doc.html` | 6 (json_schema, jwt, logger, mustache, timezones, yaml) | `ls packages/*/doc.html \| wc -l` → 6 |

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
Commit: —

### Phase 2 — pretty writer

- [ ] `write.mfb` — §5 formatting; `stringify(doc, count AS Integer)` and
      `stringify(doc, indent AS String)` (and Node forms).
- [ ] Tests: in `test_write.mfb` — exact expected output for element-only nesting, mixed content
      left inline, `<a>  </a>` left inline, comments/PIs on their own lines; the clamp rules
      (`11` → 10 spaces, `"abcdefghijkl"` → first 10, `0`/`""` byte-equal to compact); content
      equality for every reader fixture at indent `2` and `"\t"`.

Acceptance: pretty output matches the exact expectations and preserves content on every fixture.
  Check: `target/release/mfb test packages/xml` → all pass (est. 1 min).
Commit: —

### Phase 3 — documentation

- [ ] `packages/xml/src/lib.mfb` — full `DOC` blocks (`PACKAGE` intro like yaml's, with the
      compatibility policy as `INFO` paragraphs: no DTD, prefixes not resolved, UTF-8 / 1.0 only,
      limits, what content means), an `EXAMPLE` on `parse`, `root`, `attr`, `stringify`.
- [ ] `packages/xml/README.md` — mirror `packages/yaml/README.md`: usage, "Why this is a package",
      what it reads, a policy table (decision / behaviour / code), content vs. formatting.
- [ ] `packages/xml/check-doc-examples.sh` — copy `packages/mustache/check-doc-examples.sh`, adjust
      the package name.
- [ ] `packages/xml/doc.html` — `target/release/mfb build packages/xml && target/release/mfb pkg doc packages/xml/xml.mfp --out packages/xml/doc.html`.
- [ ] `planning/todo.md` — row 6 and the "XML as pure MFB" note point at plan-138.
- [ ] `examples/browser/README.md` — correct the two stale claims plan-138-A measured (arena
      free-list "known open issue"; imported-union recursion), citing the plan-138-A measurements.

Acceptance: every documented example compiles and runs.
  Check: `packages/xml/check-doc-examples.sh` → exit 0 (est. 3 min).
Commit: —

## Validation Plan

- Tests: `packages/xml/src/test_write.mfb` (escaping, refusals, pretty rules, content equality).
- Coverage check: `target/release/mfb test --coverage packages/xml`; every refusal branch in
  `write.mfb` is hit.
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
