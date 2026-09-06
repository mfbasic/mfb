# oracle — differential-test `packages/yaml` against an independent YAML 1.2 reader

A small Node project whose only job is to read the same YAML the package reads
and disagree loudly. The package's own `TESTING` blocks pin what this project
*decided*; they cannot catch a misreading of the YAML spec that the tests share.
This can.

```sh
npm install --prefix packages/yaml/oracle

mfb build packages/yaml
mkdir -p packages/yaml/oracle/probe/packages
cp packages/yaml/yaml.mfp packages/yaml/oracle/probe/packages/yaml.mfp
mfb build packages/yaml/oracle/probe

node packages/yaml/oracle/diff.mjs                    # every mode
node packages/yaml/oracle/diff.mjs corpus             # one mode
node packages/yaml/oracle/diff.mjs fuzz --count 5000
```

Exit status is 0 iff every case agreed, or diverged for a reason declared in
`divergences.json`.

## Why this module, and why Node

The oracle is [`yaml`](https://www.npmjs.com/package/yaml) (eemeli/yaml). It
implements YAML **1.2** with a selectable schema, so `oracle.mjs` can pin it to
the same language `packages/yaml` documents — the **1.2 Core Schema**.

That matters more than it sounds. The other easily available oracle, PyYAML, is
YAML **1.1**, where `yes` is a boolean, `12:30` is 750, and `0o17` and `1e3` are
strings. Every one of those is a difference in the *spec version* rather than in
either implementation, so a 1.1 oracle buries the real signal under a long list
of expected-to-differ inputs. Pinned to 1.2 Core, this oracle agrees with
`packages/yaml` on everything except the handful of constructs the package
deliberately refuses — so a divergence is nearly always a finding.

(`scripts/yaml_oracle_diff.py` keeps PyYAML around as a *second*, differently-
wrong opinion, which is worth having: the two references do not agree with each
other either, and where they disagree is exactly where the spec is worth
re-reading.)

Every option in `oracle.mjs`'s `OPTIONS` is load-bearing and commented there;
`mapAsMap: true` is the least obvious one — it makes mapping keys arrive as real
values instead of being stringified into an object, which is what lets the
oracle *see* a non-string key (`12: x`) and refuse it rather than silently agree
on `{"12":"x"}` for a document the package rejects.

## How the two sides talk

`probe/` is a tiny MFBASIC executable — the package's only consumer here. It
reads one file with `yaml::parseAll` and prints one line of JSON:

```
{"ok":true,"documents":[ ... ]}
{"ok":false,"code":77050005,"message":"yaml: duplicate mapping key `a`"}
```

`oracle.mjs` prints the same shape:

```
{"ok":true,"documents":[ ... ]}
{"ok":false,"kind":"parse","reason":"DUPLICATE_KEY: Map keys must be unique"}
{"ok":false,"kind":"unrepresentable","reason":"the number Infinity has no JSON form (.inf/.nan)"}
```

A **refusal is a result**, reported on stdout with exit 0. That leaves a non-zero
exit or an unparseable line meaning exactly one thing — the probe itself broke —
which is what the `mutate` mode checks for. And because both sides speak JSON,
the comparison is on values, never on formatting.

Two refusals count as agreement without comparing the reasons: the two readers
have their own vocabularies for "no", and demanding the same words would make the
harness fail on wording. What matters is that neither one silently produced a
value the other refused.

## The three modes

| Mode | What it does | Asserts |
| --- | --- | --- |
| `corpus` | The hand-written documents in `corpus/`, each a whole realistic document rather than a snippet, so a failure names a *construct*. | Agreement, or a declared divergence |
| `fuzz` | Random values serialized to YAML **by the oracle's own writer**, cycling through eight styles (block/flow, indent widths, line widths, quote preferences). The writer only emits YAML its own reader accepts, so any disagreement is ours. | Exact agreement |
| `mutate` | Corpus documents with random byte-level damage. | **Robustness**: whatever the damage, the reader answers with a well-formed envelope — never a crash, a hang, or unparseable output. Agreement is reported as information. |

`corpus/divergent/` holds the documents that must *not* agree; `divergences.json`
says why for each. A declared case that stops diverging fails too — that is how
the harness notices the package quietly changing a documented policy.

## What it found

Every one of these was a real defect, none of them caught by the package's own
89 tests, and all of them in the same class: **accepting invalid YAML and
silently changing what the document means**.

| Finding | Was read as | Should be |
| --- | --- | --- |
| `a: b: c` | a nested mapping `{a: {b: c}}` | rejected — YAML reaches a block collection only through a line break |
| `x: 1` / `!t: 2` | the key `"!t"`, tag silently absorbed | rejected — only the *first* key of a mapping went through the node dispatcher |
| `a: %x`, `a: @x`, `` a: `x ``, `a: ,x` | plain scalars | rejected — YAML 1.2 §7.3.3 forbids every `c-indicator` as a plain scalar's first character |
| `a: [1,` / `b: 2,` / `c]` | `a` holding `[1, {b: 2}, "c"]` | rejected — a flow collection in a block collection must stay indented past it |
| `a: 1 # c` / `  more` | the scalar `"1 more"` | rejected — a comment closes a plain scalar for good |
| `%YAML 1.2#` | version `1.2`, accepted | rejected — a `#` not preceded by whitespace is not a comment |
| `a: 1<CR>b: 2` | one line, key `"a: 1<CR>b"` | two lines — a lone CR **is** a line break (`b-break`) |

The last one is worth the detour: the package's code carried a comment asserting
that YAML 1.2 dropped the lone carriage return. It does not — `b-break ::= (
b-carriage-return b-line-feed ) | b-carriage-return | b-line-feed` — and PyYAML
agrees with the spec while this oracle refuses such a file. Both references
being available is what made the disagreement legible instead of a coin toss.

## Layout

| Path | What it is |
| --- | --- |
| `oracle.mjs` | The oracle: YAML text → the JSON envelope. Also importable (`readYaml`, `OPTIONS`). |
| `diff.mjs` | The runner: the three modes, and the comparison. |
| `divergences.json` | Corpus file → why it must diverge. |
| `corpus/` | Hand-written documents. `corpus/divergent/` are the declared ones. |
| `probe/` | The MFBASIC side: `yaml::parseAll` → the same envelope. |

`node_modules/`, `probe/packages/yaml.mfp` and `probe/build/` are git-ignored;
`package-lock.json` is tracked so `npm install` is reproducible.
