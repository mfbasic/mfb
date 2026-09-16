# xml — a strict XML 1.0 reader and writer

`xml::parse` turns XML text into a tree; `xml::stringify` writes a tree back.

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

This package is differential-tested against two independent implementations — a
Node oracle (saxes, @xmldom/xmldom) and a Rust oracle (roxmltree, quick-xml) — as
equal peers, over a hand-written corpus, the W3C XML Conformance Test Suite, and
seeded fuzzing in both directions. See [`oracle/README.md`](oracle/README.md).

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

## The compatibility policy

Every one of these is a deliberate decision. Each fails with an `Error`; none
silently changes the data.

| Decision | Behaviour | Code |
| --- | --- | --- |
| **No DTD, of any kind** | Any `<!DOCTYPE` is refused — with or without an internal subset, with or without an external ID. No entity declarations, no default attributes, no validation. An undeclared entity is therefore a well-formedness error, not a lookup. | `ErrUnsupported` |
| **UTF-8 only** | An `encoding` pseudo-attribute naming anything else is refused. Input is UTF-8; output is UTF-8. | `ErrUnsupported` |
| **XML 1.0 only** | A `version` other than `1.0` is refused. There is no XML 1.1 mode. | `ErrUnsupported` |
| **Namespace prefixes are checked, never resolved** | Names are kept as the raw `prefix:local` text. A prefix must be declared in scope; `xml` is pre-bound; `xmlns` may not be declared or bound; `xml` may not be rebound; two attributes may not share an expanded name. | `ErrInvalidFormat` |
| **Well-formedness is absolute** | A mismatched or unclosed tag, a second root element, text outside the root, a literal `]]>` in character data, `--` inside a comment, a reserved `xml` processing-instruction target, or a character reference to a character XML 1.0 forbids. Nothing is repaired. | `ErrInvalidFormat` |
| **Duplicate attributes** | An element may not carry the same attribute name twice. | `ErrAlreadyExists` |
| **Nesting is bounded** | Elements may nest 256 levels, matching `json::parse`. | `ErrDepthExceeded` |
| **Documents are bounded** | A document may hold 1,000,000 nodes. | `xml::ErrorNodeLimit` (`93140001`) |
| **No HTML** | No tag-soup recovery, no void elements, no case folding. `examples/browser/dom` is the forgiving HTML parser; this is its opposite. | — |

The writer refuses whatever it could not write faithfully, with
`ErrInvalidArgument`: a name that is not a valid XML name or carries two colons,
duplicate attributes, a character XML 1.0 forbids, a comment holding `--` or
ending in `-`, a processing instruction whose target is reserved or whose data
holds `?>`, and a document without exactly one root element.

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

## Querying: an XPath 1.0 subset

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
| Name tests | `name`, `p:name` (literal), `*` |
| Node-type tests | `text()`, `node()`, `comment()`, `processing-instruction()` |
| Attributes | `@name`, `@*` |
| Predicates | `[1]`, `[last()]`, `[position() < 3]`, `[@a]`, `[@a='v']`, `[b]`, `[b='v']`, chained `[..][..]` |
| Filter expression | `(//a)[1]` |
| Operators | `or`, `and`, `=`, `!=`, `<`, `<=`, `>`, `>=`, `+`, `-`, `*`, `div`, `mod`, unary `-`, `\|` |
| Literals | `'…'`, `"…"`, numbers |
| Functions | `count`, `contains`, `starts-with`, `string`, `normalize-space`, `not`, `position`, `last`, `name`, `local-name`, `concat`, `string-length`, `number`, `sum`, `true`, `false`, `boolean` |

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

Four entry points:

| Function | Returns |
| --- | --- |
| `xml::evaluate(doc, expr)` | an `xml::XPathValue`: nodes, attributes, a string, a number or a boolean |
| `xml::select(doc, expr)` | `List OF Node`, in document order |
| `xml::selectAttributes(doc, expr)` | `List OF Attribute`, in document order |
| `xml::valueOf(doc, expr)` | the XPath string-value of any result |

Anything recognizable as XPath but outside the subset — an unabbreviated axis
such as `child::a`, a variable, or a function not listed above — is refused with
`ErrUnsupported` rather than misread. Text that is not XPath at all is
`ErrInvalidFormat`.

Two consequences of the language are worth knowing, both about numbers.

**`NaN` and `±Infinity` are values in XPath, and MFBASIC's `Float` cannot hold
either** — the runtime raises instead. `xml::valueOf` prints them correctly
(`"NaN"`, `"Infinity"`, `"-Infinity"`), so `xml::valueOf(doc, "string(1 div 0)")`
works; `xml::evaluate` refuses such a result rather than inventing a number for
it.

**Numbers are printed with two decimals.** XPath 1.0 §4.2 asks for as many digits
as are needed to tell a double from its neighbours, so `sum(//price)` over
`9.99`, `12.50` and `8.25` is `30.740000000000002`. MFBASIC's `toString(Float)`
is fixed at two decimal places, so `xml::valueOf` says `30.74` — and `1 div 3` is
`0.33`, not `0.3333333333333333`. The arithmetic is unaffected; only the text
form is. A caller who needs full precision should take the `XNumber` from
`xml::evaluate` and format the `Float` itself.

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
