# oracle — differential-testing `packages/json_schema`

The package's own tests (`mfb test packages/json_schema`) pin the behaviour this
project decided on. They cannot catch a *misreading* of the specification,
because the same misreading is in both the code and the test. This directory is
the other half: the same questions, asked of implementations that were written
by other people from the same specification.

```sh
npm install          # once
./run.sh             # build everything, then every mode
```

Exit status is 0 iff every disagreement is one of the known ones listed in
`oracle.mjs`, each with a written reason.

## The three oracles

| Oracle | What it settles |
| --- | --- |
| [ajv](https://ajv.js.org) 8 (`ajv/dist/2020`) | Whether an instance is valid, on a large random sample. ajv is the most-used JSON Schema validator there is; where the two disagree, one of them is wrong. |
| Node's own `RegExp`, with the `u` flag | Whether `pattern` means in this package what it means in ECMA-262. This is the seam with the most room for silent divergence, so it is tested directly rather than through a schema. |
| The official [JSON-Schema-Test-Suite](https://github.com/json-schema-org/JSON-Schema-Test-Suite) | Which one is wrong. Every case states whether the instance is valid, so this is ground truth rather than a second opinion — and it is the only mode that can find a bug the other two share. |

Get the suite with `./fetch-suite.sh`; without it, `suite` mode skips itself and
says so.

## The modes

```sh
node oracle.mjs                        # all four
node oracle.mjs corpus                 # one
node oracle.mjs fuzz --seed 7          # reproducible random testing
node oracle.mjs suite --suite <dir>    # a checkout somewhere else
```

* **corpus** — hand-written schemas in `corpus.mjs` covering the whole supported
  subset, read by both implementations. Ordinary schemas, deliberately: an
  everyday shape is where a silent disagreement costs the most.
* **pattern** — every `pattern` in the corpus plus a generated set, each tried
  against 47 subjects (ASCII, Greek, Arabic-Indic digits, astral characters,
  every ECMA-262 whitespace character) and compared with `new RegExp(p, "u")`.
* **fuzz** — random schemas built from the supported subset only, so a compile
  failure is a finding rather than an expected refusal, with random instances.
  Seeded, so a failure reproduces.
* **suite** — the official suite, compared against its own `valid` flags.

Two environment variables help when a run does fail: `ORACLE_FULL=1` prints
whole schemas instead of truncating them, and `ORACLE_MAX_FAILURES=N` raises the
40-failure print cap. `ORACLE_NO_EXPECTED=1` drops the suite's file-level
allowances, which is how that list is kept from hiding a real wrong verdict.

## How it talks to the package

`driver/` is an ordinary MFB executable (`jsvalidate`) that imports the package
and answers a whole job file at once:

```json
{"cases":    [{"id": "c1", "schema": {...}, "instance": ...}],
 "patterns": [{"id": "p1", "pattern": "\\d+", "text": "12"}]}
```

and writes one JSON document back, in the same order. A process per question
would cost more than answering it, and a fuzzing run asks tens of thousands.
The driver is also a usable command-line validator in its own right:

```sh
driver/build/jsvalidate.out schema.json instance.json
driver/build/jsvalidate.out --translate '\d+'      # -> [0-9]+
driver/build/jsvalidate.out --match '^a' 'abc'     # -> true
```

Compiling and validating are attempted separately so that a refused *schema* and
a rejected *instance* stay distinguishable — the distinction the oracle cares
about most, because the two implementations are allowed to differ on the first
and not on the second.

## Results, and what they mean

At the time of writing, on this package:

| Mode | Compared | Agreed | Known divergences | Unexplained |
| --- | --- | --- | --- | --- |
| corpus | 370 | 356 | 14 | 0 |
| pattern | 5934 | 5564 | 370 | 0 |
| fuzz (seeds 1–6, 400 schemas each) | 14400 | 14388 | 12 | 0 |
| suite | 1302 | 1297 | 5 | 0 |

The suite line is the one to read: **1297 of 1302 official Draft 2020-12 cases
agree with their stated verdicts, and the five that do not are the
`vocabulary.json` cases, which name a custom dialect.** This package refuses every
dialect but Draft 2020-12, by design, so those five are refusals rather than wrong
verdicts. The suite mode registers the suite's own `remotes/` documents with the
package's registry, as the suite's runner does, so the cases that refer to them
are checked too.

Reproduce the table with `./run.sh` and `node oracle.mjs fuzz --seed N`.

## What the oracle found

Running it is not a formality; it found real bugs on both sides.

In this package, while preparing 1.0:

* A `$ref` that names a location inside a schema resource, without entering that
  resource's root, left the resource out of the dynamic scope, so a
  `$dynamicRef` inside it was redirected to the wrong anchor. The official
  `dynamicRef.json` case "avoids the root of each schema" found it. Every
  reference now enters the resource that contains its target.
* The official `format-assertion` remote documents refer to the 2020-12
  metaschemas, which the package had not bundled. They are built in now. The
  format-assertion vocabulary itself is not, and its two remote documents are
  left out of the oracle's registry, because an eagerly checked registry refuses
  every case over them.

In this package, before it shipped:

* `type: "integer"` raised an arithmetic overflow on `1e21`. `isIntegral` asked
  `math::floor`, which returns an **Integer**, so an ordinary JSON number above
  2^63 could not be classified at all. The remainder form has no such range.
* A root `$id` was registered twice — once by `compile` and once by the walk —
  so every document with a root `$id` failed as a duplicate identifier.
* The evaluation budget was a fixed 1,000,000 subschema evaluations, which is
  simultaneously too small for a large document and too large for a one-byte
  one. It is now 10,000 plus 100 per instance value, so the work is bounded by
  what the caller handed in.

In ajv, each proven against the specification or the official suite rather than
asserted:

* A root-level `$dynamicRef` whose `$dynamicAnchor` sits in `$defs` overflows
  ajv's stack. The static target is that subschema, and the official suite's
  `dynamicRef.json` cases agree with this package.

* `multipleOf` is tested with `division !== parseInt(division)`, and `parseInt`
  reads a quotient in exponent notation as its leading digits. `parseInt(5e299)`
  is `5`, so ajv calls `1e300` not a multiple of `2` when exact arithmetic says
  it is.
* A passing `contains` is treated as having evaluated **every** item. The
  specification annotates only the matching ones, and the official suite agrees:
  `[1, 2, "foo"]` under `{prefixItems: [true], contains: {type: "string"},
  unevaluatedItems: false}` is invalid, and ajv calls it valid.
* `contains` is skipped entirely on an **empty** array when a sibling
  `prefixItems` holds a `false` schema: `{prefixItems: [false], contains: false}`
  is called valid for `[]`, while `{contains: false}` alone is correctly called
  invalid.
* `{enum: []}` is refused at compile time, though the 2020-12 metaschema types
  `enum` as `{type: "array", items: true}` with no minimum length.

## The divergences that are decisions, not bugs

`\b` and `\B`. ECMA-262 defines a word character as `[A-Za-z0-9_]`; the `regex`
package's word test is Unicode-aware, and a word boundary cannot be written
without lookaround. The two agree on any text whose word characters are ASCII
and differ where they are not. Everything else in the pattern grammar is either
translated exactly or refused — see `src/ecma.mfb`.

`multipleOf` on decimals. The package reads each number as its shortest decimal
and divides exactly, so `0.3` is a multiple of `0.1`. ajv divides in binary64 and
says it is not. The package's own tests pin the decimal reading, and the corpus
records the disagreement as `decimal-multipleof`.
