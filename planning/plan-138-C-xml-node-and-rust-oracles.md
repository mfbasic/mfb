# plan-138-C: `xml` oracles — Node and Rust, both directions

Last updated: 2026-09-15
Effort: large (3h–1d)
Depends on: plan-138-B (Prerequisites: `planning/plan-138-A-xml-tree-builder-and-reader.md`)

Builds `packages/xml/oracle/`: a Node oracle, a Rust oracle, and an MFB probe that all emit the same
JSON envelope, plus a runner that compares them three-way. The two oracles are **equal peers**: a
case passes only when the package, Node and Rust agree. Where Node and Rust disagree with each
other, the case fails as an *oracle disagreement* until the XML spec or the W3C suite settles it
and the decision is recorded in `divergences.json`.

Outcome: `node packages/xml/oracle/diff.mjs` exits 0 across `corpus`, `xmlconf`, `fuzz-read`,
`fuzz-write`, `roundtrip`, `mutate` and `perf`, and every real defect it found in `packages/xml` is
fixed with a regression test.

References:

- `packages/yaml/oracle/README.md` and `diff.mjs` — modes, envelope, "a refusal is a result",
  `divergences.json`, `corpus/divergent/`.
- `packages/jwt/oracle/probe/src/main.mfb` — the job-file probe protocol (one process answers a
  whole batch).
- `packages/mustache/oracle/fetch-spec.sh` — fetch an upstream suite instead of vendoring it
  (`git ls-files packages/mustache/oracle/spec | wc -l` → 0).
- plan-138-A §4 — the content projection all three sides implement.

## Prerequisites

See plan-138-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-138-B complete | `ls planning/completed/plan-138-B-*` → one file | NOT MET |

## 1. Goal

- `node packages/xml/oracle/diff.mjs` → exit 0, and the README's "What it found" table lists every
  defect it found, each fixed in `packages/xml` with a TESTING case.

### Non-goals (explicit constraints)

- No XPath here (plan-138-E).
- The W3C suite is fetched, not committed.
- No change to the root Cargo workspace (`Cargo.toml` `members = [".", "repository", "wire"]`). The
  Rust oracle is a standalone Cargo project.
- No new `scripts/` entry. Package oracles live beside their package, matching the five existing
  `packages/*/oracle/` directories. AGENTS.md's `tools/<name>/` rule is for repo-level
  probes/oracles, and every package oracle so far lives here.

## 2. Current State

- No oracle exists for `xml`. yaml's is Node plus a PyYAML second opinion
  (`packages/yaml/oracle/pyyaml_diff.py`). No package has a Rust oracle yet
  (`find packages -name Cargo.toml` → none).
- The root `.gitignore` covers `node_modules/` (line 39), `build/` (line 24) and `packages/**/*.mfp`
  (line 34), but not a nested Rust `target/`
  (`git check-ignore -v packages/xml/oracle/rust/target/x` → no match).

### Measured populations

| What | Value | Command |
|---|---|---|
| saxes | 6.0.0 | `npm view saxes version` |
| @xmldom/xmldom | 0.9.12 | `npm view @xmldom/xmldom version` |
| roxmltree | 0.21.1 | `cargo search roxmltree --limit 1` |
| quick-xml | 0.42.0 | `cargo search quick-xml --limit 1` |
| W3C XML Conformance Test Suite `xmlts20130923.tar.gz` | HTTP 200, 641,522 bytes | `curl -sIL https://www.w3.org/XML/Test/xmlts20130923.tar.gz` |
| Node | v24.12.0 | `node --version` |

### Verified properties

- **saxes does not process DTDs.** It reports a `doctype` event and leaves entities to the caller
  (`npm view saxes readme`, section "Regarding `<!DOCTYPE` and `<!ENTITY`"). It has `xmlns: true`
  for namespace checking and `position` tracking (same readme, options list).
- **roxmltree can refuse DTDs.** `ParsingOptions` has `allow_dtd` and `nodes_limit`
  (docs.rs `roxmltree/0.21.1/roxmltree/struct.ParsingOptions.html`).
- **Whether saxes and roxmltree reject a non-UTF-8 `encoding` or `version="1.1"` on their own is
  UNVERIFIED.** Both wrappers check the declaration themselves (Phase 2), so policy agreement does not
  depend on the libraries.
- **@xmldom/xmldom's parser is lenient** (the reason it is not the Node reader). It is used only as
  a DOM and serializer. Leniency is a claim, not measured; it does not matter because the design
  never parses with it.

## 3. Design Overview

```
packages/xml/oracle/
  README.md            modes, envelope, "What it found", layout (mirror yaml's)
  package.json         saxes ^6.0.0, @xmldom/xmldom ^0.9.12 (xpath added in E)
  package-lock.json    tracked
  .gitignore           probe/packages/  probe/build/  rust/target/  xmlconf/
  oracle.mjs           Node: read (saxes → tree → envelope), write (tree → XML via xmldom)
  diff.mjs             runner: modes, three-way comparison, divergences
  divergences.json     case → reason, with a spec section or W3C test id
  fetch-xmlconf.sh     download + unpack the W3C suite into xmlconf/
  corpus/              hand-written documents; corpus/divergent/ for declared ones
  rust/Cargo.toml      standalone: own [workspace] table; roxmltree 0.21.1, quick-xml 0.42.0, serde_json
  rust/Cargo.lock      tracked
  rust/src/main.rs     `xmloracle read <job.json>` / `xmloracle write <job.json>`
  probe/project.json   executable `xmlprobe`, depends on file:packages/xml.mfp
  probe/src/main.mfb   `xmlprobe read|write <job.json>`
```

**Envelope (identical from all three).** For a read:

```
{"ok":true,"content":["doc",[ <node>, ... ]]}
<node> := ["e", name, [[attrName, value], ...sorted by attrName], [<node>, ...]]
        | ["t", text]
{"ok":false,"kind":"parse"|"unsupported"|"limit","reason":"..."}
```

The content is plan-138-A §4's projection: comments/PIs removed, text merged, layout whitespace
removed, attributes sorted. **Two refusals agree without comparing `kind` or `reason`**, as in yaml.
Only accept-vs-refuse and content are compared. `kind` is reported for triage.

**Job files.** Every side takes one JSON job (`{"cases":[{"id","xml"}]}` for read,
`{"cases":[{"id","tree","indent"}]}` for write) and answers all cases in one process, like the jwt
probe. The runner spawns each side once per batch.

**Where the risk is.** Oracle-vs-oracle disagreement is expected on edge rules (namespace
edge cases, attribute normalization, char-ref validity). Equal peers means each disagreement is a
recorded decision, never a silent majority vote. The W3C suite is the tiebreaker.

**Gate class:** new tooling. The gate is the runner's exit status per mode.

Rejected:

- **xmldom as the Node reader** — lenient; would agree with our acceptance bugs.
- **fast-xml-parser** — lenient for the same reason.
- **libxml2 via `libxmljs2`** — native build dependency, and libxml2 is what `todo.md` rejected
  for the package itself.
- **Vendoring xmlconf** — mustache precedent: fetch the upstream suite rather than pin a copy.
- **Majority vote (2 of 3)** — the user chose equal peers; a vote would hide oracle bugs.

## 4. Modes

| Mode | Input | Asserts |
|---|---|---|
| `corpus` | `corpus/*.xml` | three-way agreement, or a declared divergence (and a declared one that stops diverging fails) |
| `xmlconf` | the W3C suite (`fetch-xmlconf.sh`) | per test `TYPE`: `not-wf` → all three refuse; `valid`/`invalid` → if the file has a DOCTYPE, all three refuse (policy), else all three accept with equal content; `error` → reported only. Tests with `RECOMMENDATION` XML 1.1 / NS 1.1 → all refuse (version policy). Tests with `NAMESPACE="no"` are skipped with a printed count, because all three sides run in namespace-checking mode and those tests are defined without it |
| `fuzz-read` | random trees written to XML **by each oracle's own writer** in varied styles: indent none/2/tab, `'` vs `"` quoting, CDATA for text, char refs for non-ASCII, `<a/>` vs `<a></a>`, interleaved comments/PIs, `xmlns:pN` declarations for the random prefixes used | the package (and the other oracle) reads each with content equal to the generating tree; exact |
| `fuzz-write` | random trees → `xmlprobe write` at indent `0`, `2`, `"\t"` | both oracles read the output with content equal to the tree; exact |
| `roundtrip` | corpus + fuzz trees | `content(read(write(read(x)))) = content(read(x))` on the package, compact and pretty |
| `mutate` | corpus with random byte damage (flip, delete, insert `<`/`&`/`]]>`/`\r`/`\0`/invalid UTF-8) | robustness: the probe always answers a well-formed envelope, exit 0, within 30 s; agreement reported as information |
| `perf` | generated 100k-node flat, deep and wide shapes (plan-138-A Phase 1) | `xmlprobe read` of each ≤ 3.00 s |

Random trees use a seeded PRNG. `--seed` and `--count` are printed on failure so a case replays.

## Compatibility / Format Impact

Tooling only. It may produce fixes to `packages/xml`. Each fix is a behavior change pinned by a new
TESTING case and recorded in the oracle README "What it found".

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the work; `- [~]`
> partial; moot tasks struck through with evidence; fill `Commit:`. **Unticked means NOT DONE.**

### Phase 1 — scaffolding and the probe

- [ ] `packages/xml/oracle/package.json`, `package-lock.json` (`npm install --prefix packages/xml/oracle`),
      `.gitignore` as §3.
- [ ] `packages/xml/oracle/rust/Cargo.toml` with an empty `[workspace]` table so Cargo does not
      attach it to the root workspace; `rust/src/main.rs` skeleton that parses a job file and prints
      `{"results":[]}`.
- [ ] `packages/xml/oracle/probe/project.json` (mirror `packages/jwt/oracle/probe/project.json`) and
      `probe/src/main.mfb` implementing `read` and `write` job handling and the §3 envelope
      projection in MFB.
- [ ] Tests: none new in the package; the probe is exercised by Phase 2.

Acceptance: all three sides build and answer an empty job.
  Check: `cargo build --release --manifest-path packages/xml/oracle/rust/Cargo.toml` → `Finished`;
  `target/release/mfb build packages/xml && mkdir -p packages/xml/oracle/probe/packages && cp packages/xml/xml.mfp packages/xml/oracle/probe/packages/ && target/release/mfb build packages/xml/oracle/probe`
  → `Wrote executable`; `git status --short packages/xml/oracle` shows no `target/`, `build/` or
  `node_modules/` (est. 4 min).
Commit: —

### Phase 2 — readers and `corpus`

- [ ] `oracle.mjs` `read`: saxes with `{ xmlns: true, position: true }`; a `doctype` event →
      `unsupported`; declaration `version` ≠ `1.0` or `encoding` not UTF-8 (case-insensitive) →
      `unsupported`; build the tree from events; apply the §3 projection.
- [ ] `rust/src/main.rs` `read`: roxmltree with `allow_dtd: false`, the same declaration checks by
      reading the first bytes, the same projection, `serde_json` output.
- [ ] `diff.mjs` with the three-way comparator, `divergences.json` handling, and `corpus` mode.
- [ ] `corpus/` — at least one realistic document each for: elements and attributes, text escapes,
      char refs, CDATA, comments and PIs, layout whitespace, data whitespace (`<a>  </a>`),
      namespaces and prefixes, CRLF and lone CR, attribute-value normalization, non-ASCII names, BOM,
      XML declaration variants, an SVG, an Atom/RSS feed, a Maven `pom.xml`-shaped file. Refusals:
      internal-subset DOCTYPE, external DOCTYPE, undeclared prefix, duplicate attribute,
      `encoding="ISO-8859-1"`, `version="1.1"`, undeclared entity, mismatched tag, two roots, `]]>`
      in text, `&#0;`.
- [ ] Fix every package defect found: a failing TESTING case first in the matching
      `packages/xml/src/test_*.mfb`, then the fix; add a row to the README "What it found" table.

Acceptance: the corpus agrees three-way.
  Check: `node packages/xml/oracle/diff.mjs corpus` → exit 0 (est. 1 min).
Commit: —

### Phase 3 — `xmlconf`

- [ ] `fetch-xmlconf.sh` — download `https://www.w3.org/XML/Test/xmlts20130923.tar.gz` into
      `xmlconf/` (git-ignored) and unpack; `XMLCONF_URL` override (mustache's `MUSTACHE_SPEC_REF`
      pattern).
- [ ] `diff.mjs xmlconf` — walk the suite's test catalogs and apply the §4 rules; print counts per
      outcome, including the `NAMESPACE="no"` skip count.
- [ ] Record each oracle disagreement in `divergences.json` with the spec section or test id that
      decides it; fix each package defect as in Phase 2.

Acceptance: the suite agrees under the §4 rules.
  Check: `packages/xml/oracle/fetch-xmlconf.sh && node packages/xml/oracle/diff.mjs xmlconf` → exit 0,
  with the printed skip count equal to the number of `NAMESPACE="no"` tests
  (`grep -rho 'NAMESPACE="no"' packages/xml/oracle/xmlconf --include='*.xml' | wc -l`) (est. 5 min).
Commit: —

### Phase 4 — writers, `fuzz-read`, `fuzz-write`, `roundtrip`

- [ ] `oracle.mjs write`: tree → xmldom `Document` → `XMLSerializer`, with the §4 style variations
      applied (indent by inserting layout text nodes; CDATA sections; char refs by post-processing
      text nodes before serialization).
- [ ] `rust/src/main.rs write`: tree → quick-xml `Writer` with the same style variations.
- [ ] `diff.mjs` `fuzz-read`, `fuzz-write`, `roundtrip`, seeded generator with names drawn from
      ASCII, non-ASCII `NameStartChar` ranges and declared prefixes; text drawn from all `Char`
      ranges including `]]>`, CR, tab, and whitespace-only runs as sole children.
- [ ] Fix every package defect as in Phase 2.

Acceptance: both directions agree exactly.
  Check: `node packages/xml/oracle/diff.mjs fuzz-read --count 2000 && node packages/xml/oracle/diff.mjs fuzz-write --count 2000 && node packages/xml/oracle/diff.mjs roundtrip`
  → exit 0 (est. 5 min).
Commit: —

### Phase 5 — `mutate`, `perf`, README

- [ ] `diff.mjs mutate` and `perf` per §4.
- [ ] `packages/xml/oracle/README.md` — setup commands, why each library, envelope, modes, "What it
      found", layout (mirror `packages/yaml/oracle/README.md`).

Acceptance: robustness and performance hold, and the README documents every mode.
  Check: `node packages/xml/oracle/diff.mjs mutate --count 2000 && node packages/xml/oracle/diff.mjs perf`
  → exit 0 (est. 5 min).
Commit: —

## Validation Plan

- Tests: every defect found adds a TESTING case in `packages/xml/src/test_*.mfb` before its fix.
- Coverage check: `target/release/mfb test --coverage packages/xml` after Phase 5; any reader branch
  the oracle never reaches gets a corpus document.
- Runtime proof: `node packages/xml/oracle/diff.mjs` (all modes) → exit 0.
- Doc sync: `packages/xml/oracle/README.md`; `packages/xml/README.md` links it.
- Final gate: runs once at the end of plan-138-E.

## Open Decisions

- `NAMESPACE="no"` W3C tests — **skip with a printed count** vs. run the three sides without
  namespace checks for those files. Recommend skip: the package has no non-namespace mode, and
  adding one only for the suite adds a user-invisible code path. (§4)

## Corrections

<Filled in during execution.>

## Summary

The risk is in the three-way comparator's projection: if the projection hides a difference, every
mode passes vacuously. Phase 2's refusal corpus and exact expected trees guard it. Package fixes
land only behind new TESTING cases.
