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
| plan-138-B complete | `ls planning/completed/plan-138-B-*` → one file | MET (re-measured 2026-09-15 → `planning/completed/plan-138-B-xml-serializer-and-docs.md`; archived in `47178e067`) |

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
| saxes | 6.0.0 | `npm view saxes version` (re-measured 2026-09-15) |
| @xmldom/xmldom | 0.9.12 | `npm view @xmldom/xmldom version` (re-measured 2026-09-15) |
| roxmltree | 0.21.1 | `cargo search roxmltree --limit 1` (re-measured 2026-09-15) |
| quick-xml | 0.42.0 | `cargo search quick-xml --limit 1` (re-measured 2026-09-15) |
| W3C XML Conformance Test Suite `xmlts20130923.tar.gz` | HTTP 200, 641,522 bytes | `curl -sIL https://www.w3.org/XML/Test/xmlts20130923.tar.gz` (re-measured 2026-09-15: `HTTP/2 200`, `content-length: 641522`) |
| Node | v24.12.0 | `node --version` (re-measured 2026-09-15) |
| a nested Rust `target/` is still NOT ignored | `git check-ignore -v packages/xml/oracle/rust/target/x` → no match (exit 1); `node_modules/` IS ignored (`.gitignore:39`) | re-measured 2026-09-15 — the Phase 1 `.gitignore` is what covers it |

### Verified properties

- **saxes does not process DTDs.** It reports a `doctype` event and leaves entities to the caller
  (`npm view saxes readme`, section "Regarding `<!DOCTYPE` and `<!ENTITY`"). It has `xmlns: true`
  for namespace checking and `position` tracking (same readme, options list).
- **roxmltree can refuse DTDs.** `ParsingOptions` has `allow_dtd` and `nodes_limit`
  (docs.rs `roxmltree/0.21.1/roxmltree/struct.ParsingOptions.html`). Confirmed in the vendored
  source: `allow_dtd` "is set to `false` by default for security reasons", and the refusal arrives as
  the distinct variant `Error::DtdDetected` (`roxmltree-0.21.1/src/parse.rs:97`), so a DOCTYPE is
  told apart from a syntax error rather than guessed at from a message.

- **`Node::namespaces()` yields every namespace IN SCOPE, not the ones declared on that element**
  (measured, `/tmp/nsprobe`, a throwaway probe: for `<a xmlns:p="urn:p" xmlns="urn:d"><b><c xmlns:q="urn:q"/></b></a>`,
  element `b` — which declares nothing — reports `[p=urn:p, (default)=urn:d]`, and `c` reports
  `[q=urn:q, p=urn:p, (default)=urn:d]`). The doc comment does not say so, and the naive reading is
  the opposite. It matters because the package keeps `xmlns`/`xmlns:p` as ORDINARY ATTRIBUTES while
  roxmltree removes them from `attributes()` entirely (same probe: `<a xmlns:p="urn:p" p:id="1" plain="2"/>`
  reports only `id` and `plain`). So the Rust oracle rebuilds each element's declarations by diffing
  its in-scope set against its parent's — emitting them once, where they were written, instead of
  repeating every ancestor's declaration on every descendant, which would have disagreed with the
  package on every namespaced document in the corpus.

- **Names arrive resolved, so the raw `prefix:local` text must be rebuilt.** `tag_name().name()` and
  `attribute.name()` are local parts with the URI held separately; `lookup_prefix(uri)` returns the
  prefix (`Some("p")` in the probe). The package never resolves a prefix, so the oracle puts it back
  to compare like with like.
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

- [x] `packages/xml/oracle/package.json`, `package-lock.json` (`npm install --prefix packages/xml/oracle`),
      `.gitignore` as §3.
- [x] `packages/xml/oracle/rust/Cargo.toml` with an empty `[workspace]` table so Cargo does not
      attach it to the root workspace; `rust/src/main.rs` skeleton that parses a job file and prints
      `{"results":[]}`.
- [x] `packages/xml/oracle/probe/project.json` (mirror `packages/jwt/oracle/probe/project.json`) and
      `probe/src/main.mfb` implementing `read` and `write` job handling and the §3 envelope
      projection in MFB.
- [x] Tests: none new in the package; the probe is exercised by Phase 2. (It is already exercised
      here on real jobs rather than only on the empty one — see the acceptance evidence. An empty
      job would pass even if every projection function were broken.)

Acceptance: all three sides build and answer an empty job.
  Check: `cargo build --release --manifest-path packages/xml/oracle/rust/Cargo.toml` → `Finished`;
  `target/release/mfb build packages/xml && mkdir -p packages/xml/oracle/probe/packages && cp packages/xml/xml.mfp packages/xml/oracle/probe/packages/ && target/release/mfb build packages/xml/oracle/probe`
  → `Wrote executable`; `git status --short packages/xml/oracle` shows no `target/`, `build/` or
  `node_modules/` (est. 4 min).
  MET: `cargo build --release --manifest-path packages/xml/oracle/rust/Cargo.toml` →
  `Finished \`release\` profile [optimized] target(s) in 4.76s`, exit 0; the probe →
  `Wrote executable to packages/xml/oracle/probe/build/xmlprobe.out`. Both sides answer the empty job
  `{"cases":[]}` with `{"results":[]}` and exit 0 (`xmloracle read`, `xmloracle write`, and the probe).

  Cleanliness: `git status --short packages/xml/oracle` collapses to `?? packages/xml/oracle/` because
  the whole directory is new, which hides what is inside it — so measured with
  `git status --short --untracked-files=all packages/xml/oracle`, which lists exactly eight files, all
  of them source: `.gitignore`, `package.json`, `package-lock.json`, `probe/project.json`,
  `probe/src/main.mfb`, `rust/Cargo.toml`, `rust/Cargo.lock`, `rust/src/main.rs`. Each artifact is
  matched by a rule (`git check-ignore -v`): `rust/target/release/xmloracle` → `.gitignore:10`,
  `probe/build/xmlprobe.out` → `:9`, `probe/packages/xml.mfp` → `:8`, and `node_modules/…` → the root
  `.gitignore:39`.

  The probe answers real jobs too, in both directions and on both outcomes. `read`: `<a id="1" b="2"> <b/> </a>`
  → `["e","a",[["b","2"],["id","1"]],[["e","b",[],[]]]]` (attributes sorted, layout whitespace gone);
  `<a>x<!--c-->y</a>` → one merged text node `"xy"`; `<!DOCTYPE a><a/>` → `unsupported`; `<a></b>` →
  `parse`. `write`: the tree `["doc",[["e","a",[["id","1"]],[["t","x"],["e","b",[],[]]]]]]` at indent
  `""` → `<?xml version="1.0" encoding="UTF-8"?><a id="1">x<b/></a>`; the same shape at indent `"  "`
  → the declaration and `<a>`/`<b/>`/`</a>` on their own lines; `["doc",[]]` → refused, "a document
  must hold exactly one root element, not 0"; and `["c",…]` / `["p",…]` nodes round-trip as a prolog
  comment and a processing instruction. So `documentOfJson`/`nodeOfJson` are exercised, not just
  `contentOfDocument`.
Commit: `1c8a2d36c`

### Phase 2 — readers and `corpus`

- [x] `oracle.mjs` `read`: saxes with `{ xmlns: true, position: true }`; a `doctype` event →
      `unsupported`; declaration `version` ≠ `1.0` or `encoding` not UTF-8 (case-insensitive) →
      `unsupported`; build the tree from events; apply the §3 projection.
- [x] `rust/src/main.rs` `read`: roxmltree with `allow_dtd: false`, the same declaration checks by
      reading the first bytes, the same projection, `serde_json` output.
- [x] `diff.mjs` with the three-way comparator, `divergences.json` handling, and `corpus` mode.
- [x] `corpus/` — at least one realistic document each for: elements and attributes, text escapes,
      char refs, CDATA, comments and PIs, layout whitespace, data whitespace (`<a>  </a>`),
      namespaces and prefixes, CRLF and lone CR, attribute-value normalization, non-ASCII names, BOM,
      XML declaration variants, an SVG, an Atom/RSS feed, a Maven `pom.xml`-shaped file. Refusals:
      internal-subset DOCTYPE, external DOCTYPE, undeclared prefix, duplicate attribute,
      `encoding="ISO-8859-1"`, `version="1.1"`, undeclared entity, mismatched tag, two roots, `]]>`
      in text, `&#0;`.
      All 25 written: 14 accepting (`elements-and-attributes`, `escapes-and-refs`, `cdata`,
      `comments-and-pis`, `whitespace` — layout and `<a>   </a>` data both, `namespaces` — including a
      rebound prefix and an `xmlns=""` undeclaration, `line-ends`, `attribute-normalization`,
      `non-ascii-names`, `bom`, `declaration-variants`, `svg`, `atom-feed`, `pom`) and 11 `refuse-*`,
      one per refusal the phase lists. They live in `corpus/` beside the accepting ones, not in a
      separate directory, because §4 defines the mode over `corpus/*.xml` — a sibling directory would
      never be read.
- [x] Fix every package defect found: a failing TESTING case first in the matching
      `packages/xml/src/test_*.mfb`, then the fix; add a row to the README "What it found" table.
      **No package defect surfaced in this phase** — all 25 cases agreed on the first three-way run,
      so there is nothing to fix and no row to add. Recorded rather than left silent: an empty "What
      it found" table should mean "it found nothing here", not "nobody looked".

Acceptance: the corpus agrees three-way.
  Check: `node packages/xml/oracle/diff.mjs corpus` → exit 0 (est. 1 min).
  MET: `node packages/xml/oracle/diff.mjs corpus` → `ok   corpus: 25 case(s) agreed three ways`,
  exit 0.

  **Checked against a vacuous pass**, which this plan's Summary names as the phase's real risk ("if
  the projection hides a difference, every mode passes vacuously"). Two things were measured rather
  than assumed:

  1. *The comparator can fail.* `compare` was fed fabricated results — 9 probes, run by importing it
     (which is why `diff.mjs` now guards `main()` behind a "run directly" check; a module that
     executes on import cannot be tested). It accepts the two genuine-agreement shapes (all three
     agree; all three refuse with different kinds and reasons) and DETECTS all seven planted
     disagreements: the package accepting while both oracles refuse, the package refusing while both
     accept, the Rust oracle alone disagreeing, the Node oracle alone disagreeing, the package
     disagreeing with both, a differing attribute value, and differing text. Result:
     `comparator falsification: all 9 probes behaved`.
  2. *The projection is not empty.* On `namespaces.xml` — the hardest document, with a rebound
     prefix, an `xmlns=""` undeclaration and `xml:`-prefixed attributes — the package and the Rust
     oracle emit byte-identical content:
     `["e","root",[["xmlns","urn:default"],["xmlns:p","urn:p"],["xmlns:q","urn:q"]],[["e","child",[["p:id","1"],["plain","3"],["q:id","2"]],[]],["e","p:elem",[["xml:lang","en"],["xml:space","preserve"]],[["t","text"]]],["e","rebound",[["xmlns:p","urn:other"]],[["e","p:inner",[],[]]]],["e","undefault",[["xmlns",""]],[["e","bare",[],[]]]]]]`.
     Two independent implementations reached the same non-trivial structure, with declarations
     reconstructed where they were written, attributes sorted, and layout whitespace gone.
Commit: —

### Phase 3 — `xmlconf`

- [x] `fetch-xmlconf.sh` — download `https://www.w3.org/XML/Test/xmlts20130923.tar.gz` into
      `xmlconf/` (git-ignored) and unpack; `XMLCONF_URL` override (mustache's `MUSTACHE_SPEC_REF`
      pattern). → `unpacked 3078 xml files`.
- [x] `diff.mjs xmlconf` — walk the suite's test catalogs and apply the §4 rules; print counts per
      outcome, including the `NAMESPACE="no"` skip count.
- [x] Record each oracle disagreement in `divergences.json` with the spec section or test id that
      decides it; fix each package defect as in Phase 2.
      **`divergences.json` is still empty, and that is the stronger outcome.** The first run
      disagreed on 22 of 2,458 cases. Every one was resolved by fixing the side that was wrong —
      one package defect, three defects in this plan's own policy rules, and nine gaps in the Rust
      wrapper — rather than by declaring a divergence to make the run green. A divergence entry
      records a decision the specification settles; none of these needed one, because the W3C suite
      and the Node oracle together settled each case outright.

Acceptance: the suite agrees under the §4 rules.
  Check: `packages/xml/oracle/fetch-xmlconf.sh && node packages/xml/oracle/diff.mjs xmlconf` → exit 0,
  with the printed skip count equal to the number of `NAMESPACE="no"` tests.
  **Corrected:** the count is **14**, and the plan's original command
  (`grep -rho 'NAMESPACE="no"' … | wc -l` → 11) undercounts it, because `oasis/oasis.xml` writes its
  attributes with SINGLE quotes (`NAMESPACE='no'`, `URI='p01pass2.xml'`). The command that measures
  it is the census in Corrections, which accepts both quote styles. A skip count of 11 would now be
  the failing answer, not the passing one.
  MET: `node packages/xml/oracle/diff.mjs xmlconf` → `ok   xmlconf: 2140 case(s) agreed three ways`,
  exit 0, printing `71 accepted, 2069 refused by policy; skipped 14 NAMESPACE=no, 27 TYPE=error,
  87 not UTF-8`. The skip count **is** the corrected 14. `TYPE=error` reports 27 rather than the
  census's 33 because the encoding skip is applied first and six of those files are not UTF-8.
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

**Phase 3 — the suite found a real package defect: a colon in a processing-instruction target.**
Namespaces in XML 1.0, erratum NE08, makes a PI target an NCName, so `<?a:b bogus?>` is not
namespace-well-formed — the suite's `rmt-ns10-042`, `TYPE="not-wf"`. The reader checked QNames for
element and attribute names but not for PI targets, and accepted it; the Node oracle refused it. Two
independent authorities against the package, so the package was wrong. Fixed in
`packages/xml/src/read.mfb` behind a failing case first (`test_refuse.mfb`, "a colon in a
processing-instruction target is refused", which also pins that a colon-free target still parses);
`mfb test packages/xml` went 157 → 158. **This is the oracle's first real find, and it owes a row in
the README's "What it found" table in Phase 5.**

**Phase 3 — three of this plan's own §4 policy rules were wrong, and the suite proved it.**

1. *`<!DOCTYPE` cannot be detected by a substring scan.* `o-p15pass1`, `o-p16pass1` and `o-p18pass1`
   carry the literal text `<!DOCTYPE` inside a comment, a processing instruction and a CDATA section
   respectively, and are well-formed DTD-less documents. The rule now strips comments, PIs and CDATA
   before looking. The same mistake bit the Rust wrapper's character-reference check, where `&#c`
   inside a PI and inside CDATA (`o-p16pass1`, `o-p18pass1`) read as a malformed reference.
2. *`RECOMMENDATION="XML1.1"` does not mean "refuse".* `rmt-016` and `rmt-019` assert name characters
   "illegal in XML 1.0" — but that is the FOURTH edition. XML 1.0 **Fifth Edition**, which this
   package implements, adopted XML 1.1's name rules, so `U+1D032` and `U+EFFFF` are legal here and
   accepting them is correct. The suite says so itself through `EDITION="1 2 3 4"`: a test that names
   the editions it applies to, and omits the fifth, does not apply. The rule now skips those and keys
   the version policy off what the DOCUMENT declares.
3. *`NS1.1` tests are skipped, not refused* — this reader implements Namespaces 1.0.

**Phase 3 — roxmltree is more permissive than the policy, so the wrapper enforces it.** The Rust side
accepted nine things the package and Node both refused: reserved `xml`/`XML`/`xmL` PI targets, a PI
target not followed by whitespace (`<?pitarget+++?>` — the target is a *Name*, so taking "everything
up to whitespace" hid it), a colon in a PI target, `<:foo/>` (roxmltree splits it into an empty
prefix and local `foo`, so no colon survives in the parsed tree — only the source shows it),
`xmlns:a=""` undeclaration (Namespaces 1.1 only), and `xmlns:xmlns`. The wrapper now checks each
itself, exactly as it already checked version and encoding, so agreement never rests on how
permissive a library happens to be. One more needed `Node::range()`: an explicitly declared
`xmlns:xml` never reaches `namespaces()` because roxmltree treats the prefix as pre-bound, so it is
recovered from the element's own start tag (`rmt-ns10-028`).

**Phase 3 — the suite uses both quote styles, so the acceptance grep undercounts `NAMESPACE="no"`.**
`oasis/oasis.xml` writes every attribute single-quoted (`<TEST TYPE='valid' … URI='p01pass2.xml'>`),
and the plan's acceptance command matches only double quotes. Measured with a census that accepts
both (run over `packages/xml/oracle/xmlconf/xmlconf`):

| What | Value |
|---|---|
| `TEST` entries across 21 catalogs | 2,586 |
| entries with no `URI` attribute | 0 |
| entries whose file is missing on disk | 0 |
| `TYPE` | `not-wf` 1,499 · `valid` 812 · `invalid` 242 · `error` 33 |
| `NAMESPACE` | absent 2,493 · `yes` 79 · **`no` 14** |
| `RECOMMENDATION` | absent 1,821 · `XML1.0-errata4e` 393 · `XML1.1` 266 · `NS1.0` 48 · `XML1.0-errata2e` 34 · `XML1.0-errata3e` 13 · `NS1.1` 8 · `NS1.0-errata1e` 3 |

A double-quote-only grep reports 11 of those 14, and reports 348 of the oasis entries as having no
`URI` at all. Every `URI` resolves against its own catalog's directory, so `xml:base` — which appears
only in `eduni/xmlconf.xml`, a wrapper that includes sub-catalogs — never needs handling: the mode
walks the 21 catalogs that actually hold `TEST` entries.

**Phase 3 — 87 test files cannot be sent to the three sides at all, and need their own skip count.**
§4 provides for skipping `NAMESPACE="no"` but says nothing about encoding. Of the 2,586 test files,
42 begin with a UTF-16 BOM and 45 are not valid UTF-8 (census above). They cannot cross the JSON
string boundary the job protocol uses, and MFBASIC's `String` cannot hold them in the first place —
letter A's Corrections already records that invalid UTF-8 is unreachable through `parse(text)`. They
are skipped with a printed count, exactly as the namespace tests are, rather than being silently
counted as agreement. The package's UTF-8-only policy is pinned where it is reachable, in
`test_chars.mfb`.

**Phase 3 — nearly every `valid`/`invalid` test is a DOCTYPE policy refusal.** 2,144 of the 2,499
UTF-8 test files hold a `<!DOCTYPE`: 794 of 812 `valid`, 165 of 242 `invalid`, 1,161 of 1,499
`not-wf`, 24 of 33 `error`. Under §4's rule that any file with a DOCTYPE must be refused by all three
sides, the suite therefore exercises the DOCTYPE policy far more than it exercises the grammar. That
is worth stating rather than discovering later from a suspiciously green run: the mode prints the
breakdown so the number of tests that reached the grammar at all is visible.

## Summary

The risk is in the three-way comparator's projection: if the projection hides a difference, every
mode passes vacuously. Phase 2's refusal corpus and exact expected trees guard it. Package fixes
land only behind new TESTING cases.
