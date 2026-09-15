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
