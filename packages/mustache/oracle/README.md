# oracle — check `packages/mustache` against the specification, and against the reference

Two independent checks in one small Node project.

The first is **conformance**: the official
[Mustache specification](https://github.com/mustache/spec) is a set of JSON
files of `{template, data, partials, expected}` cases, and running them is the
only way to know the package implements Mustache rather than something that
looks like it.

The second is **differential**: mustache.js is the reference implementation, and
the specification is minimal — it says nothing about a stray `{{`, a partial
with no trailing newline, an unmatched delimiter, or a name that is only
whitespace. Comparing renders on generated and mutated templates covers that
undocumented remainder, which is most of what a real template exercises.

```sh
npm install --prefix packages/mustache/oracle
packages/mustache/oracle/fetch-spec.sh

mfb build packages/mustache
mkdir -p packages/mustache/oracle/probe/packages
cp packages/mustache/mustache.mfp packages/mustache/oracle/probe/packages/mustache.mfp
mfb build packages/mustache/oracle/probe

node packages/mustache/oracle/diff.mjs                    # every mode
node packages/mustache/oracle/diff.mjs spec               # one mode
node packages/mustache/oracle/diff.mjs fuzz --count 5000
```

Exit status is 0 iff every specification case passed and every comparison
agreed, or diverged for a reason declared in `divergences.json`.

## The specification is not vendored

`fetch-spec.sh` downloads it. Pinning a copy would quietly turn "the
specification says" into "the specification said, once", and the whole value of
an external suite is that it is external. `spec/` is git-ignored.

Only the six **required** modules are fetched. `~lambdas` is the specification's
one optional module and this package does not implement it — a `json::Json`
value cannot hold a callable — so running those cases would report failures for
a feature documented as absent. `~dynamic-names` and `~inheritance` are likewise
optional and out of scope.

## Why mustache.js, and why it is not the authority

mustache.js *is* the reference implementation: the specification's cases were
written alongside it, and where the specification is silent, what mustache.js
does is what template authors have learned to expect. That makes it the right
thing to compare against — and the wrong thing to defer to.

It fails one of the 136 required cases,
`interpolation/Dotted Names - Context Precedence`. `diff.mjs spec` reports that
count on every run, so the difference stays a fact on the screen rather than a
belief. On that one construct the package follows the specification and
deliberately differs from the reference; `divergences.json` says so.

## How the two sides talk

`probe/` is a tiny MFBASIC executable — the package's only consumer here. It
reads one case file and prints one line of JSON:

```
{"ok":true,"output":"..."}
{"ok":false,"code":77050003,"message":"mustache: a tag opened at byte 4 is never closed"}
```

`oracle.mjs` prints the same shape:

```
{"ok":true,"output":"..."}
{"ok":false,"kind":"render","reason":"Unclosed tag at 8"}
{"ok":false,"kind":"oracle-defect","reason":"Cannot read properties of null (reading 'x')"}
```

A **refusal is a result**, reported on stdout with exit 0. That leaves a
non-zero exit or an unparseable line meaning exactly one thing — the probe
itself broke — which is what the `mutate` mode checks for.

Two refusals count as agreement without comparing the reasons: the two
implementations have their own vocabularies for "no", and demanding the same
words would make the harness fail on wording. What matters is that neither one
silently rendered a template the other refused.

An `oracle-defect` is neither. mustache.js 4.2.0 **crashes** on
`{{#a}}{{x}}{{/a}}` over `{"a":[null]}`: a section over a list pushes each
element as a context frame, and the plain-name branch of `Context.lookup`
indexes that frame with no null guard, so a null element throws a `TypeError`.
(The dotted-name branch does guard it, which is why `{{x.y}}` on the same data
is fine.) The `fuzz` mode found that; it is a defect in the oracle rather than
an answer from it, so those cases are counted and reported separately — never as
a package failure, and never silently, because a growing count would mean the
oracle had stopped being useful. `corpus/null-list-element.json` and the
package's own tests pin what the package does with the same input.

## The four modes

| Mode | What it does | Asserts |
| --- | --- | --- |
| `spec` | The official suite, 136 cases across the six required modules. | Each case's own `expected` output. Also reports how many of them mustache.js fails. |
| `corpus` | The hand-written cases in `corpus/`, each a whole realistic template rather than a snippet, so a failure names a *construct*. | Agreement with mustache.js, or a declared divergence |
| `fuzz` | Generated contexts with generated templates that reference them — sections, inverted sections, partials, comments, delimiter changes and standalone lines, nested. | Exact agreement |
| `mutate` | Corpus templates with random byte-level damage. | **Robustness**: whatever the damage, the renderer answers with a well-formed envelope — never a crash, a hang, or unparseable output. Agreement is reported as information. |

`corpus/divergent/` holds the cases that must *not* agree, and
`divergences.json` says why for each. A declared case that stops diverging fails
too — that is how the harness notices the package quietly changing a documented
policy.

### What the generator deliberately avoids

Every key in a generated context is unique (`n0`, `n1`, …), so no name in one
frame can shadow a name in another. That keeps the fuzzer away from the one
construct where the package and mustache.js disagree on purpose — a dotted name
whose first segment resolves in the inner frame while its remainder only
resolves in an outer one. That is a stated decision, pinned in
`corpus/divergent/`, and a fuzzer rediscovering it a few hundred times a run
would bury everything else. Interpolations are only ever generated for scalars
and for names that do not exist, for the same reason: interpolating a collection
is undefined and the two implementations answer differently by design.

## What it found

| Finding | Where |
| --- | --- |
| `json::stringify` emitted a raw CR LF pair inside a JSON string, producing a document no parser accepts — CR LF is one extended grapheme cluster, and the escaper walked graphemes. A **built-in package** bug, hit the moment the probe reported a template with Windows line endings. | `src/codegen/builtins/json/helper_escape_string.rs` |
| A `{{>p}}` after an emoji was indented one column short of the reference, which counts UTF-16 code units. | `packages/mustache/src/core.mfb` (`indentColumns`) |
| mustache.js 4.2.0 crashes on a null element of a list. | reported above; not ours to fix |

## Layout

| Path | What it is |
| --- | --- |
| `oracle.mjs` | The oracle: a case → the JSON envelope. Also importable (`renderCase`). |
| `diff.mjs` | The runner: the four modes, and the comparison. |
| `fetch-spec.sh` | Downloads the official suite into `spec/`. |
| `divergences.json` | Corpus file → why it must diverge. |
| `corpus/` | Hand-written cases. `corpus/divergent/` are the declared ones. |
| `probe/` | The MFBASIC side: `mustache::renderWith` → the same envelope. |

`node_modules/`, `spec/`, `probe/packages/mustache.mfp` and `probe/build/` are
git-ignored; `package-lock.json` is tracked so `npm install` is reproducible.
