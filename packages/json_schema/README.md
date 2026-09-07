# json_schema — a JSON Schema Draft 2020-12 validator

`json_schema::compile` reads a schema and `json_schema::validate` measures
instances against it.

```mfb
IMPORT json_schema
IMPORT json

LET schema AS json_schema::Schema = json_schema::compileText(text)
FOR EACH problem IN json_schema::validate(schema, json::parse(body))
  io::print(json_schema::describe(problem))
NEXT
```

A schema is an ordinary `json::Json` value, because a JSON Schema *is* a JSON
document: `json::parse` of `{"type": "string"}` is already the schema. Nothing
here invents a second tree to hold one, and an importer that only wants to check
data never names `json::` at all — `compileText` and `validate` take text and a
parsed value respectively.

Full API and prose: `mfb pkg doc packages/json_schema/json_schema.mfp`, or
`mfb doc packages/json_schema` for the internals too.

## Two steps, and why

Everything that can be wrong with a schema **on its own** is found once, by
`compile`, before any instance is seen: a keyword holding the wrong JSON type, a
`$ref` that names nothing, a `pattern` that is not a valid regular expression, a
construct outside the supported subset. `compile` fails for those.

`validate` therefore never fails for a schema reason. Every outcome it has is a
`List OF json_schema::ValidationError`, empty when the instance is valid. That
is the split the two kinds of failure deserve: a broken schema is a program
error and stops you; invalid data is data, and comes back as a list you can
count, sort, and show to somebody.

```mfb
TYPE ValidationError
  keyword      AS String   ' "type", "required", "$ref", ...
  instancePath AS String   ' a JSON Pointer into the instance
  schemaPath   AS String   ' a JSON Pointer into the schema
  message      AS String
END TYPE
```

Errors are not truncated at the first one: a form with three bad fields reports
three. `schemaPath` points into the compiled document, so a failure inside a
`$ref` names where the failing keyword actually lives rather than where the
reference was written.

## What it implements

The Core and Validation vocabularies of Draft 2020-12, with the applicator and
unevaluated vocabularies:

* **Applicators** — `allOf`, `anyOf`, `oneOf`, `not`, `if`/`then`/`else`,
  `dependentSchemas`, `properties`, `patternProperties`,
  `additionalProperties`, `propertyNames`, `prefixItems`, `items`, `contains`,
  `unevaluatedProperties`, `unevaluatedItems`.
* **Assertions** — `type`, `enum`, `const`, `multipleOf`, `maximum`,
  `exclusiveMaximum`, `minimum`, `exclusiveMinimum`, `maxLength`, `minLength`,
  `pattern`, `maxItems`, `minItems`, `uniqueItems`, `maxContains`,
  `minContains`, `maxProperties`, `minProperties`, `required`,
  `dependentRequired`.
* **Identifiers and references** — `$id`, `$anchor`, `$defs`, `$ref`,
  `$schema`, with full RFC 3986 URI-reference resolution, so a relative `$id`
  rebases every reference beneath it.
* **Annotations**, checked for shape and otherwise ignored — `title`,
  `description`, `default`, `examples`, `deprecated`, `readOnly`, `writeOnly`,
  `$comment`, `format`, `contentEncoding`, `contentMediaType`, `contentSchema`.

**1213 of the 1301 cases in the official
[JSON-Schema-Test-Suite](https://github.com/json-schema-org/JSON-Schema-Test-Suite)
for Draft 2020-12 pass, and not one of the remaining 88 is a wrong verdict** —
every one is a refusal in a documented area below. `packages/json_schema/oracle`
reproduces that, and differential-tests the package against ajv besides.

## The compatibility policy

Every one of these is a deliberate decision, and every one fails at `compile`
rather than being approximated — so a schema is never validated against
something other than what it says.

| Decision | Behaviour | Code |
| --- | --- | --- |
| Nothing is retrieved | A `$ref` must resolve inside the document being compiled. One naming an external resource fails. | `errorCode::ErrNotFound` |
| No dynamic references | `$dynamicRef`, `$dynamicAnchor`, `$recursiveRef` and `$recursiveAnchor` are refused. | `errorCode::ErrUnsupported` |
| One dialect | A `$schema` naming another metaschema selects a different vocabulary set, and is refused rather than guessed at. | `errorCode::ErrUnsupported` |
| `format` is an annotation | Which is the Draft 2020-12 default, and what every other validator does unless a format vocabulary is switched on. | — |
| A malformed schema is refused | `{"type": 5}` and `{"required": "a"}` are errors, not keywords to ignore. | `errorCode::ErrInvalidFormat` |
| A `pattern` is translated exactly, or refused | See below. | `ErrInvalidFormat` / `ErrUnsupported` |

### Why nothing is retrieved

A `$ref` is a URI, and a schema is untrusted input in exactly the way an
instance is. Fetching a URL that untrusted input names is a server-side request
forgery: it reaches whatever the *server* can reach, including link-local
metadata endpoints and services behind the firewall, and it brings recursion
limits, response-size limits and cache poisoning with it. None of that is a
policy this package can pick for you. Compose the documents yourself — resolve
each `$ref` into one document under your own rules — and compile the result.

### Regular expressions

JSON Schema defines `pattern` and `patternProperties` against **ECMA-262**. The
`regex` package has a portable dialect of its **own** (`mfb spec stdlib regex`),
close to ECMA-262 but not identical: `\d` is Unicode `Nd` rather than `[0-9]`,
`\a` is BEL rather than a syntax error, `.` excludes only `\n`, and `[]` is a
parse error rather than an empty class. Handing a schema author's pattern
straight to `regex::match` would accept it and quietly mean something else.

So it is translated. Every construct whose ECMA-262 meaning can be written
exactly in the `regex` dialect is rewritten into that spelling; every construct
whose meaning cannot be is **refused**. Nothing is passed through on the hope
that the two agree.

| ECMA-262 | Becomes |
| --- | --- |
| `\d` `\D` `\w` `\W` `\s` `\S` | explicit ASCII / ECMA-262 classes |
| `.` | a class excluding all four line terminators |
| `[]` `[^]` | an explicit empty / whole-code-point-space class |
| `\uHHHH`, a surrogate pair, `\u{...}`, `\cX` | `\x{...}` |
| `\p{Uppercase_Letter}`, `\p{gc=Nd}`, `\p{Script=Greek}` | `\p{Lu}`, `\p{Nd}`, `\p{sc=Greek}` |
| `&&` and `[:alpha:]` inside a class | escaped, so they stay literal |
| `(?<name>...)` | an ordinary capturing group (nothing here observes a capture) |

Refused with `errorCode::ErrUnsupported`: lookaround, backreferences,
`Script_Extensions`, a binary property the engine does not implement, a negated
shorthand inside a character class (`[a\D]`), and an escape naming a lone
surrogate. Refused with `errorCode::ErrInvalidFormat`: anything ECMA-262 itself
rejects under the `u` flag, including `\a`, `\A`, `\z`, a lone `}` or `]`, and
`(?i)`.

The one construct carried over rather than translated is **`\b` and `\B`**.
ECMA-262 defines a word character as `[A-Za-z0-9_]`; the `regex` engine's word
test is Unicode-aware, and a word boundary cannot be expressed without
lookaround. The two agree on any text whose word characters are ASCII, and
differ where they are not.

`json_schema::patternMatches` and `json_schema::translatePattern` expose that
seam, so a disagreement can be read rather than guessed at.

### Numbers

`json::JsonNum` holds a `Float` (`mfb man json`), so an integer beyond 2^53 and
a decimal with more digits than binary64 holds are rounded at parse time, and
`const`, `enum`, `multipleOf` and `uniqueItems` compare the rounded values. This
is JavaScript's model exactly — which is the model the reference validators use,
so it is a divergence from the *specification*, not from the ecosystem. A schema
that must distinguish `1e400` from `2e400`, or `1.0000000000000001` from `1.0`,
needs a lossless numeric representation that the `json` package does not have.

## Bounds

A schema is untrusted input in exactly the way an instance is, and none of these
is reachable by a schema that terminates.

* **Depth** — evaluation nests at most 200 levels (`errorCode::ErrDepthExceeded`).
* **Reference loops** — a `$ref` that re-enters the same subschema at the same
  instance location has closed a loop with no progress on either side, and is
  refused with a distinct code (`93120002`) rather than left to hit the depth
  limit.
* **Work** — 10000 subschema evaluations plus 100 per value in the *instance*
  (`93120001`). A fixed total would be the wrong shape twice over: too small and
  an ordinary large document stops validating, too large and a one-byte instance
  can still be made to cost minutes. Charging per instance node makes the total
  linear in what the caller handed in — work they already paid for once at
  `json::parse`. Ten nested two-branch `anyOf`s demand 2^10 evaluations from a
  few hundred bytes of schema; against a scalar instance they get 10100.

## Why this is a package and not a built-in

JSON has a crisp six-variant data model and the compiler already consumes it.
JSON Schema does not: it is a *policy*. Which draft, which vocabularies, whether
`format` asserts, whether a `$ref` may be fetched and from where, what a
`pattern` means when the host regex engine is not ECMA-262 — every one of those
is a decision, and every one of them is a decision that reasonable users make
differently. A package can carry that policy explicitly, document it, and evolve
it on its own schedule instead of tying it to every MFB release.

## Testing

```sh
mfb test packages/json_schema               # the package's own suite (117 cases)
packages/json_schema/check-doc-examples.sh  # every DOC example compiles and runs
packages/json_schema/oracle/run.sh          # differential-test against ajv and the official suite
```

The first pins what this project decided. The second guards the documentation:
a DOC block's prose is a string the compiler never reads, so an example that
stopped compiling would say nothing until a reader tried it. The third checks
the decisions themselves against implementations written by other people from
the same specification, and against the official suite's ground truth — see
`oracle/README.md`, including the bugs it found on each side.
