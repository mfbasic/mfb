# mustache — Mustache v1.4 templates over `json::Json`

`mustache::render` turns a template and a `json::Json` value into text;
`mustache::renderWith` adds a `Map OF String TO String` of partials.

```mfb
IMPORT mustache
IMPORT json

LET html AS String = mustache::render(template, context)
```

The context is an ordinary `json::Json` tree, so anything that already produces
one — `json::parse` on a config file, `yaml::parse` on a YAML one, an HTTP body
— is a context. Because the model is `json::Json`, an importer writes
`IMPORT json` alongside `IMPORT mustache` to name the type. (Imports are not
transitive, so importing `mustache` alone does not put `json::` in scope.)

File I/O stays separate:

```mfb
LET template AS String = fs::readText("page.mustache")
LET context AS json::Json = json::parse(fs::readText("page.json"))
LET html AS String = mustache::render(template, context)
```

That is the same split `csv::parse`, `json::parse` and `yaml::parse` already use
— none of them owns filesystem access, and neither does this.

Full API and prose: `mfb pkg doc packages/mustache/mustache.mfp`, or
`mfb doc packages/mustache` for the internals too.

## Conformance

The package passes **all 136 cases of the six required modules** of the
[official Mustache specification](https://github.com/mustache/spec) v1.4 —
`comments`, `delimiters`, `interpolation`, `inverted`, `partials` and
`sections`:

```sh
npm install --prefix packages/mustache/oracle
packages/mustache/oracle/fetch-spec.sh
node packages/mustache/oracle/diff.mjs spec
```

The reference implementation, mustache.js 4.2.0, passes 135 of them. The one it
fails is `interpolation/Dotted Names - Context Precedence`, and this package
follows the specification rather than the reference there — see the
compatibility table below.

## What it implements

Interpolation (`{{name}}`, `{{{name}}}`, `{{&name}}`), sections (`{{#name}}`),
inverted sections (`{{^name}}`), comments (`{{! note}}`), partials
(`{{>name}}`), and set-delimiters (`{{=<% %>=}}`) — including the parts that are
easy to miss and impossible to work around:

| Construct | What it does |
| --- | --- |
| **Dotted names** | `{{a.b.c}}` walks into nested objects. Only the FIRST segment searches the context stack; the rest resolve strictly against what came before. |
| **The implicit iterator** | `{{.}}` is the current context, so `{{#tags}}{{.}} {{/tags}}` prints a list of strings, and `{{#rows}}{{#.}}{{.}}{{/.}}{{/rows}}` an array of arrays. |
| **Standalone lines** | A line holding nothing but whitespace and a section, comment, partial or set-delimiters tag disappears completely — tag, whitespace and newline. An interpolation is content, so a line carrying one is never standalone. |
| **Partial indentation** | A standalone `{{>body}}` reprints the partial under the whitespace that preceded it, applied to the partial's SOURCE before it is parsed — so the partial's lines are indented and the newlines inside an interpolated value are not. |
| **Delimiter scope** | A `{{=<% %>=}}` holds to the end of the template that wrote it. It does not reach into a partial, and a change inside a partial does not leak out. |

## Why this is a package and not a built-in

Rendering a template is not a language service. The compiler consumes JSON (`rg
-n "IMPORT json|func_json" src` finds the HTTP integration) and reads YAML and
CSV because programs exchange data in them; nothing in MFB needs to render a
Mustache template to compile, link or run. And Mustache is a *policy* as much as
a grammar: the specification leaves a dozen questions open — is `0` falsey, what
does interpolating an object write, which characters does `{{name}}` escape —
and every implementation answers them slightly differently. A package can state
those answers, test them against a real oracle, and change them on its own
schedule instead of on the release schedule of the language.

## The compatibility policy

Every one of these is a deliberate decision, and the oracle project pins each of
them against mustache.js.

| Decision | Behaviour |
| --- | --- |
| **Lambdas are not supported** | The specification's one optional module. A lambda is a context value that is a function, and a `json::Json` tree holds null, booleans, numbers, strings, arrays and objects — none of them callable. A `{{#name}}` over a value that is not a lambda behaves exactly as the specification says, so a template written for a lambda renders as an ordinary section rather than failing. |
| **A dotted name resolves only against its parent** | The specification's `Dotted Names - Context Precedence`. With `{"a": {"b": {}}, "b": {"c": "ERROR"}}`, `{{#a}}{{b.c}}{{/a}}` renders nothing: the enclosing `b` is not a fallback for the `b` inside `a`. **mustache.js renders `ERROR`** — this is the one required-module case the reference implementation fails. |
| **Escaping is the reference's, not the minimum** | `{{name}}` escapes `&`, `<`, `>`, `"`, `'`, `/`, `` ` `` and `=`. Three more than `encoding::htmlEscape` rewrites, and `&#39;` for an apostrophe where that function writes `&apos;`. `/` closes a `<script>` block early from inside a string literal, `` ` `` is an attribute delimiter in older Internet Explorer, and `=` starts an attribute in an unquoted attribute value. Available on its own as `mustache::escapeHtml`. |
| **Zero and the empty string are falsey** | So `{{#count}}` over `0` renders nothing, as it does in mustache.js and Handlebars. The specification is silent; Ruby's implementation decides the other way. |
| **Interpolating an array or object writes compact JSON** | Undefined in the specification, so each implementation falls back to its host language: mustache.js writes `[object Object]`. JSON is deterministic, identical on every platform, loses nothing, and is legible in the one place this comes up — a template printing a value the author thought was a scalar. |
| **A set-delimiters tag must name exactly two delimiters** | `{{=a b c=}}` fails with `errorCode::ErrInvalidFormat`. mustache.js splits with a limit of two and silently uses the first two; guessing which two were meant changes what every following tag in the template means. |
| **Whitespace means ASCII whitespace** | A no-break space inside a tag or on an otherwise-standalone line is content here, where JavaScript's `\s` counts it as whitespace. Every character Mustache gives meaning to is ASCII, and a byte-level scan is what makes the tokenizer exact. |
| **A missing partial renders nothing** | As the specification requires — a template may name a partial that only some callers supply. |
| **A malformed template fails** | An unclosed tag, a `{{/name}}` with nothing open or naming the wrong section, and a section left open at the end are all `errorCode::ErrInvalidFormat`. A template that silently drops a misspelled closing tag produces a page missing a section, which is much harder to find than an error. |

## Limits

An untrusted template cannot exhaust memory or the stack.

| Limit | Value | Failure |
| --- | --- | --- |
| Template (and each partial body) | 1 048 576 bytes | `mustache::limitCode()` (93120001) |
| Section nesting | 64 | `errorCode::ErrDepthExceeded` |
| Partial expansion depth | 16 | `errorCode::ErrDepthExceeded` |
| Rendered output | 16 777 216 bytes | `mustache::limitCode()` (93120001) |

The partial limit is the load-bearing one. A partial may name any partial,
including itself — the specification's own recursion test relies on it — so the
construct cannot be refused, only bounded. `{{>a}}` inside a partial called `a`
would otherwise expand forever, and a partial that includes itself inside a
section over a list multiplies at every level. The output limit is checked as
the output grows rather than at the end, so such a template stops at the limit
instead of after exhausting memory.

Partials are supplied as a `Map OF String TO String` and never loaded from disk,
so a template cannot reach a file the caller did not choose. A program that does
want partials from a directory reads them itself, and should use `fs::isWithin`
or `fs::openWithin` so that a `../` in a partial name cannot escape it.

## Layout

| File | What it holds |
| --- | --- |
| `src/lib.mfb` | The public API and its documentation: `render`, `renderWith`, `escapeHtml`, `limitCode`. |
| `src/parse.mfb` | The tokenizer and parser: template text to a flat node list, standalone-line stripping, section linking. |
| `src/render.mfb` | The walk: sections, inverted sections, partials, partial re-indentation, the output limit. |
| `src/context.mfb` | Name resolution against the context stack, truthiness, and what a value interpolates as. |
| `src/escape.mfb` | The HTML escaping a `{{name}}` tag applies. |
| `src/core.mfb` | Limits, error codes, the node record, byte primitives. |
| `src/test_*.mfb` | The `TESTING` blocks. |
| `oracle/` | The Node differential oracle and the official specification runner. |

## Testing

```sh
mfb build packages/mustache
mfb test  packages/mustache            # the package's own suite (81 cases)
./packages/mustache/check-doc-examples.sh   # every DOC example compiles and runs

npm install --prefix packages/mustache/oracle
packages/mustache/oracle/fetch-spec.sh
mkdir -p packages/mustache/oracle/probe/packages
cp packages/mustache/mustache.mfp packages/mustache/oracle/probe/packages/mustache.mfp
mfb build packages/mustache/oracle/probe
node packages/mustache/oracle/diff.mjs      # spec + corpus + fuzz + mutate
```

`mfb test packages/mustache` pins the behaviour above. Those tests cannot catch
a misreading of the specification that the tests share — that is what
`oracle/` is for: it runs the official suite, which was written by someone else,
and compares against an implementation that was written by someone else. See
[`oracle/README.md`](oracle/README.md).
