# plan-138-A: `xml` package — tree model, builder spike, and the strict reader

Last updated: 2026-09-15
Overall Effort: huge (>3d) — the whole plan-138 feature: a pure-MFBASIC `packages/xml` that reads
XML 1.0 (5th edition, UTF-8, no DTD) into a `UNION`-of-records tree, writes it back compact or
pretty-printed, answers an XPath 1.0 subset, and is checked three ways against a Node oracle and a
Rust oracle as equal peers
Effort: large (3h–1d)
Depends on: nothing inside plan-138 (see Prerequisites for the whole-feature gate)

plan-138 adds `packages/xml`. The behavioral outcome of the whole feature: **for any input, the
package, the Node oracle and the Rust oracle either all refuse it or all produce the same content;
anything the package writes, both oracles read back as the same content; and every XPath expression
in the subset returns the same result on all three.** "Content" is defined once, in §4 of this
document, and every letter uses that definition.

This letter (A) builds the reader: the node types, a tree builder proven fast enough for 100k
nodes, and a strict well-formedness parser with namespace-prefix checks and limits.

| Letter | Delivers | Effort |
|---|---|---|
| **A** (this) | builder spike, tree model, strict reader, limits | large |
| B | `xml::stringify` compact + pretty, DOC/README/doc.html | large |
| C | Node + Rust oracles, probe, corpus, W3C suite, fuzz/round-trip/mutate/perf | large |
| D | XPath 1.0 subset in the package | large |
| E | XPath on both oracles, final validation, archive | medium |

References:

- `planning/todo.md` — row 6 "XML | Pure MFB package" and the note "XML as pure MFB, not a libxml2
  binding … Scope to XML only".
- `packages/yaml/` — the precedent for a pure-MFB format reader with a differential oracle
  (`packages/yaml/oracle/README.md`).
- `examples/browser/dom/src/lib.mfb` (`Node` union, `lib.mfb:88`) and
  `examples/browser/dom/src/parse.mfb` (`parse`, frame stack `Frame`) — the tree shape the user
  chose to mirror, and the builder pattern this plan must NOT copy (see Verified properties).
- W3C Extensible Markup Language (XML) 1.0 (Fifth Edition) and Namespaces in XML 1.0 (Third
  Edition) — the grammar and well-formedness constraints.
- `mfb spec memory arenas` (`src/docs/spec/memory/04_arenas.md`) and `.ai/collections.md` — the
  copy semantics that decide the builder.
- `src/docs/spec/diagnostics/02_error-codes.md` — the shared error codes reused here.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| `packages/xml` does not exist yet | `ls packages/xml` → `No such file or directory` | MET (re-measured at the gate 2026-09-15 in `worktree-P-138`: `ls packages/xml` → `No such file or directory`. Phase 2 creates it, so this row is only ever true before the gate.) |
| Release compiler is current with HEAD | `cargo build --release` → `Finished` | MET (re-measured 2026-09-15: built from scratch in the `worktree-P-138` worktree → `Finished \`release\` profile [optimized] target(s) in 1m 33s`, exit 0; `target/release/mfb` written 12:30) |
| Node ≥ 18 for the oracle | `node --version` → `v24.12.0` | MET (re-measured 2026-09-15 → `v24.12.0`) |
| crates.io reachable for the Rust oracle | `cargo search roxmltree --limit 1` → `roxmltree = "0.21.1"` | MET (re-measured 2026-09-15 → `roxmltree = "0.21.1"    # Represent an XML as a read-only tree.`) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify.
>
> **If you stop, report the current status of *all* prerequisites.**

## 1. Goal

- `mfb test packages/xml` passes, and a program importing `xml` can `xml::parse` a UTF-8 XML 1.0
  document into an `xml::Document`, getting `Error` for every well-formedness violation, every
  `<!DOCTYPE`, every non-UTF-8 encoding declaration, every `version` other than `1.0`, and every
  namespace-prefix violation; and parsing a 100k-node document takes ≤ 3 s on the development Mac.

### Non-goals (explicit constraints)

- **No DTD support of any kind.** Any `<!DOCTYPE` — with or without an internal subset, with or
  without an external ID — is refused with `ErrUnsupported`. No entity declarations, no default
  attributes, no validation. (User decision.)
- **No namespace resolution.** Names are stored as the raw `prefix:local` string. Prefixes are
  *checked* (declared in scope, reserved prefixes respected) but never expanded to URIs. (User
  decision; checking agreed so the oracles agree.)
- **UTF-8 only, XML 1.0 only.** No UTF-16, no Latin-1, no XML 1.1. A leading UTF-8 BOM is accepted.
- **No HTML.** No tag-soup recovery, no void elements, no case folding — `todo.md` scopes HTML out.
- **No schema validation** (DTD or XSD). "Validate" in this feature means differential testing.
- **No changes to `src/`** (the compiler). If a compiler bug blocks this plan, it is fixed under
  its own bug document per AGENTS.md ("Never leave a bug you found"), not absorbed as scope here.
- **No changes to `examples/browser/dom`.** It stays a forgiving HTML parser.

## 2. Current State

There is no XML reader anywhere in the tree: `todo.md` row 6 records that
`grep -ril xml src/codegen/builtins/ packages/` hits only HTML escaping and MIME tables.

Precedents this plan mirrors:

- **Package layout** — `packages/yaml/`: `project.json` (kind `package`, sources `src/**/*.mfb`),
  `src/lib.mfb` holding the `EXPORT FUNC` API with `DOC` blocks (`lib.mfb:163 parse`,
  `lib.mfb:216 parseAll`), `src/core.mfb` holding cross-file `PUBLIC` helpers and constants,
  `src/test_*.mfb` holding `TESTING` blocks run by `mfb test <path>`.
- **Byte scanning** — yaml converts the input once with `strings::toBytes` (`lib.mfb:164`,
  `lib.mfb:217`) and slices text back out with `core.mfb:textOf(bytes, from, stop)`; byte
  constants are `PUBLIC LET B_* AS Integer` (`core.mfb:68-76`). `strings::` itself indexes by
  Unicode scalar, not byte (`mfb man strings mid` → "Extract a substring by Unicode scalar index").
- **Error codes** — every package reuses the shared `7705xxxx` codes for the common cases
  (`grep -rn "PUBLIC LET ERR_" packages/*/src` → yaml, jwt, mustache, json_schema all do) and
  allocates a package-specific `93NNxxxx` code only for a failure no shared code names
  (`yaml::ErrorExpansionLimit = 93110001`, `packages/yaml/src/core.mfb:51`).
- **Limits** — yaml: `DEPTH_LIMIT = 100`, `NODE_LIMIT = 1000000`, `ALIAS_LIMIT = 10000`
  (`core.mfb:31-38`); `json::parse` caps nesting at 256 with `ErrDepthExceeded`
  (`src/docs/spec/stdlib/04_json.md`, "256 levels"). yaml's `src/test_limits.mfb` asserts both that
  a limit fires and that a document just inside it parses.
- **Tree shape** — `dom`: `EXPORT UNION Node` of `ElementNode`/`TextNode`/`HeaderNode` records
  (`examples/browser/dom/src/lib.mfb:88`), constructors widening a record to `Node` via a typed
  `LET` (`lib.mfb:103 element`).

### Measured populations

| What | Value | Command |
|---|---|---|
| yaml package source | 2,768 lines | `wc -l packages/yaml/src/*.mfb` → 2768 total |
| yaml oracle (mjs + json + README) | 686 lines | `wc -l packages/yaml/oracle/*.mjs packages/yaml/oracle/*.json packages/yaml/oracle/README.md` → 686 total |
| packages that already carry an `oracle/` | 5 (json_schema, jwt, mustache, timezones, yaml) | `ls packages/` + `find packages/<p>/oracle` |
| `dom`-style frame-stack builder, 10k-node tree | 15.82 s – 27.32 s; 50k exceeded a 120 s cap | scratch probe `/tmp/xmlarena/xt` (package building an `EXPORT UNION` tree with `dom`'s `pushKid`/`closeTag` pattern), `perl -e 'alarm 120; exec @ARGV' xtapp.out <n> build\|own\|imported` |
| flat record churn (no nested subtrees) | 1M kept in a list: 2.37 s; 1M transient: 0.35 s; 100k kept: 0.16 s | scratch probe `/tmp/xmlarena/churn` |
| `json::parse` (native built-in), 100k values, 1,177,815 bytes | 0.74 s – 1.35 s (3 runs) | scratch probe `/tmp/xmlarena/jsonbase` |
| `yaml::parse` (pure MFB) on the same document | 1.20 s – 1.64 s (3 runs) | scratch probe `/tmp/xmlarena/yamlbase` (yaml source copied to `/tmp`) |
| package-specific error ranges in use | `9311` yaml; `9312` json_schema + mustache; `9313` jwt → `9314` free | `grep -rn -E "EXPORT LET [A-Za-z]+ AS Integer = 931[0-9]{5}" packages examples` |

### Verified properties

- **The arena free-list slowdown is fixed.** The browser README still calls it "a known open arena
  issue" (`examples/browser/README.md`, section "DOM"), but `04_arenas.md` documents the segregated
  quick/large bins that replaced the quadratic insert, and the flat churn probe above allocates and
  frees 1M records in 0.35 s. The README line is stale.
- **`collections::get` returns an owned deep copy.** `.ai/collections.md`: "`get` has returned an
  owned deep copy since bug-538"; `04_arenas.md`: "`collections::get`/`getOr` materialize the
  element". In the frame-stack builder every `pushKid` does `collections::get(stack, top)` on a frame
  whose `kids` holds every subtree built so far, so each step copies the whole partial tree. That is
  consistent with the 10k/50k numbers. **UNVERIFIED as the sole cause** — Phase 1 measures it.
- **A consumer can recurse over an imported union.** The browser README says a recursive function
  over an imported union "does not lower to native code across a package boundary"; the `xtapp`
  probe's `countImported` (a recursive `FUNC` over `xt::Node` in the consumer) returned `10002` for
  a 10k build. The README line is stale; consumers may write their own tree walks.
- **`collections::take(list, n)` keeps the first `n`; `collections::drop(list, n)` discards the first
  `n`** (`mfb man collections take`, `mfb man collections drop`).
- **The planned API compiles as a package in `packages/**`, not a built-in.** A scratch package
  named `xml` (`/tmp/xmlarena/pkgcheck/xml`) builds with: `EXPORT LET ErrorNodeLimit`; every §5
  record, the `Node` union and `Document`; plan-138-D's `XPathValue` union; a recursive
  `PUBLIC FUNC` in a second source file; and four `EXPORT FUNC stringify` overloads `(Document)`,
  `(Node)`, `(Document, Integer)`, `(Document, String)`. A consumer importing it matches
  `CASE Element/Text/Comment/ProcessingInstruction` and `CASE XNodes` and prints the expected values
  (`mfb build xml && cp xml/xml.mfp app/packages/ && mfb build app && ./app/build/xmlcheck.out` →
  `doc:5`, `doc-count:2`, `doc-indent:[<tab>]`, `node:1`, `comment note`, `node:1`, `pi target`,
  `node:3`, `element a attrs=1`, `xnodes 3`, `93140001`).
- **Compiler bug `bugs/bug-631-imported-overload-ambiguous-on-imported-record-field-argument.md`.**
  In that consumer, `FOR EACH n IN doc.children` then `xml::stringify(n)` fails with
  `error[2-203-0101 TYPE_OVERLOAD_AMBIGUOUS]: … Call to xml.stringify matches 2 imported overloads`.
  Binding `LET node AS xml::Node = n` first compiles and runs. Narrowed in bug-631: the trigger is
  any argument read from a **field of an imported record** (direct, untyped `LET`, `FOR EACH`, or
  `collections::get`), not the loop. The monomorphizer's `concrete_types` holds only the consumer's
  own types, so the field types as `Unknown`, which matches every overload (bug-36's wildcard). It
  is a prerequisite of plan-138-B, which ships the overloads.
- **Lowercase constructors beside capitalized record names work across the package boundary.**
  Adding `EXPORT FUNC element/text/comment/pi/document/root/attr/attrOr/textOf` (a second source file
  in the scratch package) and calling them from the consumer prints `root r textOf=[hi there]`,
  `attrOr none`, `attr trapped` (a `FAIL error(77050004, …)` caught with `TRAP`).
- **Package tooling works on a package.** On a `/tmp` copy of `packages/yaml`:
  `mfb test --coverage` → `Tests: 89  Pass: 89  Fail: 0` and wrote `coverage.html`;
  `mfb pkg doc yaml.mfp --out /tmp/xmlarena/yaml-doc.html` → `Wrote documentation`.

## 3. Design Overview

Three layers inside `packages/xml`, each landable on its own:

1. **Tree model** (`src/tree.mfb`) — records, the `Node` union, `Document`, constructors and
   accessors. No parsing.
2. **Scanner** (`src/scan.mfb`, `src/chars.mfb`) — bytes in, UTF-8 decoded scalars with line/column,
   XML `Char`/`NameStartChar`/`NameChar` classes, line-end normalization.
3. **Reader** (`src/read.mfb`) — the well-formedness grammar, namespace-prefix checks, limits, and
   the tree builder chosen in Phase 1.

**Design uncertainty concentrates in the builder** (can an immutable `UNION` tree of 100k nodes be
built in ≤ 3 s given copy-on-`get`?). It is Phase 1, a throwaway spike, before any package code.

**Correctness risk concentrates in the reader's grammar**: every "accepts invalid XML and silently
changes the meaning" class that `packages/yaml/oracle/README.md` "What it found" catalogues for
YAML. It is guarded by TESTING blocks here and by the three-way oracle in C.

**Gate class:** behavior-changing new code. Byte-identity does not apply; the gates are
`mfb test packages/xml` and measured timings.

Rejected alternatives:

- **libxml2 binding** — `todo.md`: large wrapper surface, CVE history, not a system library on
  Windows (would need `libsnd`-style vendoring).
- **Reusing `dom::parse`** — it never fails, lowercases names, knows HTML void tags and drops
  comments; XML is the opposite contract.
- **`dom`'s frame-stack builder** — measured at 15.82 s for 10k nodes (table above).
- **`json::Json` as the model (yaml's choice)** — cannot hold attributes, element order with mixed
  content, or comments.

## 4. Content vs. formatting (the definition every letter uses)

The user's rule: *content* must survive every read, write and pretty-print and must match across
the three implementations; *formatting* may change freely.

**Content** is the tree after this projection:

1. Comments and processing instructions are removed (they are kept in the tree, but they are not
   content).
2. Adjacent text (including text that was CDATA, and text that becomes adjacent once step 1 removes
   a comment or PI between it) is merged into one text node.
3. A text node consisting only of XML whitespace (`#x20 #x9 #xD #xA`) **whose parent also has at
   least one element child** is *layout whitespace* — formatting — and is removed. Any other text,
   including a whitespace-only text node that is its parent's only content (`<a>   </a>`), is data
   and is kept byte-for-byte.
4. Attributes compare as a set of (name, value) pairs (XML gives attribute order no meaning).
5. Names compare as their raw `prefix:local` strings.
6. Values compare after the XML-mandated normalizations the reader already applies: line ends (§2.11)
   and attribute-value normalization for CDATA-typed attributes (§3.3.3; with no DTD every
   attribute is CDATA-typed).

The parser **keeps** whitespace-only text nodes in the tree; only the content projection (and B's
pretty-printer, which replaces layout whitespace with its own indentation) treats them as layout.

## 5. Tree model

```
EXPORT TYPE Attribute
  name AS String          ' raw "prefix:local" or "local"
  value AS String         ' after §3.3.3 normalization and reference expansion
END TYPE

EXPORT TYPE Element
  name AS String
  attributes AS List OF Attribute   ' document order, no duplicates
  children AS List OF Node
END TYPE

EXPORT TYPE Text
  text AS String          ' CDATA sections arrive merged into Text
END TYPE

EXPORT TYPE Comment
  text AS String
END TYPE

EXPORT TYPE ProcessingInstruction
  target AS String
  data AS String
END TYPE

EXPORT UNION Node
  Element
  Text
  Comment
  ProcessingInstruction
END UNION

EXPORT TYPE Document
  children AS List OF Node   ' prolog/epilog Comments and PIs, and exactly one Element
END TYPE
```

- `xmlns` / `xmlns:p` declarations stay in `attributes` like any other attribute.
- The XML declaration is not stored: the only accepted values are `version="1.0"`,
  `encoding` = `UTF-8` (case-insensitive) or absent, and `standalone` = `yes`/`no`/absent.
- Whitespace outside the root element (production `Misc`/`S`) is not stored.
- Attributes are a `List`, not a `Map` like `dom`'s, so document order survives and duplicates are
  detected during parsing.

Accessors (`src/lib.mfb`, `EXPORT FUNC`): `xml::parse(text AS String) AS Document`,
`xml::root(doc AS Document) AS Element`, `xml::attr(e AS Element, name AS String) AS String`
(fails `ErrNotFound`), `xml::attrOr(e AS Element, name AS String, fallback AS String) AS String`,
`xml::textOf(n AS Node) AS String` (XPath string-value: concatenated descendant text), and the
constructors `xml::element(name, attributes, children) AS Node`, `xml::text(s) AS Node`,
`xml::comment(s) AS Node`, `xml::pi(target, data) AS Node`, `xml::document(children) AS Document`.

## 6. Scanner

- Input is converted once with `strings::toBytes` (yaml precedent). A leading `EF BB BF` is skipped.
- UTF-8 is decoded by hand to scalars for validation. Overlong forms, surrogates (`D800–DFFF`),
  values above `10FFFF` and truncated sequences → `ErrInvalidFormat`.
- `Char` (§2.2: `#x9 | #xA | #xD | [#x20-#xD7FF] | [#xE000-#xFFFD] | [#x10000-#x10FFFF]`) is
  checked on every scalar of the document, including inside comments, PIs, CDATA and attribute
  values.
- `NameStartChar` / `NameChar` follow §2.3 of the 5th edition exactly (the ranges, not a
  letter/digit approximation — `dom`'s `isNameChar` ASCII table is the counter-example).
- Line ends: `#xD #xA` and a lone `#xD` both become `#xA` before any other processing (§2.11). The
  yaml oracle README records a real bug from getting lone CR wrong.
- Position: every error message is `xml: line L, column C: <what>`, with line/column in scalars,
  1-based. The oracles compare refusals without comparing messages (C).
- Text is sliced back out with a `textOf(bytes, from, stop)` equivalent over byte offsets, so a run of
  character data costs one allocation, not one per scalar.

## 7. Reader

Productions implemented, each with its well-formedness constraint:

| Construct | Accepted | Refused (code) |
|---|---|---|
| XML declaration | only at byte 0 (after BOM); `version="1.0"`; `encoding` UTF-8/absent; `standalone` yes/no | other version → `ErrUnsupported`; other encoding → `ErrUnsupported`; malformed → `ErrInvalidFormat` |
| `<!DOCTYPE` | never | `ErrUnsupported` |
| Elements | start/end tags with matching names, `<a/>` | mismatched end tag, unclosed element, text/second element after the root, no root → `ErrInvalidFormat` |
| Attributes | `name="…"` or `name='…'`, whitespace-separated | duplicate name → `ErrAlreadyExists`; `<` in value, missing quotes/`=`/separating whitespace → `ErrInvalidFormat` |
| Attribute value | references expanded; `#x20 #xD #xA #x9` literal chars → `#x20` (§3.3.3); a `&#10;`-style char ref is kept as its character | — |
| References | `&lt; &gt; &amp; &apos; &quot;`, `&#N;`, `&#xH;` | any other entity name (undeclared entity, a WF error without a DTD) or a char ref to a non-`Char` → `ErrInvalidFormat` |
| Character data | everything else | literal `]]>` in text → `ErrInvalidFormat` |
| CDATA | `<![CDATA[ … ]]>` merged into the adjacent Text | unterminated → `ErrInvalidFormat` |
| Comments | `<!-- … -->` | `--` inside, or a comment ending `--->` → `ErrInvalidFormat` |
| PIs | `<?target data?>` | target matching `[Xx][Mm][Ll]` other than the declaration → `ErrInvalidFormat` |
| Namespace prefixes | `xmlns:p="uri"` declarations scoped to the element; `xml` prefix is pre-bound | an element or attribute prefix not declared in scope; `xmlns:p=""`; binding or declaring `xmlns`; binding `xml` to another URI or another prefix to the XML namespace URI; a name with more than one `:` or an empty prefix/local part; two attributes whose prefixes are bound to the same URI and whose local parts match → all `ErrInvalidFormat` |

Limits (`src/core.mfb`, `PUBLIC LET`):

- `DEPTH_LIMIT = 256` element nesting levels → `ErrDepthExceeded` (`77050024`), matching
  `json::parse`'s 256. Deeper structure is well-formed but exceeds what this reader descends.
- `NODE_LIMIT = 1000000` nodes (elements + text + comments + PIs) → `xml::ErrorNodeLimit =
  93140001` (`EXPORT LET`), matching yaml's node limit value and its package-specific-code pattern.

Error constants in `src/core.mfb` (yaml naming): `ERR_FORMAT = 77050003`, `ERR_UNSUPPORTED =
77050007`, `ERR_NOT_FOUND = 77050004`, `ERR_DUPLICATE = 77050005`, `ERR_DEPTH = 77050024`,
`ERR_ARGUMENT = 77050002` (used by B and D).

## 8. Final public API (all letters)

Everything a consumer of `IMPORT xml` sees when plan-138 is complete. Every name here compiled in
the scratch package check (Verified properties) except the XPath functions and `XNumber`, which
follow the same `EXPORT FUNC` / `EXPORT TYPE` forms.

**Types** (A §5, D §5)

| Type | Fields |
|---|---|
| `xml::Document` | `children AS List OF Node` |
| `xml::Node` (UNION) | `Element` \| `Text` \| `Comment` \| `ProcessingInstruction` |
| `xml::Element` | `name AS String`, `attributes AS List OF Attribute`, `children AS List OF Node` |
| `xml::Attribute` | `name AS String`, `value AS String` |
| `xml::Text` | `text AS String` |
| `xml::Comment` | `text AS String` |
| `xml::ProcessingInstruction` | `target AS String`, `data AS String` |
| `xml::XPathValue` (UNION) | `XNodes` \| `XAttributes` \| `XString` \| `XNumber` \| `XBoolean` |
| `xml::XNodes` / `XAttributes` | `nodes AS List OF Node` / `attributes AS List OF Attribute` |
| `xml::XString` / `XNumber` / `XBoolean` | `value AS String` / `Float` / `Boolean` |

**Functions**

| Function | Letter | Fails with |
|---|---|---|
| `parse(text AS String) AS Document` | A | `ErrInvalidFormat`, `ErrUnsupported`, `ErrAlreadyExists`, `ErrDepthExceeded`, `xml::ErrorNodeLimit` |
| `stringify(doc AS Document) AS String` | B | `ErrInvalidArgument`, `ErrDepthExceeded` |
| `stringify(doc AS Document, count AS Integer) AS String` | B | same |
| `stringify(doc AS Document, indent AS String) AS String` | B | same |
| `stringify(n AS Node) AS String` (+ `count` / `indent` forms) | B | same |
| `root(doc AS Document) AS Element` | A | `ErrNotFound` |
| `attr(e AS Element, name AS String) AS String` | A | `ErrNotFound` |
| `attrOr(e AS Element, name AS String, fallback AS String) AS String` | A | — |
| `textOf(n AS Node) AS String` | A | — |
| `document(children AS List OF Node) AS Document` | A | — |
| `element(name AS String, attributes AS List OF Attribute, children AS List OF Node) AS Node` | A | — |
| `text(s AS String) AS Node` | A | — |
| `comment(s AS String) AS Node` | A | — |
| `pi(target AS String, data AS String) AS Node` | A | — |
| `evaluate(doc AS Document, expr AS String) AS XPathValue` | D | `ErrInvalidFormat` (syntax), `ErrUnsupported` (outside subset, mixed node-set) |
| `select(doc AS Document, expr AS String) AS List OF Node` | D | as `evaluate`, plus `ErrInvalidArgument` if not an element/text/comment/PI node-set |
| `selectAttributes(doc AS Document, expr AS String) AS List OF Attribute` | D | as `evaluate`, plus `ErrInvalidArgument` if not an attribute node-set |
| `valueOf(doc AS Document, expr AS String) AS String` | D | as `evaluate` |

**Constants:** `xml::ErrorNodeLimit = 93140001`. The other codes are the shared
`errorCode::` values listed in §7.

## Compatibility / Format Impact

New package only. No compiler, spec, or existing-package change. The public surface introduced here
(`xml::Document`, `xml::Node` and its records, `xml::parse`, accessors, constructors,
`xml::ErrorNodeLimit`) is the contract letters B–E build on.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as the work;
> `- [~]` for partial with what remains; mark moot tasks `- [x] ~~text~~ — moot: <evidence>`; fill
> `Commit:` when a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — builder spike (throwaway, in `/tmp`)

Decides the builder before any package code exists. Nothing lands in the repo except this plan's
update.

- [x] Write a scratch package + consumer under `/tmp/xml-builder-spike/` (AGENTS.md: one-off probes
      live in `/tmp`) holding the §5 records and a **pending-children builder**: one
      `MUT pending AS List OF Node` for completed nodes, and one flat `MUT open AS List OF OpenTag`
      (`name`, `attributes`, `start AS Integer` = `len(pending)` when the tag opened; no subtrees).
      On a close tag: `kids = collections::drop(pending, start)`,
      `pending = collections::take(pending, start)`, then
      `pending = collections::append(pending, <Element with kids>)`. The only list with subtrees is
      never read back with `get`.
- [x] Measure three shapes at 100k nodes, 3 runs each, recording min–max:
      **all three exceeded the 30 s alarm** (`bash /tmp/xml-builder-spike/time.sh …/app/build/spike.out
      100000` → `flat|deep|wide run1..3 exit=142 30.01s`). See Corrections.
      (a) flat — `<root>` + 50,000 `<item id="N">value N</item>`;
      (b) deep — repeated chains nested 256 levels, to 100k nodes total;
      (c) wide — 100,000 empty `<i/>` directly under the root.
- [x] If any shape exceeds 3 s: build with `mfb build --debug` and read the arena report
      (`04_arenas.md` "Measuring an Arena") to find which operation copies. If the cause is a
      compiler defect (e.g. `drop`/`take` copying more than the elements they return), file it
      with the write-bug skill and fix it per AGENTS.md before Phase 2. Record every number and the
      chosen design in this plan's Corrections section.

Acceptance: a builder design is recorded here with all three shapes measured ≤ 3 s.
  Check: the spike consumer run as `perl -e 'alarm 30; exec @ARGV' ./spike.out <shape> 100000` →
  prints the node count, 3 runs per shape, each ≤ 3.00 s (est. 2 min to run).
  **MET by the recursive-descent builder** (Corrections "Phase 1"), 100k nodes, 3 runs each:
  flat 0.12–0.13 s (`nodes=99999 depth=2`), deep 0.12 s (`nodes=100000 depth=256`),
  wide 0.11 s (`nodes=100000 depth=2`) — every run ≤ 0.13 s against a 3.00 s budget.
  NOT met by the planned pending-children builder: all three shapes hit the 30 s alarm.
Commit: — (plan update only)

### Phase 2 — package skeleton and tree model

- [x] `packages/xml/project.json` — kind `package`, `name` `xml`, version `0.1.0`, sources
      `src/**/*.mfb` role `package` (mirror `packages/yaml/project.json`), with a `description` that
      states the §1 non-goals.
- [x] `packages/xml/src/tree.mfb` — the §5 records, union and `Document`.
- [x] `packages/xml/src/core.mfb` — error constants, `ErrorNodeLimit`, `DEPTH_LIMIT`, `NODE_LIMIT`,
      byte constants.
- [x] `packages/xml/src/lib.mfb` — constructors and accessors from §5 (`parse` is added in Phase 4),
      each with a minimal `DOC` block (full prose lands in B).
- [x] Tests: `packages/xml/src/test_tree.mfb` — constructors round-trip through accessors; `attr`
      fails `ErrNotFound`; `attrOr` falls back; `textOf` concatenates nested text in document order.

Acceptance: the tree API works from TESTING blocks.
  Check: `target/release/mfb test packages/xml` → all cases pass, exit 0 (est. 1 min).
  MET: `./target/release/mfb test packages/xml` → `Tests: 11  Pass: 11  Fail: 0`, exit 0.
Commit: —

### Phase 3 — scanner and character classes

- [ ] `packages/xml/src/chars.mfb` — UTF-8 decode with the §6 refusals; `isChar`, `isNameStartChar`,
      `isNameChar`, `isSpace` over scalars using the 5th-edition ranges.
- [ ] `packages/xml/src/scan.mfb` — cursor over the byte list with line/column, BOM skip, line-end
      normalization, `textOf`-style slicing, and the `xml: line L, column C:` error formatter.
- [ ] Tests: `packages/xml/src/test_chars.mfb` — each `NameStartChar` range boundary in and one past
      out; `#xFFFE`/`#xFFFF`/`#x0`/lone surrogate bytes refused; overlong `C0 80` refused; CRLF and
      lone CR become LF; line/column correct after a multi-byte scalar.

Acceptance: character and scanner rules hold at every range boundary.
  Check: `target/release/mfb test packages/xml` → all cases pass (est. 1 min).
Commit: —

### Phase 4 — reader

- [ ] `packages/xml/src/read.mfb` — every row of the §7 table, building the tree with the Phase 1
      design; namespace scope as a stack of (prefix → URI) frames pushed per element.
- [ ] `packages/xml/src/lib.mfb` — `EXPORT FUNC parse(text AS String) AS Document`.
- [ ] Tests: `packages/xml/src/test_read.mfb` — one accepting case per §7 row (with the tree it must
      produce, whitespace-only text nodes present); `packages/xml/src/test_refuse.mfb` — one refusal
      per §7 refusal cell, asserting the error code.

Acceptance: every §7 row has a passing accept case and every refusal cell a passing refusal case.
  Check: `target/release/mfb test packages/xml` → all pass; `grep -c "CASE" packages/xml/src/test_refuse.mfb`
  ≥ the number of refusal cells in §7 (est. 1 min).
Commit: —

### Phase 5 — limits and the 100k gate

- [ ] `read.mfb` — enforce `DEPTH_LIMIT` and `NODE_LIMIT`.
- [ ] Tests: `packages/xml/src/test_limits.mfb` — depth 256 parses, 257 fails `ErrDepthExceeded`; a
      document just inside `NODE_LIMIT` is not built in-test (too slow for a unit test) — instead a
      `PUBLIC` limit parameter on an internal `parseWithLimits` is exercised at a small limit (limit
      fires at N+1, passes at N), mirroring yaml's "a guard that rejects everything protects nothing".
- [ ] Measure `xml::parse` on the three Phase 1 shapes through a `/tmp` consumer of the built
      `packages/xml/xml.mfp`; record numbers here.

Acceptance: limits fire exactly at the boundary, and the real reader meets the budget.
  Check: `target/release/mfb test packages/xml` → pass; `/tmp` consumer on each shape at 100k → each
  of 3 runs ≤ 3.00 s (est. 3 min).
Commit: —

## Validation Plan

- Tests: `packages/xml/src/test_tree.mfb`, `test_chars.mfb`, `test_read.mfb`, `test_refuse.mfb`,
  `test_limits.mfb` — accept and refuse cases for every grammar row.
- Coverage check: `target/release/mfb test --coverage packages/xml` → open `coverage.html`; every
  refusal branch in `read.mfb` is hit.
- Runtime proof: the Phase 5 `/tmp` consumer parsing the 100k shapes.
- Doc sync: none in A (docs land in B).
- Final gate: runs once at the end of plan-138-E.

## Open Decisions

- `DEPTH_LIMIT` value — **256** (matches `json::parse`) vs. yaml's 100. Recommend 256: real XML
  (SVG, OOXML) nests deeper than typical YAML config. (§7)

## Corrections

**Phase 1 — the planned pending-children builder is rejected; the reader builds the tree by
recursive descent.** The plan predicted the pending-children design would avoid the copy-on-`get`
cost that killed `dom`'s frame stack. It does not: it replaces `get` with `collections::take`, which
is just as quadratic over a `List OF Node`. Measured in `/tmp/xml-builder-spike` (spike package +
consumer, timed by `/tmp/xml-builder-spike/time.sh`, 3 runs each):

| Builder | Shape | n | Time |
|---|---|---|---|
| pending-children (planned) | flat, deep, wide | 100,000 | **all killed at the 30 s alarm** (`exit=142 30.01s`) |
| pending-children | wide | 2,000 | 1.37–1.38 s |
| pending-children | wide | 8,000 | 22.21–22.31 s (4× the nodes → 16× the time: quadratic) |
| recursive descent (chosen) | flat | 100,000 | 0.12–0.13 s |
| recursive descent | deep (256 levels) | 100,000 | 0.12 s |
| recursive descent | wide | 100,000 | 0.11 s |

Localized by isolation shapes at 8,000 nodes rather than by the arena report, which the phase
offered as one way to find the copying operation — the isolation split names it outright:

| Isolation shape (per step, over a `List OF Node` unless noted) | Time at n=8,000 |
|---|---|
| `pending = collections::append(pending, leaf())` | 0.02 s |
| `pending = collections::take(pending, len(pending))` then append | 21.96–21.98 s |
| `collections::drop(pending, len(pending))` (returns *no* elements) then append | 0.02 s |
| `pending = collections::drop(pending, 0)` (returns *every* element) then append | 21.65–22.16 s |
| `xs = collections::take(xs, len(xs))` then append, over `List OF Integer` | 0.61–0.62 s |
| `xs = collections::drop(xs, 0)` then append, over `List OF Integer` | 0.61–0.62 s |

So it is not `take` in particular: the cost tracks the number of elements **returned** (drop-none is
free, drop-all is not). Self-reassigning a list through `take`/`drop` copies the whole list every
time — quadratic for any element type — and an element type that reaches a type cycle additionally
pays a deep graph copy per element, ~35× more (21.96 s vs 0.62 s at the same n). `collections::append`
on the identical local is O(1) amortized because it has an in-place arm (`.ai/collections.md`,
"In-place MUT append"); `take`/`drop` have no equivalent last-read/move arm. That compiler-side gap
is filed as its own bug document per this plan's "no changes to `src/`" non-goal; it does **not**
block plan-138, because the chosen builder never performs that operation.

The chosen design never rewrites a shared list: each
element's children are collected in a **frame-local** `MUT kids AS List OF Node` that only ever gets
`collections::append` (the in-place arm, `.ai/collections.md` "In-place MUT append"), and the
finished `Element` is returned up to the caller's own local list. Recursion depth is bounded by
`DEPTH_LIMIT` (§7), which is what makes recursive descent safe here; the 256-level deep shape
measures it.

Consequences for the rest of the plan: Phase 4 builds with this design ("the Phase 1 design").
§3's "Design uncertainty concentrates in the builder" is now settled, and the §3 rejected-alternatives
list gains the pending-children builder beside `dom`'s frame stack.

## Summary

The engineering risk in A is the builder under copy-on-`get` (Phase 1, measured first) and the
breadth of well-formedness rules (Phases 3–4, each rule a test). The compiler, `dom`, and every
existing package are untouched.
