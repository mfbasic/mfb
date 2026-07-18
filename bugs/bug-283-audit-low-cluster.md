# bug-283: `mfb audit` LOW cluster (JSON C1/bidi passthrough, spec drift, libraries/resources ignored, inline-trap resource recursion)

Last updated: 2026-07-17
Effort: medium (1h–2h across items)
Severity: LOW
Class: Security / Correctness / Docs

Status: Open
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
