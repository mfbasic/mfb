# Changelog

All notable changes to the `xml` package. The package follows
[Semantic Versioning](https://semver.org/): from 1.0.0 on, a breaking change to the
exported API, to an error code, or to a result for a given input requires a new major
version. A change to which documents are refused, or to which error a refusal carries,
counts as a change to a result.

## 1.0.0

The first stable release. The package was 0.1.0 while its design was settled and was
never published, so the entries below describe the whole 1.0.0 surface.

### Reading and writing

- `xml::parse` reads XML 1.0 strictly: well-formedness, the namespace-prefix rules,
  the five predefined entities and character references, CDATA, comments, processing
  instructions, the XML declaration and a leading UTF-8 BOM. Line ends (§2.11) and
  attribute values (§3.3.3) are normalized while reading.
- Refused, each with the error code in the README's compatibility table: any DOCTYPE,
  an encoding other than UTF-8, a version other than 1.0, malformed or ill-formed
  input, duplicate attributes, nesting past 256 levels, and documents past 1,000,000
  nodes.
- Every attribute counts toward the node limit, as an XPath or DOM node does. A tag
  with a very large number of attributes is bounded like any other part of a document.
- `xml::stringify` writes a document (with its declaration) or a fragment, compact or
  pretty-printed. The writer refuses any tree the reader would refuse to read back,
  with `ErrInvalidArgument`. That covers the name rules, duplicate attributes and
  expanded names, namespace declarations, undeclared prefixes in a document (a fragment
  is written for embedding, so its prefixes are not checked), and a processing-instruction
  target holding a colon.

### Querying

- `xml::evaluate`, `xml::select`, `xml::selectAttributes` and `xml::valueOf` evaluate
  XPath 1.0 over the tree. The subset is the abbreviated syntax with the core function
  library. The functions that need a namespace resolver or a DTD (`namespace-uri`,
  `lang`, `id`) are refused as unsupported.
- An attribute is a context node: a predicate on `@id` tests its value, `.` selects it,
  and `..` selects its element. A step may follow an attribute (`//a/@id/..`), and a
  filter may select from an attribute set (`(//a/@id)[2]`).
- `prefix:*` and `@prefix:*` name tests select every element or attribute with a prefix,
  matched as the literal text the reader stores.
- Numbers follow XPath 1.0. The text of a number is the shortest decimal that reads back
  as the same double (§4.2), with no exponent: `1 div 3` is `0.3333333333333333`, and
  `10000000000000000000000` is written out in full. `number()` accepts the §3.5 grammar
  only, so `1e5`, `+5` and `Infinity` are NaN, and `5.` is 5. `mod` is the exact
  remainder that takes the sign of the dividend, and `floor`, `ceiling` and `round`
  follow §4.4.
- A boolean on either side of a comparison converts a node-set as a whole, so
  `//empty = true()` is true when the set has a member.

### Behaviour a caller may notice

- A call with the wrong number of arguments, or with an argument of the wrong type,
  is an XPath error, `ErrInvalidFormat`, not an unsupported feature. The same goes for
  `sum()` and `count()` given a value that is not a node-set.
- A malformed reference is reported as malformed. `AT&T` is refused as a literal `&`,
  with the position of the `&`, rather than as an undeclared entity.
- Trees built by hand are bounded too: `xml::stringify`, `xml::textOf`, `xml::evaluate`
  and the other tree walks refuse one nested past 256 levels with `ErrDepthExceeded`,
  instead of exhausting the stack.
- The content comparison used to test round trips escapes its delimiters, so two
  different trees cannot share a key.

### Verification

- The package's own test suite (`mfb test packages/xml`) covers every rule above.
- `mfb audit packages/xml` reports no errors, and every `EXAMPLE` in the API docs builds
  and runs (`check-doc-examples.sh`).
- The differential oracles (`oracle/`) compare the package with two independent
  implementations on the W3C XML Conformance Test Suite, a hand-written corpus, the XPath
  corpus and seeded fuzzing in both directions. Their declared divergences are listed
  in `oracle/divergences.json`, each with its specification citation.
