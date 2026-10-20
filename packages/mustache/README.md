# mustache — Mustache v1.4 templates over `json::Json`

`mustache::render` turns a template and a `json::Json` value into text;
`mustache::renderWith` adds a `Map OF String TO String` of partials. Both take an
optional output limit, and every failure is a registry error code.

```mfb
IMPORT mustache
IMPORT json

LET html AS String = mustache::render(template, context)
```

The context is an ordinary `json::Json` tree, so anything that already produces
one — `json::parse` on a config file, `yaml::parse` on a YAML one, an HTTP body —
is a context. Because the model is `json::Json`, an importer writes `IMPORT json`
alongside `IMPORT mustache` to name the type. (Imports are not transitive, so
importing `mustache` alone does not put `json::` in scope.)

File I/O stays separate:

```mfb
LET template AS String = fs::readText("page.mustache")
LET context AS json::Json = json::parse(fs::readText("page.json"))
LET html AS String = mustache::render(template, context)
```

## API

| Function | Signature | Notes |
| --- | --- | --- |
| `render` | `(template AS String, context AS json::Json, maxBytes AS Integer = 16777216) AS String` | A `{{>name}}` renders as nothing. |
| `renderWith` | `(template AS String, context AS json::Json, partials AS Map OF String TO String, maxBytes AS Integer = 16777216) AS String` | A `{{>name}}` the map does not supply renders as nothing. |
| `escapeHtml` | `(text AS String) AS String` | The escaping a `{{name}}` tag applies, for HTML built around a fragment. |

`maxBytes` is the largest output allowed, in UTF-8 bytes, the same parameter and
default as `compress::inflate`. It must not be negative.

## Errors

Every failure is a registry code, so one `TRAP` handler serves the whole package
and no package-specific constant is needed:

| Code | When |
| --- | --- |
| `errorCode::ErrInvalidFormat` | A malformed template: an unclosed tag, a close with nothing open or naming the wrong section, a section left open, or a set-delimiters tag that does not name exactly two delimiters. |
| `errorCode::ErrDepthExceeded` | Sections nested past 64, or partials expanded past 16 — which is how a partial that reaches itself is reported. |
| `errorCode::ErrTooLarge` | A template or partial body past 1 048 576 bytes, or output past `maxBytes`. |
| `errorCode::ErrInvalidArgument` | A negative `maxBytes`. |

```mfb
LET html AS String = mustache::render(template, context, 65536) TRAP(problem)
  IF problem.code = errorCode::ErrTooLarge THEN
    io::printError("that template renders too much output")
    RECOVER ""
  END IF
  PROPAGATE
END TRAP
```

## What it implements

Interpolation (`{{name}}`, `{{{name}}}`, `{{&name}}`), sections (`{{#name}}`),
inverted sections (`{{^name}}`), comments (`{{! note}}`), partials (`{{>name}}`),
and set-delimiters (`{{=<% %>=}}`) — including the parts that are easy to miss:

| Construct | What it does |
| --- | --- |
| **Dotted names** | `{{a.b.c}}` walks into nested objects. Only the FIRST segment searches the context stack; the rest resolve strictly against what came before. |
| **The implicit iterator** | `{{.}}` is the current context, so `{{#tags}}{{.}} {{/tags}}` prints a list of strings, and `{{#rows}}{{#.}}{{.}}{{/.}}{{/rows}}` an array of arrays. |
| **Standalone lines** | A line holding nothing but whitespace and a section, comment, partial or set-delimiters tag disappears completely — tag, whitespace and newline. An interpolation is content, so a line carrying one is never standalone. |
| **Partial indentation** | A standalone `{{>body}}` reprints the partial under the whitespace that preceded it, applied to the partial's SOURCE before it is parsed — so the partial's lines are indented and the newlines inside an interpolated value are not. |
| **Delimiter scope** | A `{{=<% %>=}}` holds to the end of the template that wrote it. It does not reach into a partial, and a change inside a partial does not leak out. |

## Decisions

The specification leaves these open, or the reference implementation (mustache.js)
answers them differently. Each is a deliberate choice, pinned by a test and by the
oracle corpus.

| Decision | Behaviour and reason |
| --- | --- |
| **Lambdas are not supported** | The specification's one optional module. A lambda is a context value that is a function, and a `json::Json` tree holds no callables. A `{{#name}}` over a value that is not a lambda behaves exactly as the specification says, so a lambda-style template renders as an ordinary section. |
| **A dotted name resolves only against its parent** | The specification's `Dotted Names - Context Precedence`. With `{"a": {"b": {}}, "b": {"c": "ERROR"}}`, `{{#a}}{{b.c}}{{/a}}` renders nothing. **mustache.js renders `ERROR`** — the one required-module case it fails. |
| **Escaping is the reference's, not the minimum** | `{{name}}` escapes `&`, `<`, `>`, `"`, `'`, `/`, `` ` `` and `=`, as mustache.js does. That is three more than `encoding::htmlEscape`, and `&#39;` where that function writes `&apos;`. `/` closes a `<script>` block early inside a string literal, `` ` `` is an attribute delimiter in older Internet Explorer, and `=` starts an attribute in an unquoted value. |
| **Zero and the empty string are falsey** | So `{{#count}}` over `0` renders nothing, as in mustache.js and Handlebars. Ruby's implementation decides the other way. |
| **Interpolating an array or object writes compact JSON** | The specification is silent, and mustache.js writes `[object Object]`. JSON is deterministic, identical on every platform, loses nothing, and is written exactly as `json::stringify` writes it. |
| **A set-delimiters tag must name exactly two delimiters** | `{{=a b c=}}` fails with `errorCode::ErrInvalidFormat`. mustache.js silently keeps the first two, which changes what every following tag means. |
| **Whitespace means ASCII whitespace** | A no-break space inside a tag or on an otherwise-standalone line is content here, where JavaScript's `\s` counts it as whitespace. Every character Mustache gives meaning to is ASCII. |
| **A missing partial renders nothing** | As the specification requires — a template may name a partial that only some callers supply. |
| **A malformed template fails** | Every construct above that cannot be read is an error, not a silently different page. |
| **Partials come from a map, never from disk** | A template cannot reach a file the caller did not choose. A program that wants partials from a directory reads them itself, and uses `fs::isWithin` or `fs::openWithin` so a `../` cannot escape it. |

## Limits

An untrusted template cannot exhaust memory or the stack.

| Limit | Value | Set by | Failure |
| --- | --- | --- | --- |
| Template, and each partial body | 1 048 576 bytes | fixed | `ErrTooLarge` |
| Section nesting | 64 | fixed | `ErrDepthExceeded` |
| Partial expansion depth | 16 | fixed | `ErrDepthExceeded` |
| Rendered output | 16 777 216 bytes by default | `maxBytes` | `ErrTooLarge` |

The template and depth limits are fixed: a template is source the program wrote,
and the depth limit is a property of the native stack. The output limit is the
caller's, because the right size depends on the page.

The partial limit is the load-bearing one. A partial may name any partial,
including itself — the specification's own recursion test relies on it — so the
construct cannot be refused, only bounded. The output limit is checked as the
output grows, so a template that multiplies its output stops at the limit instead
of after exhausting memory.

## Performance

A context is flattened once, before rendering, into a table of values that the
renderer walks by index (see `src/context.mfb`). Pushing a section frame and
resolving a name then copy integers, never a subtree. Before this design a section
over a large list copied its whole root context for every item: a 20 000-item list
took 32 s to render. The same program (which also builds and parses its input)
now takes about 0.6 s.

Scaling is measured by `tools/mustache-bench` (see its README). Its result is not
yet linear at the largest sizes, and the cause is in the language runtime, not this
package: the arena allocator's large-block flush sweeps every parked small block,
and the flush is triggered by frees of large blocks alone. The flattening makes
many small allocations, so the sweep grows with the document. CHANGELOG.md records
the measurements.

## Layout

| File | What it holds |
| --- | --- |
| `src/lib.mfb` | The public API and its documentation: `render`, `renderWith`, `escapeHtml`. |
| `src/context.mfb` | The flattened context (`Doc`), name resolution, truthiness, and what a value interpolates as. |
| `src/parse.mfb` | The tokenizer and parser: template text to a flat node list, standalone-line stripping, section linking. |
| `src/render.mfb` | The walk: sections, inverted sections, partials, partial re-indentation, the output limit. |
| `src/escape.mfb` | The HTML escaping a `{{name}}` tag applies. |
| `src/core.mfb` | Limits, the node record, byte primitives. |
| `src/test_*.mfb` | The `TESTING` blocks. |
| `oracle/` | The Node differential oracle and the official specification runner. |
| `check-doc-examples.sh` | Compiles and runs every `EXAMPLE` in `src/lib.mfb`. |

## Testing

```sh
mfb build packages/mustache
mfb test  packages/mustache            # the package's own suite (92 cases)
./packages/mustache/check-doc-examples.sh   # every DOC example compiles and runs

npm install --prefix packages/mustache/oracle
packages/mustache/oracle/fetch-spec.sh
mkdir -p packages/mustache/oracle/probe/packages
cp packages/mustache/mustache.mfp packages/mustache/oracle/probe/packages/mustache.mfp
mfb build packages/mustache/oracle/probe
node packages/mustache/oracle/diff.mjs      # spec + corpus + fuzz + mutate
```

`mfb test` pins the behaviour above. The oracle is what checks it against a
specification and an implementation written by someone else: the official suite
has 136 required cases, all of which this package passes, and mustache.js fails
one of them (see [`oracle/README.md`](oracle/README.md)).
