# oracle — differential-test `packages/xml` against two independent implementations

A Node project and a Rust project whose only job is to read and write the same
XML the package does, and disagree loudly. The package's own `TESTING` blocks pin
what this package *decided*; they cannot catch a misreading of the XML
specification that the tests share. This can.

```sh
npm install --prefix packages/xml/oracle
cargo build --release --manifest-path packages/xml/oracle/rust/Cargo.toml

mfb build packages/xml
mkdir -p packages/xml/oracle/probe/packages
cp packages/xml/xml.mfp packages/xml/oracle/probe/packages/xml.mfp
mfb build packages/xml/oracle/probe

packages/xml/oracle/fetch-xmlconf.sh          # the W3C suite, once

node packages/xml/oracle/diff.mjs             # every mode
node packages/xml/oracle/diff.mjs corpus      # one mode
node packages/xml/oracle/diff.mjs fuzz-read --seed 7 --count 2000
```

Exit status is 0 iff every case agreed, or diverged for a reason declared in
`divergences.json`.

## Three sides, no reference

The package, the Node oracle and the Rust oracle are **equal peers**. A case
passes only when all three agree. Where the two oracles disagree with each
*other*, the case fails as an oracle disagreement until the specification or the
W3C suite settles it — never by majority vote, which would hide an oracle bug
behind two votes.

That rule has already earned its keep. Of the 22 disagreements the W3C suite
produced on its first run, exactly one was a package defect; the rest were
defects in the harness and in this plan's own policy rules. A two-sided oracle
would have had to guess which side was wrong.

| Side | Reads with | Writes with |
| --- | --- | --- |
| package | `xml::parse` | `xml::stringify` |
| Node | [`saxes`](https://www.npmjs.com/package/saxes) | [`@xmldom/xmldom`](https://www.npmjs.com/package/@xmldom/xmldom) |
| Rust | [`roxmltree`](https://crates.io/crates/roxmltree) | [`quick-xml`](https://crates.io/crates/quick-xml) |

**Why saxes reads and xmldom only writes.** xmldom's parser is lenient, and a
lenient oracle agrees with our acceptance bugs — the one failure mode an oracle
may not have. saxes is strict and namespace-aware. xmldom's DOM and serializer
are still useful, as an independent *writer*.

**Why the wrappers check policy themselves.** Whether a library refuses
`version="1.1"`, a non-UTF-8 encoding, `standalone="YES"`, a reserved `xml`
processing-instruction target or a character reference to a surrogate is an
accident of that library. Each wrapper checks those itself, so agreement between
the three sides never rests on how permissive a dependency happens to be.

## How the sides talk

Every side answers one JSON job file and writes one JSON document back, holding
one result per case in the same order. A whole job per process, not a process per
case: a fuzz run asks tens of thousands of questions.

```
read    {"cases":[{"id":"c1","xml":"<a/>"}]}
write   {"cases":[{"id":"c1","tree":["doc",[...]],"indent":"  "}]}

        {"results":[{"id":"c1","ok":true,"content":["doc",[...]]},
                    {"id":"c2","ok":false,"kind":"parse","reason":"..."}]}
```

A **refusal is a result**, on stdout with exit 0. That leaves a non-zero exit or
an unparseable document meaning exactly one thing — that side broke — which is
what the `mutate` mode checks for. Two refusals count as agreement without
comparing `kind` or `reason`: three implementations have three vocabularies for
"no", and demanding the same words would make the harness fail on wording rather
than on meaning.

### The content envelope

```
["doc", [<node>, ...]]
<node> := ["e", name, [[attrName, value], ...sorted], [<node>, ...]]
        | ["t", text]
```

This is plan-138-A §4's content projection: comments and processing instructions
removed, adjacent text merged, whitespace-only text dropped where the parent also
has an element child, attributes sorted, names kept as raw `prefix:local` text.
**All three sides compute it independently.** A projection computed once and
shared would hide exactly the disagreements this oracle exists to find.

## Modes

| Mode | Input | Asserts |
| --- | --- | --- |
| `corpus` | `corpus/*.xml` | three-way agreement, or a declared divergence — and a declared divergence that stops diverging fails, so the entry gets removed |
| `xmlconf` | the W3C XML Conformance Test Suite | per test: XML 1.1 and Namespaces 1.1 tests, and tests whose `EDITION` omits the fifth, do not apply; any document with a DOCTYPE is refused by policy; `not-wf` is refused; everything else is accepted with equal content |
| `fuzz-read` | random trees written out in six styles | every side reads back **the content of the tree that was written** — not merely the same thing as each other |
| `fuzz-write` | random trees written by all three writers | both readers read every writer's output back with the tree's content |
| `roundtrip` | corpus + random trees | `content(read(write(read(x)))) = content(read(x))`, compact and pretty |
| `mutate` | corpus documents with random byte damage | robustness, not correctness: the probe always answers a well-formed envelope, exits 0, and finishes inside 30 s. Agreement is *reported*, never asserted — damaged input has no right answer |
| `perf` | generated 100k-node flat, deep and wide documents | `xmlprobe read` of each within 3.00 s |
| `xpath` | `corpus/xpath/*.json` — 148 expressions over `corpus/catalog.xml` | the three XPath engines agree on the result's kind and value |
| `fuzz-xpath` | expressions drawn from each random tree's own names, attributes and text | the same, over documents nobody wrote by hand |

The XPath modes run over documents with **no namespace declarations**, and that
is a scope decision rather than an oversight. XPath 1.0's unprefixed name test
matches only no-namespace elements, so on a document with `xmlns="urn:x"` both
oracles correctly select *nothing* for `//book`, while this package — which never
resolves a prefix — matches the literal name and selects every book. A resolver
bridges a prefixed name; nothing bridges a default namespace. Neither side is
wrong, so such documents are outside these modes.

The XPath engines are a third independent pair: `xpath-eval` over the same
roxmltree parse on the Rust side, and npm `xpath` over a DOM built from saxes
events on the Node side. A number is compared as its XPath `string()` text, so
all three must agree on §4.2 formatting, not merely on the value.

`mutate`'s damage is a bit flip, a deleted byte, a truncation, an inserted `<`,
`&`, `]]>`, CR, NUL or `</`, or a raw `0xFF`. A mutation that leaves the document
invalid UTF-8 is skipped with a printed count: it cannot cross the JSON job
boundary, and MFBASIC's `String` cannot hold it — the same wall the W3C suite's
87 non-UTF-8 files hit.

`fuzz-read` and `fuzz-write` compare against the generating tree rather than only
against each other, because three implementations agreeing on a wrong answer
would otherwise pass — and that is precisely how the quick-xml indent bug below
was caught. `--seed` and `--count` replay any failure; both are printed on every
failure line.

The six `fuzz-read` styles vary what readers actually differ on: compact,
two-space and tab indentation, single-quoted attributes, CDATA for text,
character references for non-ASCII, and `<a></a>` versus `<a/>`.

## What it found

| Where | What | Fixed by |
| --- | --- | --- |
| **package** | A colon in a processing-instruction target was accepted. Namespaces in XML 1.0 erratum NE08 makes a target an NCName, and the suite marks `<?a:b bogus?>` not-well-formed (`rmt-ns10-042`). The reader checked QNames for elements and attributes, but not for PI targets. | `read.mfb`, behind a failing case in `test_refuse.mfb` |
| harness (Node) | xmldom's serializer writes a literal carriage return, which §2.11 normalizes to a line feed on the way back in — so the text came back changed. 480 of 18,000 write cases. | `oracle.mjs` writes `&#13;` |
| harness (Rust) | quick-xml's `Writer::new_with_indent` indents inside elements holding text, changing their content. | the Rust side applies the package's layout rule itself |
| harness (Rust) | roxmltree accepted nine things the policy refuses: reserved `xml` PI targets, a target not followed by whitespace, a colon in a target, `<:foo/>`, `xmlns:a=""` undeclaration, `xmlns:xmlns`. | checks in the wrapper |
| harness (Rust) | An explicitly declared `xmlns:xml` never reaches `namespaces()`; names arrive resolved, and `namespaces()` returns everything in scope rather than what an element declares. | recovered via `Node::range()`, and declarations diffed against the parent's scope |
| harness (generator) | It indented an element whose only children were processing instructions — with no element child, that whitespace becomes DATA. The same bug the package's own writer had. | `generate.mjs`, same rule |
| plan | `<!DOCTYPE` was detected by substring, so three suite documents holding that text inside a comment, a PI and a CDATA section were misjudged. | strip those first |
| plan | `RECOMMENDATION="XML1.1"` was read as "must refuse". XML 1.0 **Fifth Edition** adopted XML 1.1's name characters, so `rmt-016` and `rmt-019` are legal here; the suite says which editions a test applies to via `EDITION`. | honour `EDITION` |
| **package** | A union of two ATTRIBUTE sets (`//book/@id \| //book/@year`) was refused as "a union needs a node-set". Two attribute sets union like any others; only a union that MIXES attributes with elements is refused. | `xpath_eval.mfb`, behind a failing case |
| **package** | That union then returned its attributes in the wrong ORDER — all the `id`s, then all the `year`s. A union yields a node-set, and a node-set is in document order, so the two interleave. | ordered insertion, with a case asserting the order |
| harness (Node) | The DOM built from saxes events kept the whitespace text nodes from outside the document element, so `//text()` and `//node()` returned two more nodes than either other side. XPath 1.0 §5.1 gives the root node no text children. | `oracle.mjs` ignores text outside the root |
| harness (both oracles) | Selecting the document node itself (`.` at the top) rendered three different ways, though all three had selected the same node. It is now reported as the document element, which is what the package does. | envelope rule in both oracles |
| **package** | Selecting the document node returned the PROLOG too — `//a/..` on a document opening with a comment came back with the comment beside the root element. The document node has no `Node` form here, so it is reported as the document element; the prolog is not the document. | `xpath_eval.mfb`, behind a failing case in `test_xpath_edges.mfb` |
| harness (Node) | npm `xpath` implements `string-length`, `substring` and `translate` on raw JavaScript strings, which count UTF-16 code units — so every character above the BMP counted twice. `string-length(string(//Ωmega))` said 4 where the package and roxmltree said 3. XPath 1.0 §4.2 defines all three in terms of *characters*. 52 of 20,000 `fuzz-xpath` cases. | `oracle.mjs` supplies its own three, over code points |

Every row above was resolved by fixing the side that was wrong. `divergences.json`
holds exactly one thing, and it is not a disagreement about XML or XPath:
`sum(//price)` is `30.740000000000002` in both oracles and `30.74` in the
package, because MFBASIC's `toString(Float)` is fixed at two decimal places. The
arithmetic agrees; only the text form differs.

## Layout

```
README.md            this file
package.json         saxes, @xmldom/xmldom, xpath
oracle.mjs           the Node side: read (saxes), write (xmldom), evaluate (xpath)
generate.mjs         the seeded tree generator and its style writer
diff.mjs             the runner: modes, three-way comparison, divergences
divergences.json     case → reason, with a spec section or W3C test id
fetch-xmlconf.sh     download and unpack the W3C suite into xmlconf/
corpus/              hand-written documents, including refusals
rust/                the Rust side: roxmltree + quick-xml
probe/               the MFBASIC side: the package's only consumer here
```

`node_modules/`, `rust/target/`, `probe/build/`, `probe/packages/` and
`xmlconf/` are ignored — the suite is fetched rather than vendored, following
`packages/mustache/oracle/fetch-spec.sh`.
