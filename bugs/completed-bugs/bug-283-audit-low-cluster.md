# bug-283: `mfb audit` LOW cluster (JSON C1/bidi passthrough, spec drift, libraries/resources ignored, inline-trap resource recursion)

Last updated: 2026-07-18
Effort: medium (1h–2h across items)
Severity: LOW
Class: Security / Correctness / Docs

Status: Fixed 2026-07-18
Regression Test: per-item (tests/ audit fixtures + spec)

LOW-severity residuals in `mfb audit`, found during goal-06. Distinct root causes,
one document per the repo's low-cluster convention. The MEDIUM audit findings are
filed separately (bug-278..281).

References:

- `bugs/completed-bugs/bug-210-*` (terminal-safe escaping for text renderer),
  `bug-211-*` (inline-TRAP resource/LINK reporting).
- Found during goal-06 review of `src/audit/**`.

## Items

### A1 — JSON renderer passes C1 controls and bidi overrides raw (spec falsely claims parity)
- `src/audit/json.rs:72-86` (`write_string`); false claim at
  `src/docs/spec/tooling/04_audit-format.md:31-35`.
- `write_string` escapes only `"`, `\`, and U+0000–U+001F. The text renderer's
  `terminal_safe::safe` (bug-210) additionally escapes DEL, C1 controls
  (U+0080–U+009F — U+009B is a one-byte CSI on some terminals), and bidi/format
  overrides (U+202E etc.). So `mfb audit --format json` on a terminal can still be
  visually spoofed by a crafted package name — the exact attack bug-210 closed for
  text — and the spec's "escapes the same characters" is false. Output stays valid
  JSON.
- Fix: in `write_string`, also `\uXXXX`-escape the C1 and `terminal_safe`
  bidi/format set (lossless), or correct the spec if raw passthrough is intended.
- Prior-work: new (bug-210 fixed text.rs only).

### A2 — audit-format spec analysis-model section has drifted from behavior
- `src/docs/spec/tooling/04_audit-format.md:205-209, 252-260`.
- The "Fallible-call table" omits the per-builtin fallible sets the code has
  (`is_fallible_builtin`: 25 crypto + 6 datetime entries, bug-96) and the LINK
  `SUCCESS_ON` seeding; the fixpoint section claims it starts "from an empty set"
  though `fallible_functions` seeds with `link_fallible_calls`; the
  resource-producer section describes only `LET` bindings though `Statement::Assign`
  reassignments are detected (bug-211). A reader reimplementing from the spec
  produces lesser output than `mfb audit`.
- Fix: add the crypto/datetime builtin lists, the LINK-gate seeding, and the
  reassignment acquisition rule to the spec. (Coordinate with bug-278's table
  extension.)
- Prior-work: new (drift from bug-96/211 fixes).

### A3 — audit ignores the `libraries` and `resources` manifest sections
- `src/audit/collect/mod.rs:39-46` (`collect` reads only `packages`).
- plan-46's `libraries` section decides which native library file each `LINK` binds
  to at build time, and plan-55's `resources` section copies arbitrary files into
  the build output. The Native-links section reports `LINK` symbols but never the
  manifest-declared locator/paths, though `src/docs/spec/tooling/08_auditability.md`
  requires surfacing linked native libraries. A project pointing a benign-looking
  `LINK "sqlite3"` at a vendored `./vendor/evil.dylib` audits identically to one
  using the system library.
- Fix: emit the resolved library locator per `NativeLinkEntry` (or a Libraries
  section) and optionally list `resources` entries.
- Prior-work: new (both sections postdate the last audit-module review).

### A4 — resource acquisitions inside inline-`TRAP` handler bodies are not scanned
- `src/audit/collect/source.rs:121-185` (`collect_resources`).
- `collect_resources` recurses through IF/MATCH/loop bodies and unwraps a `Trapped`
  *value* to its inner call (bug-211), but never descends into a `Trapped`
  expression's `handler` statement list. A fallback acquisition such as
  `LET h = primary() TRAP(e) … LET h2 = fs::open(alt) … END TRAP` omits `h2` from
  Resources and its close-may-fail finding. Sibling of bug-280 (inline-trap
  fallibility labeling).
- Fix: also recurse into `Expression::Trapped::handler` for `Let`/`Assign` values
  (and mirror in the walk).
- Prior-work: new (bug-211 fixed Assign + Trapped-value, not handler bodies).

## Goal

- A1 closes the JSON terminal-spoofing corner; A2 resyncs the spec; A3 surfaces the
  manifest-declared native surface; A4 scans handler-body acquisitions.

### Non-goals (must NOT change)

- The audit finding codes/format beyond adding escapes (A1) and the libraries rows
  (A3).

## Blast Radius

Each item is a single site (cited). A2 pairs with bug-278; A4 pairs with bug-280.

## Fix Design / Phases

- [ ] Phase 1: fixtures for A1/A3/A4 (A2 is a doc edit).
- [ ] Phase 2: apply per-item fixes.
- [ ] Phase 3: regenerate audit goldens; full suite green.

## Validation Plan

- Regression: crafted-name JSON escaping; vendored-library audit surfacing;
  handler-body resource fixture.
- Doc sync: 04_audit-format.md (A1/A2), 08_auditability.md (A3).

## Summary

Four localized audit residuals; each is a small collector/renderer/spec change.
Value is completing the supply-chain-audit story before MVP.

## Resolution

All four items landed.

- **A1** — `write_string` now escapes the full `terminal_safe` set (DEL, the C1
  controls, the bidi/format overrides) in addition to C0, as `\uXXXX` so the
  output stays valid JSON. `is_terminal_unsafe` became `pub(crate)` so the two
  renderers share one definition of the set rather than drifting again. The
  spec's parity claim is now true and says which set it means.
  Test: `json::tests::write_string_escapes_the_terminal_unsafe_set`.
- **A2** — spec corrected: the fixpoint is documented as seeded from the `LINK`
  `SUCCESS_ON` set rather than "an empty set", the fallible-call table names the
  per-builtin sets and why they exist, the inline-`TRAP` containment rule from
  bug-280 is written down, and the resource-producer section records that
  reassignment and inline-`TRAP` handler bodies are recognized too.
- **A3** — new `Libraries` and `Resource files` sections (text) /
  `libraries` and `resourceFiles` arrays (JSON), from `collect_libraries`. A
  project pointing `LINK "sqlite3"` at a vendored file now shows
  `sqlite3 macos vendor evil.dylib` instead of auditing identically to one using
  the system library.
- **A4** — `collect_resources` descends into inline-`TRAP` handler bodies via
  `trapped_handlers`, so a fallback acquisition inside a handler gets its
  Resources row and close-may-fail finding. Handled after the statement match so
  every statement shape is covered, not only `Let`/`Assign`.

Verified end-to-end for A3 (a manifest with a vendored macOS locator and a
`resources` entry renders both new sections) and A4 (`LET h2 = fs::openFile(...)`
inside a handler now appears as a `File` resource). Golden churn was purely
additive: eight fixtures gained two empty JSON keys, and the two projects that
declare `libraries` gained their real section. 994 acceptance tests green.

### Unrelated flake observed

`rt-behavior/resources/closed-default-tls-drop-rt` segfaulted (exit 139) once
during this work and passed 3/3 on re-run. These changes are audit-only and
cannot reach codegen; this matches the known pre-existing `variants_for_union`
HashMap-iteration nondeterminism in resource-union drop. Not caused here, not
fixed here, and worth its own bug.
