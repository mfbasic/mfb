# xml — a strict XML 1.0 reader and writer, with XPath 1.0

`xml::parse` turns XML text into a tree; `xml::stringify` writes a tree back; and
`xml::evaluate`, `xml::select` and `xml::valueOf` query it with XPath 1.0.

```mfb
IMPORT xml

LET doc AS xml::Document = xml::parse(text)
LET root AS xml::Element = xml::root(doc)
```

File I/O stays separate:

```mfb
LET text AS String = fs::readText(path)
LET doc AS xml::Document = xml::parse(text)
```

That is the same split `csv::parse`, `json::parse` and `yaml::parse` already use —
none of them owns filesystem access, and neither does this.

Full API and prose: `mfb pkg doc packages/xml/xml.mfp`, or `mfb doc packages/xml`
for the internals too.

## Why this is a package and not a built-in

A libxml2 binding would mean a large wrapper surface, an inherited CVE history,
and vendoring on Windows, where it is not a system library. More to the point,
reading XML means adopting a **compatibility policy**: what to do about DTDs,
about encodings, about namespace prefixes. A package can evolve that policy on
its own schedule instead of tying it to every MFB release. `yaml` is the
precedent.

## The tree

```
Document  children AS List OF Node
Node      Element | Text | Comment | ProcessingInstruction
Element   name AS String, attributes AS List OF Attribute, children AS List OF Node
Attribute name AS String, value AS String
Text      text AS String
Comment   text AS String
ProcessingInstruction  target AS String, data AS String
```

Attributes are a `List`, not a `Map`, so document order survives and duplicates
are caught while reading. `xmlns` and `xmlns:p` declarations are ordinary
attributes.

`xml::root(doc)` returns the `Element` **record**. To pass it where a `Node` is
wanted — `xml::stringify`, for instance — widen it through a typed binding
first, which is how every variant record reaches its union:

```mfb
LET node AS xml::Node = xml::root(doc)
io::print(xml::stringify(node))
```

## What it reads

Elements, attributes, text, CDATA sections, comments and processing
instructions; the five predefined entities (`&lt; &gt; &amp; &apos; &quot;`) and
numeric character references in both decimal and hexadecimal; the XML
declaration; a leading UTF-8 BOM; and namespace declarations, whose prefixes are
checked against the declarations in scope.

XML 1.0 line-end normalization (§2.11) and attribute-value normalization
(§3.3.3) are applied while reading, so a document round-trips with the text the
specification says it has.

Every attribute counts toward the node limit, as an XPath or DOM node does, so a
tag cannot exceed the bound by carrying many attributes.

## The compatibility policy

Every one of these is a deliberate decision. Each fails with an `Error`; none
silently changes the data.

| Decision | Behaviour | Code |
| --- | --- | --- |
| **No DTD, of any kind** | Any `<!DOCTYPE` is refused — with or without an internal subset, with or without an external ID. No entity declarations, no default attributes, no validation. An undeclared entity is therefore a well-formedness error, not a lookup. | `ErrUnsupported` |
| **UTF-8 only** | An `encoding` pseudo-attribute naming anything else is refused. Input is UTF-8; output is UTF-8. | `ErrUnsupported` |
| **XML 1.0 only** | A `version` other than `1.0` is refused. There is no XML 1.1 mode. | `ErrUnsupported` |
| **Namespace prefixes are checked, never resolved** | Names are kept as the raw `prefix:local` text. A prefix must be declared in scope; `xml` is pre-bound; `xmlns` may not be declared or bound; `xml` may not be rebound; two attributes may not share an expanded name. | `ErrInvalidFormat` |
| **Well-formedness is absolute** | A mismatched or unclosed tag, a second root element, text outside the root, a literal `]]>` in character data, `--` inside a comment, a reserved `xml` processing-instruction target, a colon in a processing-instruction target, or a character reference to a character XML 1.0 forbids. Nothing is repaired. | `ErrInvalidFormat` |
| **A literal `&` is refused** | `AT&T` is an error at the `&`. A reference is a predefined entity or a character reference, terminated by `;` directly after its name. | `ErrInvalidFormat` |
| **Duplicate attributes** | An element may not carry the same attribute name twice. | `ErrAlreadyExists` |
| **Nesting is bounded** | Elements may nest 256 levels, matching `json::parse`. | `ErrDepthExceeded` |
| **Documents are bounded** | A document may hold 1,000,000 nodes, attributes included. | `xml::ErrorNodeLimit` (`93140001`) |
| **No HTML** | No tag-soup recovery, no void elements, no case folding. `examples/browser/dom` is the forgiving HTML parser; this is its opposite. | — |

The writer refuses whatever it could not write faithfully, with
`ErrInvalidArgument`: a name that is not a valid XML name or carries two colons,
duplicate attributes or expanded names, a namespace declaration the reader would
reject, an undeclared prefix in a document, a character XML 1.0 forbids, a
comment holding `--` or ending in `-`, a processing instruction whose target is
reserved or holds a colon, or whose data holds `?>`, and a document without
exactly one root element.

## Content and formatting

The package draws a line between the two, and every guarantee is stated in terms
of it. **Content** is the tree after this projection:

1. comments and processing instructions are removed;
2. adjacent text is merged into one text node;
3. a whitespace-only text node **whose parent also has an element child** is
   layout, and is removed — any other text, including a whitespace-only node
   that is its parent's only content (`<a>   </a>`), is data and is kept byte
   for byte;
4. attributes compare as a set of (name, value) pairs — XML gives their order no
   meaning;
5. names compare as their raw `prefix:local` strings.

**Content survives** every read, write and pretty-print. **Formatting may
change.** So `xml::stringify(doc, 2)` re-indents freely between elements, but an
element holding text is written exactly as it is compactly — indenting inside
text would change the text — and so is an element whose only children are
comments, which has no element child to make its whitespace layout.

The reader keeps whitespace-only text nodes in the tree. Only the content
projection, and the pretty-printer, treat them as layout.

## Querying: the XPath 1.0 subset

```mfb
FOR EACH found IN xml::select(doc, "//book[@id='b2']")
  io::print(xml::textOf(found))
NEXT

LET total AS String = xml::valueOf(doc, "count(//book)")
```

| Construct | Examples |
| --- | --- |
| Absolute and relative paths | `/a/b`, `a/b`, `.`, `..` |
| Descendant-or-self | `//a`, `a//b`, `.//b` |
| Name tests | `name`, `p:name` (literal), `*`, `p:*` (every name with the prefix) |
| Node-type tests | `text()`, `node()`, `comment()`, `processing-instruction()` |
| Attributes | `@name`, `@*`, `@p:*` |
| Predicates | `[1]`, `[last()]`, `[position() < 3]`, `[@a]`, `[@a='v']`, `[b]`, `[b='v']`, chained `[..][..]` |
| Filter expression | `(//a)[1]`, `(//a/@id)[2]`, `(//a)[1]/b` |
| Operators | `or`, `and`, `=`, `!=`, `<`, `<=`, `>`, `>=`, `+`, `-`, `*`, `div`, `mod`, unary `-`, `\|` |
| Literals | `'…'`, `"…"`, numbers |
| Functions | `count`, `sum`, `name`, `local-name`, `string`, `boolean`, `not`, `number`, `string-length`, `normalize-space`, `concat`, `contains`, `starts-with`, `substring`, `substring-before`, `substring-after`, `translate`, `floor`, `ceiling`, `round`, `position`, `last`, `true`, `false` |

**Attributes are nodes.** An attribute is its own context node in a predicate or
a function: `//book/@id[. = 'b2']` tests the value, `.` selects the attribute,
`..` selects its element, and a step may follow it (`//book/@id/..`). An
attribute has no children, so a child step after one is empty.

**Prefixes are matched literally.** `p:name` selects elements whose name is the
text `p:name` — the reader never resolved the prefix, so neither does the query.
`local-name()` returns the part after the colon.

**`//a[1]` and `(//a)[1]` are different**, as XPath 1.0 intends: a predicate on a
step applies per context node (the first `a` of *each* parent), while a filter
expression applies to the whole result (the first `a` overall).

**Selecting the document node gives the root element.** `Node` has no form for
XPath's root node, so `xml::select(doc, "/")` — and `..` from the root element —
comes back as the document element rather than as the prolog beside it. The
prolog is still queryable: on `<!--note--><?pi data?><root><a/></root>`,
`count(//comment())` and `count(//processing-instruction())` are each `1` and
`count(//node())` is `4`. Inside an expression the root node keeps its own
identity, so `name(/)` is the empty string, as XPath 1.0 §4.1 says.

**Errors come in two kinds.** `ErrInvalidFormat` means the text is not a valid
XPath expression, or applies a function to the wrong number of arguments or to a
value of the wrong type (`count('x')`). `ErrUnsupported` means the text is valid
XPath outside this subset: an unabbreviated axis such as `child::a`, a variable,
or one of the functions that need a namespace resolver or a DTD — `namespace-uri`,
`lang` and `id`. A union that mixes attributes with elements is also unsupported,
because `xml::XPathValue` holds one kind of result or the other.

Four entry points:

| Function | Returns |
| --- | --- |
| `xml::evaluate(doc, expr)` | an `xml::XPathValue`: nodes, attributes, a string, a number or a boolean |
| `xml::select(doc, expr)` | `List OF Node`, in document order |
| `xml::selectAttributes(doc, expr)` | `List OF Attribute`, in document order |
| `xml::valueOf(doc, expr)` | the XPath string-value of any result |

### Numbers

XPath numbers are IEEE doubles, with `NaN` and the two infinities as ordinary
values. MFBASIC's `Float` cannot hold `NaN` or an infinity, so the runtime raises
for them. The package therefore carries those values beside the number, and the
text form is where they show.

```mfb
LET third AS String = xml::valueOf(doc, "1 div 3")          ' 0.3333333333333333
LET none AS String = xml::valueOf(doc, "string(1 div 0)")   ' Infinity
```

**The text of a number is the shortest decimal that reads back as the same
double** (§4.2), with no exponent: `0.1 + 0.2` is `0.30000000000000004`, and
`10000000000000000000000` is written out in full. An integral value has no point.

**`number()` follows the §3.5 grammar.** An optional minus sign, then digits with
an optional point, or a point with digits, surrounded by XML whitespace. So `5.`
and `-.5` are numbers, while `1e5`, `+5`, `Infinity` and `0x10` are `NaN`. A
numeral too large for a Float is an infinity.

**`mod` is exact.** It is the remainder that takes the sign of the dividend, as C's
`fmod` computes it, even when the quotient is far beyond an Integer:
`10000000000000000000000 mod 7` is `4`.

**`floor`, `ceiling` and `round` follow §4.4.** `round` sends a half toward positive
infinity, so `round(-2.5)` is `-2`, and it does not drift near a half:
`round(0.49999999999999994)` is `0`.

`xml::evaluate` refuses a `NaN` or an infinite result, because it cannot return
one as a `Float`; read it with `xml::valueOf`, which yields `"NaN"`, `"Infinity"`
or `"-Infinity"`.

### Comparisons

A boolean on either side of a comparison makes it a boolean comparison, and a
node-set is converted to a boolean as a whole first: `//empty = true()` is true
when the set has a member, whatever that member's string-value is. A relational
operator reads both sides as numbers, so `'0' < true()` is true.

## Writing

```mfb
LET compact AS String = xml::stringify(doc)          ' one line
LET pretty AS String = xml::stringify(doc, 2)        ' two spaces per level
LET tabbed AS String = xml::stringify(doc, "\t")     ' a tab per level
```

A count clamps to `0..10` spaces and an indent string is truncated to its first
10 characters; `0` and `""` mean compact, byte for byte. The same three forms
take a `Node` instead of a `Document`, and write a fragment with no XML
declaration. This mirrors `json::stringify`.

A fragment is written for embedding in a document that declares its prefixes, so
a prefix it uses is not checked; every other rule still applies to it. A
document is checked in full, and what it writes reads back.

## Verification

The package is checked in four ways, and each is part of its release gate.

- **Its own tests** (`mfb test packages/xml`) pin every rule in this README, from
  both sides: the reader refuses what the writer refuses, and the writer refuses
  what the reader would refuse to read back.
- **The compiler's audit** (`mfb audit packages/xml`) reports no errors, and every
  `EXAMPLE` in the API docs builds and runs (`check-doc-examples.sh`).
- **Differential oracles** (`oracle/`) compare the package with two independent
  implementations — a Node one (saxes, xmldom, xpath) and a Rust one (roxmltree,
  quick-xml, xpath-eval) — as equal peers. They cover the W3C XML Conformance Test
  Suite, a hand-written corpus, the XPath corpus, and seeded fuzzing in both
  directions. A case passes only when all three agree. The one declared divergence
  is documented in `oracle/divergences.json`, with its specification citation.
- **Performance** is measured by the oracle's `perf` mode on 100,000-node flat,
  deep and wide documents, against a three-second budget.

See [`oracle/README.md`](oracle/README.md) for how to run the oracles.

## Stability

The package follows [Semantic Versioning](CHANGELOG.md). From 1.0.0 on, a change to
the exported API, to an error code, or to the result for a given input is a
breaking change, and needs a new major version. A change to the XPath subset that
adds a function or a syntax form is a minor version. Every change is recorded in
[`CHANGELOG.md`](CHANGELOG.md).
