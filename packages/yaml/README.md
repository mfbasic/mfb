# yaml — a JSON-compatible YAML 1.2 reader

`yaml::parse` turns one YAML document into a `json::Json` value; `yaml::parseAll`
turns a multi-document stream into a `List OF json::Json`.

```mfb
IMPORT yaml
IMPORT json

LET value AS json::Json = yaml::parse(text)
```

The result is an ordinary `json::Json` tree, so `json::get`, `json::getOr` and
`json::stringify` all work on it and anything that already consumes JSON — an
HTTP body, a config reader — consumes YAML unchanged. Because the model is
`json::Json`, an importer writes `IMPORT json` alongside `IMPORT yaml` to name
the type. (Imports are not transitive, so importing `yaml` alone does not put
`json::` in scope.)

File I/O stays separate:

```mfb
LET text AS String = fs::readText(path)
LET value AS json::Json = yaml::parse(text)
```

That is the same split `csv::parse` and `json::parse` already use — neither owns
filesystem access, and neither does this.

Full API and prose: `mfb pkg doc packages/yaml/yaml.mfp`, or `mfb doc
packages/yaml` for the internals too.

## Why this is a package and not a built-in

JSON has a crisp six-variant data model, and the compiler already consumes it —
`rg -n "IMPORT json|func_json" src` finds the HTTP integration. CSV has a
deliberately narrow, documented representation: `List OF List OF String`. Full
YAML is not crisp in that way. Tags, anchors, aliases, cyclic graphs, merge keys,
multiple documents, non-string mapping keys, duplicate keys, and
schema-dependent scalar typing cannot all map faithfully onto `json::Json`, so
reading YAML means adopting a **compatibility policy** — a set of decisions about
what to refuse. A package can evolve that policy on its own schedule instead of
tying it to every MFB release.

## What it reads

Block mappings and block sequences; flow mappings and flow sequences (`{a: 1}`,
`[1, 2]`), which makes **every JSON document a valid input**; plain,
single-quoted and double-quoted scalars, including multi-line ones; literal (`|`)
and folded (`>`) block scalars with indentation and chomping indicators;
comments; `%YAML 1.2`; the `---` and `...` document markers; and anchors and
aliases.

## The compatibility policy

Every one of these is a deliberate decision. Each fails with an `Error`; none
silently changes the data.

| Decision | Behaviour | Code |
| --- | --- | --- |
| **Mapping keys must be strings** | A plain key the schema types as anything else — `12:`, `true:`, `null:`, an empty key — is rejected. Quote it (`"12":`) to make it a string. | `errorCode::ErrUnsupported` |
| **Duplicate keys are rejected** | Not resolved last-wins. A duplicate key in a config file is a mistake worth hearing about. | `errorCode::ErrAlreadyExists` |
| **Multiple documents require `parseAll`** | `parse` reads a stream holding exactly one document, and fails on zero or on two. | `errorCode::ErrInvalidFormat` |
| **Tags and merge keys are rejected** | Every `!` and `!!` tag, the `%TAG` directive, and `<<`. A tag selects a type outside the schema and a merge key builds a mapping out of other mappings — both change what the document means. | `errorCode::ErrUnsupported` |
| **Aliases are expanded; cyclic aliases are rejected** | An anchor is registered only once its own node is complete, so `&a [*a]` finds no anchor. A cycle has no `json::Json` representation and `json::stringify` could not terminate on one. | `errorCode::ErrNotFound` |
| **Scalars use the YAML 1.2 Core Schema, named explicitly** | `true`/`false` are booleans; `null`, `~` and an empty value are null; `[-+]?[0-9]+`, `0x…`, `0o…` are integers; the decimal and exponent forms are floats; everything else is a string. | — |
| **Values with no JSON equivalent are rejected, not converted** | `.inf`, `-.inf` and `.nan` are numbers YAML can write and JSON cannot, so they fail instead of becoming a string, a null, or a zero. | `errorCode::ErrUnsupported` |
| **Depth, alias-expansion and total-node limits** | 100 levels of nesting, 10 000 alias expansions, 1 000 000 nodes. | `errorCode::ErrDepthExceeded`, `yaml::expansionLimitCode()` |

### The 1.1 trap, on purpose

The single most common YAML surprise is `yes`/`no`/`on`/`off` silently becoming
booleans. That is the **YAML 1.1** schema. This reader implements **1.2 Core**,
where they are strings — and it refuses `%YAML 1.1` outright rather than reading
a 1.1 document under 1.2 rules. The 1.1-only forms `0b101`, bare-octal `012`,
`1_000` and the sexagesimal `12:30` are likewise plain strings here.

### Expansion attacks

A YAML alias may name a node of any size, so a few hundred bytes can name an
exponentially large tree ("billion laughs"): ten anchors, each a ten-element
sequence of aliases to the previous one, is 10¹⁰ nodes. The node budget counts
what an alias **expands to**, not the one line that writes it, which is what
bounds that document. Blowing the alias or node budget raises the package's own
code `93110001`, which `yaml::expansionLimitCode()` returns — a generator-9
code, because no `errorCode::` value models "this input expands too far".
Generator-9 codes are not globally unique, so match it only around a call you
already know is this package's:

```mfb
LET value AS json::Json = yaml::parse(text) TRAP(problem)
  IF problem.code = yaml::expansionLimitCode() THEN
    io::printError("that YAML expands too far to read")
    RECOVER json::JsonObj[Map OF String TO json::Json {}]
  END IF
  PROPAGATE
END TRAP
```

(It is a function rather than an `EXPORT LET` constant because an exported
package constant does not currently resolve for an importer — `cst::Answer`
types as `Unknown` and dies with `TYPE_UNKNOWN_VALUE`, even though
`mfb spec language modules-and-packages` says a constant is named like any other
imported symbol. Filed separately; the accessor works today and would keep
working if that were fixed.)

## Errors

| Code | Name | When |
| --- | --- | --- |
| `77050003` | `errorCode::ErrInvalidFormat` | Malformed YAML; or `parse` got zero or several documents |
| `77050007` | `errorCode::ErrUnsupported` | A construct outside the subset (above) |
| `77050005` | `errorCode::ErrAlreadyExists` | Duplicate mapping key |
| `77050004` | `errorCode::ErrNotFound` | An alias naming no anchor — including a cyclic one |
| `77050024` | `errorCode::ErrDepthExceeded` | More than 100 levels of nesting |
| `77050025` | `errorCode::ErrInvalidSurrogate` | A `\u` escape encoding an unpaired surrogate |
| `77050010` | `errorCode::ErrOverflow` | A number too large for `Float` |
| `93110001` | `yaml::expansionLimitCode()` | Past the alias-expansion or total-node budget |

## Building and using it

```sh
mfb build packages/yaml            # writes packages/yaml/yaml.mfp
mfb test  packages/yaml            # runs the TESTING blocks
mfb doc   packages/yaml --out yaml.html
```

An importing project names the built package by path:

```json
"packages": [
  { "name": "yaml", "version": "=0.1.0", "source": "file:packages/yaml.mfp" }
]
```

`examples/yaml-json` is a worked consumer — a command-line converter that reads
YAML into `json::Json` and writes it back out again.

## How it was checked

`mfb test packages/yaml` runs 89 cases pinning the behaviour above. Those pin what
this project *decided*; they cannot catch a misreading of the YAML spec that the
tests share. Two independent implementations do that, and between them they found
nine real defects the tests did not.

**[`oracle/`](oracle) — the primary check.** A small Node project that reads the
same documents with [`yaml`](https://www.npmjs.com/package/yaml) (eemeli/yaml)
pinned to **YAML 1.2 Core**, the same language this package documents, over a
hand-written corpus, the oracle's own writer output, and byte-level mutations:

```sh
npm install --prefix packages/yaml/oracle
node packages/yaml/oracle/diff.mjs
```

Because it is pinned to 1.2, it agrees with this package on everything except the
constructs the package deliberately refuses — so a divergence is nearly always a
finding. It caught seven, every one of them the same shape: *accepting invalid
YAML and silently changing what the document means*. `a: b: c` read as a nested
mapping; `!t: 2` absorbing a tag into a key; `a: %x` accepted as a plain scalar;
a flow collection swallowing the keys below it; a comment failing to close a plain
scalar; `%YAML 1.2#` read as 1.2; and a lone carriage return treated as text
rather than as the line break YAML 1.2 says it is. `oracle/README.md` has the
table.

**`scripts/yaml_oracle_diff.py` — the second opinion.** The same idea against
**PyYAML**, which is YAML **1.1**. Its disagreements are mostly spec-version noise
(listed and checked in that script's `EXPECTED` table), but having a second,
differently-wrong reference is what made the carriage-return question decidable
rather than a coin toss — the two oracles disagree with *each other* there, and
the spec settles it. It found two bugs of its own: a literal block scalar dropping
the blank lines inside it, and the example emitter writing a root-level block
scalar with no indentation.

**`examples/yaml-json/smoke.sh`** covers the one thing neither reaches: the
example's own command-line handling.

## How it is put together

| File | What it holds |
| --- | --- |
| `src/core.mfb` | Limits, the threaded parser `State`, the shared `Node` record, byte primitives, and the line table |
| `src/scalar.mfb` | Quoted-scalar decoding, line folding, and the YAML 1.2 Core Schema |
| `src/flow.mfb` | `[a, b]` and `{a: b}` — the byte-offset, indentation-free half |
| `src/block.mfb` | Block mappings, block sequences, block scalars, multi-line plain scalars |
| `src/document.mfb` | Directives, `---`/`...`, and the per-document driver |
| `src/lib.mfb` | `parse`, `parseAll`, and the package documentation |
| `src/test_*.mfb` | `TESTING` blocks — dropped before codegen in an ordinary build |
| `oracle/` | The Node differential oracle (its own README) |

Two shapes are worth knowing before changing anything:

* **The reader works on UTF-8 bytes, not scalars or graphemes.** Every YAML
  indicator and every character that can indent is ASCII, so a byte compare is
  exact and can never split a multi-byte scalar; a byte ≥ 128 only ever occurs
  inside scalar content, copied through verbatim.
* **Parser state is threaded, not global.** The anchor table and the two
  expansion counters travel through every parse function in a `State` record, so
  a parse is re-entrant and two threads may parse at once. Each function rebuilds
  `State` once on the way out rather than once per node, because rebuilding it
  copies the anchor table.

Document splitting runs *before* any node is parsed, so every scanner below it is
handed a line range and a byte range it may not leave. That is what stops an
unterminated quoted scalar or flow collection in one document from swallowing the
next one.
