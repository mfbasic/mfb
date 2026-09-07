# yamljson — convert YAML to JSON and JSON to YAML

A command-line converter built on the `yaml` package (`packages/yaml`). It is the
worked example of that package: one direction is `yaml::parse` doing all the
work, and the other is a small YAML **emitter** that lives here, in the
application, rather than in the package.

```
yamljson to-json <file> [--compact]   read YAML, write JSON
yamljson to-yaml <file>               read JSON, write YAML
yamljson <file> [--compact]           pick the direction from the extension
```

`.yaml` / `.yml` → JSON, `.json` → YAML. JSON comes out indented by two spaces
unless `--compact` is given.

## Building and running

The package must be built to a `.mfp` and installed where this project's manifest
expects it (`packages/yaml.mfp`). From the repository root:

```sh
mkdir -p examples/yaml-json/packages
mfb build packages/yaml
cp packages/yaml/yaml.mfp examples/yaml-json/packages/yaml.mfp
mfb build examples/yaml-json

./examples/yaml-json/build/yamljson.out to-json examples/yaml-json/samples/config.yaml
./examples/yaml-json/build/yamljson.out to-yaml examples/yaml-json/samples/package.json
```

The installed `.mfp` and the build output are both git-ignored, so re-run those
steps whenever the package changes.

`examples/yaml-json/smoke.sh` builds both projects and then checks every
command-line path — both directions, all three extensions, the multi-document
case, and each of the argument and failure exits:

```sh
examples/yaml-json/smoke.sh          # or: smoke.sh path/to/mfb
```

It exists because argument handling is the part no other test covers: this
program first shipped comparing `fs::pathExtension` against `"yaml"` when that
call returns `".yaml"`, so every `yamljson <file>` invocation failed and nothing
noticed.

## The two samples

`samples/config.yaml` exercises most of the supported subset — flow and block
collections, anchors and aliases, literal and folded block scalars, comments, a
`%YAML 1.2` directive — and two of the reader's deliberate decisions:

```yaml
featureFlags:
  darkMode: yes      # a STRING under the 1.2 Core Schema, not true
  betaBanner: off    # likewise
startsAt: 12:30      # a STRING, not the 1.1 sexagesimal 750
```

A YAML 1.1 reader gives `true`, `false` and `750` for those three. That is the
single most common YAML surprise, and it is why the package names its schema
explicitly.

`samples/package.json` goes the other way and shows what the emitter does with
nested arrays, empty collections, and a string containing a line break.

## Two things worth reading the source for

**Why the emitter is here and not in the package** (`src/emit.mfb`). The
package's job is *reading*: it turns YAML into the JSON model and stops. Writing
is a separate set of decisions — how to quote, when to reach for a block scalar,
how to indent — and an application is the right place to make them. The emitter
holds itself to one rule: whatever it writes must read back as the same value, so
anything it is not certain a plain scalar would round-trip gets quoted.

It quotes more than it strictly must. `yes`, `no`, `on`, `off`, `y` and `n` are
strings to *this* reader, but a YAML 1.1 reader would take an unquoted one as a
boolean — that is a file two readers disagree about, and two quote characters
removes the disagreement.

**Why `to-json` can emit an array** (`src/main.mfb`). YAML is a stream of
documents and JSON is one value. A stream holding several documents becomes a
JSON array of them rather than quietly dropping all but the first:

```sh
$ printf -- '--- one\n--- two\n' > /tmp/two.yaml
$ ./examples/yaml-json/build/yamljson.out to-json /tmp/two.yaml --compact
["one","two"]
```

## Errors

Everything the reader refuses arrives as an ordinary `Error`, which `main`'s
function-level `TRAP` prints with its code and origin before exiting 1:

```sh
$ printf 'a: 1\na: 2\n' > /tmp/dup.yaml
$ ./examples/yaml-json/build/yamljson.out to-json /tmp/dup.yaml
yamljson: yaml: duplicate mapping key `a`
  [77050005] src/block.mfb:477
```

(The origin is where inside the reader the failure was raised — `Error.source` is
stamped at the origin and never rewritten as the error propagates.)

`mfb man errors` explains the model; `packages/yaml/README.md` lists every code
this converter can surface and why.
